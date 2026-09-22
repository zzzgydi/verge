use super::*;
use crate::script::{ProfileScript, ScriptPreview};
use sha2::{Digest, Sha256};

pub(super) enum CompileMode<'a> {
    Runtime,
    MergePreview,
    ScriptPreview(Option<&'a ProfileId>, &'a str),
}

impl FileProfileStore {
    fn script_path(&self, id: Option<&ProfileId>) -> Result<PathBuf, AppError> {
        if let Some(id) = id {
            self.require_profile(id)?;
        }
        Ok(self.root.join("scripts").join(match id {
            Some(id) => format!("profile-{}.json", id.as_str()),
            None => "global.json".into(),
        }))
    }

    pub fn script(&self, id: Option<&ProfileId>) -> Result<ProfileScript, AppError> {
        match fs::read(self.script_path(id)?) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(storage_error),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(ProfileScript::default()),
            Err(e) => Err(storage_error(e)),
        }
    }

    pub fn set_script(
        &mut self,
        id: Option<&ProfileId>,
        script: &ProfileScript,
    ) -> Result<(), AppError> {
        crate::script::validate_source(&script.draft)?;
        if let Some(active) = &script.active {
            crate::script::validate_source(active)?;
        }
        if self.script(id)?.active != script.active {
            for profile in self.list() {
                if id.is_none_or(|id| id == &profile.id) {
                    self.protect_compilation(&profile.id)?;
                }
            }
        }
        let path = self.script_path(id)?;
        fs::create_dir_all(path.parent().expect("script directory")).map_err(storage_error)?;
        atomic_write_private(&path, &serde_json::to_vec(script).map_err(storage_error)?)
            .map_err(storage_error)
    }

    pub fn save_script_draft(
        &mut self,
        id: Option<&ProfileId>,
        draft: String,
    ) -> Result<ProfileScript, AppError> {
        let mut script = self.script(id)?;
        script.draft = draft;
        self.set_script(id, &script)?;
        Ok(script)
    }

    pub fn preview_script(
        &self,
        scope: Option<&ProfileId>,
        id: &ProfileId,
        draft: &str,
    ) -> Result<ScriptPreview, AppError> {
        if let Some(scope) = scope {
            self.require_profile(scope)?;
            if scope != id {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    "Preview must use the script's profile",
                ));
            }
        }
        let (value, logs) = self.compile(
            id,
            &self.yaml(id)?,
            &self.merge,
            CompileMode::ScriptPreview(scope, draft),
        )?;
        let yaml = serde_yaml::to_string(&self.apply_network(value)?).map_err(storage_error)?;
        Ok(ScriptPreview { yaml, logs })
    }

    /// A content-addressed, private artifact makes validation, apply, getters and
    /// rollback reuse the exact same JS result, including across daemon restarts.
    pub(super) fn compile(
        &self,
        id: &ProfileId,
        source: &str,
        merge: &MergeConfig,
        mode: CompileMode<'_>,
    ) -> Result<(serde_yaml::Value, Vec<String>), AppError> {
        let profile = &self.manifest.profiles[self.profile_index(id)?];
        let value = apply_merge(&serde_yaml::from_str(source).map_err(storage_error)?, merge)?;
        let mut scripts = Vec::new();
        for scope in [None, Some(id)] {
            let active = match mode {
                CompileMode::ScriptPreview(target, source) if scope == target => {
                    Some(source.to_owned())
                }
                _ => self.script(scope)?.active,
            };
            if let Some(source) = active {
                scripts.push(source);
            }
        }
        if scripts.is_empty() {
            return Ok((value, Vec::new()));
        }
        let key = serde_json::to_vec(&("boa-0.22-api-1", source, merge, &scripts, &profile.name))
            .map_err(storage_error)?;
        let hash = format!("{:x}", Sha256::digest(key));
        let directory = self.root.join("compiled").join(id.as_str());
        let preview = !matches!(mode, CompileMode::Runtime);
        let extension = if preview { "preview" } else { "json" };
        let path = directory.join(format!("{hash}.{extension}"));
        // An enabled version reuses the preview artifact when inputs still match.
        let cached = fs::read(&path).or_else(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                fs::read(directory.join(format!(
                    "{hash}.{}",
                    if preview { "json" } else { "preview" }
                )))
            } else {
                Err(e)
            }
        });
        let result: ScriptPreview = match cached {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(storage_error)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => crate::script::transform(
                id,
                &profile.name,
                &value,
                scripts,
            )
            .map_err(|error| {
                if matches!(mode, CompileMode::ScriptPreview(..)) {
                    error
                } else {
                    AppError::new(
                        ErrorCode::ValidationFailed,
                        "Profile script failed. Open the script editor and preview it for details.",
                    )
                }
            })?,
            Err(e) => return Err(storage_error(e)),
        };
        if !path.exists() {
            fs::create_dir_all(&directory).map_err(storage_error)?;
            atomic_write_private(&path, &serde_json::to_vec(&result).map_err(storage_error)?)
                .map_err(storage_error)?;
            // Preview churn cannot evict enabled artifacts needed by getters or rollback.
            let protected = fs::read_to_string(directory.join("protected")).unwrap_or_default();
            let mut entries = fs::read_dir(&directory)
                .map_err(storage_error)?
                .filter_map(Result::ok)
                .filter(|e| e.path().extension().is_some_and(|e| e == extension))
                .filter(|e| e.file_name().to_string_lossy() != format!("{protected}.json"))
                .collect::<Vec<_>>();
            entries.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
            let excess = entries.len().saturating_sub(if preview { 2 } else { 4 });
            for entry in entries.into_iter().take(excess) {
                let _ = fs::remove_file(entry.path());
            }
        }
        Ok((
            serde_yaml::from_str(&result.yaml).map_err(storage_error)?,
            result.logs,
        ))
    }

    /// Pin the old input's artifact before a transaction, including validation-only
    /// attempts. Failed candidates must never evict the last working JS result.
    pub(super) fn protect_compilation(&self, id: &ProfileId) -> Result<(), AppError> {
        let profile = &self.manifest.profiles[self.profile_index(id)?];
        let scripts: Vec<_> = [self.script(None)?.active, self.script(Some(id))?.active]
            .into_iter()
            .flatten()
            .collect();
        if scripts.is_empty() {
            return Ok(());
        }
        let source = self.yaml(id)?;
        let key = serde_json::to_vec(&(
            "boa-0.22-api-1",
            &source,
            &self.merge,
            &scripts,
            &profile.name,
        ))
        .map_err(storage_error)?;
        let hash = format!("{:x}", Sha256::digest(key));
        let directory = self.root.join("compiled").join(id.as_str());
        // Do not replace a last-good pin with an uncompiled, failed version.
        if directory.join(format!("{hash}.json")).is_file() {
            atomic_write_private(&directory.join("protected"), hash.as_bytes())
                .map_err(storage_error)?;
        }
        Ok(())
    }

    pub fn update_details(
        &mut self,
        id: &ProfileId,
        name: String,
        source: ProfileSource,
        policy: UpdatePolicy,
        user_agent: Option<String>,
        now: i64,
    ) -> Result<(), AppError> {
        let index = self.profile_index(id)?;
        let previous = self.manifest.profiles[index].clone();
        if std::mem::discriminant(&previous.source) != std::mem::discriminant(&source) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Profile source type cannot be changed",
            ));
        }
        if matches!(source, ProfileSource::Local)
            && (!matches!(policy, UpdatePolicy::Manual) || user_agent.is_some())
        {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Local profiles cannot have subscription settings",
            ));
        }
        if user_agent
            .as_ref()
            .is_some_and(|ua| ua.contains(['\r', '\n']))
        {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "User-Agent must be a single line",
            ));
        }
        let mut profile = Profile::new(id.clone(), name.trim(), source, policy, now, user_agent)?;
        profile.updated_at = previous.updated_at;
        self.protect_compilation(id)?;
        self.manifest.profiles[index] = profile;
        if let Err(e) = self.save_manifest() {
            self.manifest.profiles[index] = previous;
            return Err(e);
        }
        Ok(())
    }

    pub fn restore_profile_details(&mut self, profile: Profile) -> Result<(), AppError> {
        let index = self.profile_index(&profile.id)?;
        let previous = std::mem::replace(&mut self.manifest.profiles[index], profile);
        if let Err(e) = self.save_manifest() {
            self.manifest.profiles[index] = previous;
            return Err(e);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::TestDir;
    use super::*;

    #[test]
    fn preview_cache_survives_drafts_and_reuses_exact_enabled_result() {
        let directory = TestDir::new("script-cache");
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        let id = ProfileId::parse("test").unwrap();
        store
            .import(
                Profile::new(
                    id.clone(),
                    "Test",
                    ProfileSource::Local,
                    UpdatePolicy::Manual,
                    0,
                    None,
                )
                .unwrap(),
                "mode: rule\n",
            )
            .unwrap();
        let source = "function main(c){c.random=Math.random();return c}";
        let preview = store.preview_script(Some(&id), &id, source).unwrap();
        store
            .set_script(
                Some(&id),
                &ProfileScript {
                    draft: source.into(),
                    active: Some(source.into()),
                },
            )
            .unwrap();
        let applied = store.merged_yaml(&id).unwrap();
        assert_eq!(applied, preview.yaml);
        for i in 0..7 {
            store
                .preview_script(
                    Some(&id),
                    &id,
                    &format!("function main(c){{c.test={i};return c}}"),
                )
                .unwrap();
        }
        assert_eq!(store.merged_yaml(&id).unwrap(), applied);
        let mut reopened = FileProfileStore::open(&directory.0).unwrap();
        assert_eq!(reopened.merged_yaml(&id).unwrap(), applied);
        reopened
            .save_script_draft(Some(&id), "broken draft".into())
            .unwrap();
        assert_eq!(reopened.merged_yaml(&id).unwrap(), applied);
        assert_eq!(reopened.yaml(&id).unwrap(), "mode: rule\n");
    }

    #[test]
    fn merge_previews_preserve_runtime_cache_and_reuse_the_accepted_preview() {
        let directory = TestDir::new("merge-script-preview-cache");
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        let id = ProfileId::parse("test").unwrap();
        store
            .import(
                Profile::new(
                    id.clone(),
                    "Test",
                    ProfileSource::Local,
                    UpdatePolicy::Manual,
                    0,
                    None,
                )
                .unwrap(),
                "mode: rule\n",
            )
            .unwrap();
        let source = "function main(c){c.random=Math.random();return c}";
        store
            .set_script(
                Some(&id),
                &ProfileScript {
                    draft: source.into(),
                    active: Some(source.into()),
                },
            )
            .unwrap();
        // Initial activation has no older compilation to pin.
        let applied = store.merged_yaml(&id).unwrap();
        let runtime = store
            .materialize_runtime(&id, "127.0.0.1:9090".parse().unwrap(), "test")
            .unwrap();
        let runtime_bytes = fs::read(&runtime).unwrap();
        let mut last = (String::new(), String::new());
        for i in 0..8 {
            let merge = format!("rules:\n - key: probe\n   op: override\n   value: {i}\n");
            last = (merge.clone(), store.preview_merge(&id, &merge).unwrap());
        }
        assert_eq!(store.merged_yaml(&id).unwrap(), applied);
        assert_eq!(fs::read(&runtime).unwrap(), runtime_bytes);
        let count = |extension| {
            fs::read_dir(directory.0.join("compiled/test"))
                .unwrap()
                .flatten()
                .filter(|entry| {
                    entry
                        .path()
                        .extension()
                        .is_some_and(|value| value == extension)
                })
                .count()
        };
        assert_eq!(count("json"), 1);
        assert_eq!(count("preview"), 2);
        let mut reopened = FileProfileStore::open(&directory.0).unwrap();
        assert_eq!(reopened.merged_yaml(&id).unwrap(), applied);
        reopened.set_merge_yaml(&last.0).unwrap();
        assert_eq!(reopened.merged_yaml(&id).unwrap(), last.1);
    }

    #[test]
    fn script_pipeline_orders_merge_global_profile_and_network() {
        let directory = TestDir::new("script-order");
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        let id = ProfileId::parse("test").unwrap();
        store
            .import(
                Profile::new(
                    id.clone(),
                    "Test",
                    ProfileSource::Local,
                    UpdatePolicy::Manual,
                    0,
                    None,
                )
                .unwrap(),
                "mode: rule\n",
            )
            .unwrap();
        store
            .set_merge_yaml("rules:\n - key: stage\n   op: override\n   value: merge\n")
            .unwrap();
        let global = "function main(c){c.stage+='-global';return c}";
        let profile = "function main(c){c.stage+='-profile';c['mixed-port']=1;return c}";
        for (scope, source) in [(None, global), (Some(&id), profile)] {
            store
                .set_script(
                    scope,
                    &ProfileScript {
                        draft: source.into(),
                        active: Some(source.into()),
                    },
                )
                .unwrap();
        }
        let network = crate::domain::CoreNetworkSettings {
            mixed_port: 7898,
            ..Default::default()
        };
        store.set_network_override(Some(network)).unwrap();
        let result: serde_yaml::Value =
            serde_yaml::from_str(&store.merged_yaml(&id).unwrap()).unwrap();
        assert_eq!(result["stage"].as_str(), Some("merge-global-profile"));
        assert_eq!(result["mixed-port"].as_u64(), Some(7898));
    }
}
