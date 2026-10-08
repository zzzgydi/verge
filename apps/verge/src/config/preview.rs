//! An isolated copy of the normal compiler inputs, used by diagnostic workers.
use super::*;
use sha2::{Digest, Sha256};

pub(crate) fn read_bounded(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    use std::io::Read;
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > limit as u64 {
        return Err(io::Error::other("AI preview file exceeds byte limit"));
    }
    let mut data = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut data)?;
    if data.len() > limit {
        return Err(io::Error::other("AI preview file exceeds byte limit"));
    }
    Ok(data)
}

pub struct ConfigPreview {
    pub store: FileProfileStore,
    pub baseline: String,
}

impl Drop for ConfigPreview {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.store.root);
    }
}

impl FileProfileStore {
    fn preview_files(&self) -> Vec<PathBuf> {
        let mut paths = vec![
            "profiles.json".into(),
            "merge.yaml".into(),
            "network-settings.yaml".into(),
            "scripts/global.json".into(),
        ];
        for profile in self.list() {
            paths.push(PathBuf::from("profiles").join(format!("{}.yaml", profile.id.as_str())));
            paths.push(
                PathBuf::from("scripts").join(format!("profile-{}.json", profile.id.as_str())),
            );
        }
        paths
    }

    /// Includes persisted inputs and in-memory settings. Never exposed to the model.
    pub fn preview_digest(&self) -> Result<String, AppError> {
        let mut hash = Sha256::new();
        hash.update(
            serde_json::to_vec(&(
                &self.manifest,
                &self.merge,
                &self.network,
                &self.internal_socket,
                self.runtime_tun,
                self.dev_mode,
            ))
            .map_err(storage_error)?,
        );
        let mut bytes = 0;
        for path in self.preview_files() {
            hash.update(path.to_string_lossy().as_bytes());
            match read_bounded(&self.root.join(path), 16 * 1024 * 1024 - bytes) {
                Ok(data) => {
                    bytes += data.len();
                    if bytes > 16 * 1024 * 1024 {
                        return Err(storage_error("AI preview inputs exceed 16 MiB"));
                    }
                    hash.update((data.len() as u64).to_le_bytes());
                    hash.update(data);
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => hash.update(b"missing"),
                Err(e) => return Err(storage_error(e)),
            }
        }
        Ok(format!("{:x}", hash.finalize()))
    }

    pub fn isolated_preview(&self, root: PathBuf) -> Result<ConfigPreview, AppError> {
        if self.list().len() > 16 {
            return Err(storage_error("AI preview supports at most 16 profiles"));
        }
        let baseline = self.preview_digest()?;
        let mut store = self.clone();
        store.root = root;
        let mut directory = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            directory.mode(0o700);
        }
        directory.create(&store.root).map_err(storage_error)?;
        let preview = ConfigPreview { store, baseline };
        let mut total = 0;
        for path in self.preview_files() {
            match read_bounded(&self.root.join(&path), 16 * 1024 * 1024 - total) {
                Ok(bytes) => {
                    total += bytes.len();
                    let target = preview.store.root.join(path);
                    fs::create_dir_all(target.parent().unwrap()).map_err(storage_error)?;
                    atomic_write_private(&target, &bytes).map_err(storage_error)?;
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(storage_error(e)),
            }
        }
        for path in ["profiles", "snapshots", "candidates", "runtime"] {
            fs::create_dir_all(preview.store.root.join(path)).map_err(storage_error)?;
        }
        // Reuse prior random/deterministic script output when inputs have not changed.
        preview.store.copy_compilation_cache_from(self)?;
        if preview.store.preview_digest()? != preview.baseline
            || self.preview_digest()? != preview.baseline
        {
            return Err(storage_error("Configuration changed during preview"));
        }
        Ok(preview)
    }

    pub(crate) fn runtime_bytes(&self) -> Result<Vec<u8>, AppError> {
        read_bounded(&self.root.join("runtime-config.yaml"), 16 * 1024 * 1024)
            .map_err(storage_error)
    }

    /// Only compiler artifacts are copied back; source, scripts, settings and runtime stay untouched.
    pub fn copy_compilation_cache_from(&self, source: &Self) -> Result<(), AppError> {
        let mut total = 0;
        for profile in self.list() {
            let relative = PathBuf::from("compiled").join(profile.id.as_str());
            let entries = match fs::read_dir(source.root.join(&relative)) {
                Ok(entries) => entries,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(storage_error(e)),
            };
            for entry in entries {
                let entry = entry.map_err(storage_error)?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                let Some(hash) = name
                    .strip_suffix(".json")
                    .or_else(|| name.strip_suffix(".preview"))
                else {
                    continue;
                };
                if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
                    continue;
                }
                let bytes =
                    read_bounded(&entry.path(), 64 * 1024 * 1024 - total).map_err(storage_error)?;
                total += bytes.len();
                if total > 64 * 1024 * 1024 {
                    return Err(storage_error("AI compilation cache exceeds 64 MiB"));
                }
                let target = self.root.join(&relative).join(name.as_ref());
                fs::create_dir_all(target.parent().unwrap()).map_err(storage_error)?;
                atomic_write_private(&target, &bytes).map_err(storage_error)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests::{TestDir, profile};
    #[test]
    fn preview_preserves_sources_and_compiler_results_and_detects_edits() {
        let dir = TestDir::new("ai-preview");
        let mut store = FileProfileStore::open(&dir.0).unwrap();
        let id = ProfileId::parse("daily").unwrap();
        store
            .import(
                profile("daily", UpdatePolicy::Manual),
                "mixed-port: 7897\nrules: ['MATCH,DIRECT']",
            )
            .unwrap();
        store.select(&id).unwrap();
        store
            .set_script(
                Some(&id),
                &crate::script::ProfileScript {
                    draft: "function main(c){c['test-random']=Math.random();return c}".into(),
                    active: Some(
                        "function main(c){c['test-random']=Math.random();return c}".into(),
                    ),
                },
            )
            .unwrap();
        let baseline = store.merged_yaml(&id).unwrap();
        let runtime = store
            .materialize_runtime(&id, "127.0.0.1:10000".parse().unwrap(), "test")
            .unwrap();
        let bytes = fs::read(&runtime).unwrap();
        let preview_path = dir.0.join("isolated");
        let mut preview = store.isolated_preview(preview_path.clone()).unwrap();
        assert_eq!(preview.store.merged_yaml(&id).unwrap(), baseline);
        preview
            .store
            .set_merge_yaml("rules: [{key: log-level, op: override, value: debug}]")
            .unwrap();
        assert!(preview.store.merged_yaml(&id).unwrap().contains("debug"));
        assert_eq!(store.preview_digest().unwrap(), preview.baseline);
        assert_eq!(fs::read(runtime).unwrap(), bytes);
        assert_eq!(store.merge(), &MergeConfig::default());
        store
            .set_merge_yaml("rules: [{key: log-level, op: override, value: info}]")
            .unwrap();
        assert_ne!(store.preview_digest().unwrap(), preview.baseline);
        drop(preview);
        assert!(!preview_path.exists());
    }
}
