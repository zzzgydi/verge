use std::{
    collections::HashSet,
    fs,
    io::{self, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
};

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use serde::{Deserialize, Serialize};
use verge_domain::{
    AppError, ApplicationSettings, ErrorCode, ProfileId, ProxyEndpoint, SettingsFieldChange,
    SettingsImportPreview,
};
pub use verge_domain::{Profile, ProfileSource, UpdatePolicy};
use zeroize::Zeroize;

const MANIFEST_VERSION: u32 = 1;
const SETTINGS_VERSION: u32 = 1;
const BACKUP_VERSION: u32 = 1;
const BACKUP_AAD: &[u8] = b"verge-encrypted-backup-v1";
const MERGE_RULE_LIMIT: usize = 64;
const MERGE_KEY_LIMIT: usize = 128;
/// 由运行物化注入的私有字段不允许出现在 merge 配置中,避免被误导为可覆盖。
const RESERVED_MERGE_KEYS: [&str; 2] = ["external-controller", "secret"];

/// 应用级 Merge 配置(全局一份):对源配置顶层键做受控的结构化深合并,
/// 在物化私有运行配置之前应用。首版不做任意脚本增强。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MergeConfig {
    pub rules: Vec<MergeRule>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MergeRule {
    pub key: String,
    #[serde(flatten)]
    pub op: MergeOp,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum MergeOp {
    /// 用 value 整体替换顶层键;键不存在时写入。
    Override { value: serde_yaml::Value },
    /// 深合并:mapping 递归合并,序列按 `name` 归并(无 `name` 的项追加),标量等同 override。
    Merge { value: serde_yaml::Value },
    /// 在目标序列前插入;目标键缺失时视为空序列。
    Prepend { items: Vec<serde_yaml::Value> },
    /// 在目标序列后追加;目标键缺失时视为空序列。
    Append { items: Vec<serde_yaml::Value> },
    /// 删除顶层键;键不存在时为空操作。
    Remove,
}

impl MergeConfig {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.rules.len() > MERGE_RULE_LIMIT {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                format!("merge config allows at most {MERGE_RULE_LIMIT} rules"),
            ));
        }
        for rule in &self.rules {
            validate_merge_key(&rule.key)?;
        }
        Ok(())
    }
}

fn validate_merge_key(key: &str) -> Result<(), AppError> {
    let valid = !key.is_empty()
        && key.len() <= MERGE_KEY_LIMIT
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if !valid {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "merge key must contain only ASCII letters, numbers, '-' or '_'",
        ));
    }
    if RESERVED_MERGE_KEYS.contains(&key) {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            format!("merge key '{key}' is reserved for runtime injection"),
        ));
    }
    Ok(())
}

/// 纯函数:源配置 + merge 配置 → 候选配置。类型冲突返回 ValidationFailed。
pub fn apply_merge(
    source: &serde_yaml::Value,
    merge: &MergeConfig,
) -> Result<serde_yaml::Value, AppError> {
    merge.validate()?;
    let mut mapping = source.as_mapping().cloned().ok_or_else(|| {
        AppError::new(
            ErrorCode::ValidationFailed,
            "profile YAML root must be a mapping",
        )
    })?;
    for rule in &merge.rules {
        let key = serde_yaml::Value::String(rule.key.clone());
        match &rule.op {
            MergeOp::Override { value } => {
                mapping.insert(key, value.clone());
            }
            MergeOp::Remove => {
                mapping.remove(&key);
            }
            MergeOp::Prepend { items } => {
                let target = take_sequence(&mut mapping, &key, &rule.key, "prepend")?;
                let mut merged = items.clone();
                merged.extend(target);
                mapping.insert(key, serde_yaml::Value::Sequence(merged));
            }
            MergeOp::Append { items } => {
                let mut target = take_sequence(&mut mapping, &key, &rule.key, "append")?;
                target.extend(items.clone());
                mapping.insert(key, serde_yaml::Value::Sequence(target));
            }
            MergeOp::Merge { value } => match mapping.remove(&key) {
                None => {
                    mapping.insert(key, value.clone());
                }
                Some(existing) => {
                    let merged = merge_values(existing, value, &rule.key)?;
                    mapping.insert(key, merged);
                }
            },
        }
    }
    Ok(serde_yaml::Value::Mapping(mapping))
}

fn take_sequence(
    mapping: &mut serde_yaml::Mapping,
    key: &serde_yaml::Value,
    name: &str,
    op: &str,
) -> Result<Vec<serde_yaml::Value>, AppError> {
    match mapping.remove(key) {
        None => Ok(Vec::new()),
        Some(serde_yaml::Value::Sequence(items)) => Ok(items),
        Some(_) => Err(AppError::new(
            ErrorCode::ValidationFailed,
            format!("merge {op} on '{name}' requires a sequence target"),
        )),
    }
}

fn merge_values(
    existing: serde_yaml::Value,
    incoming: &serde_yaml::Value,
    key: &str,
) -> Result<serde_yaml::Value, AppError> {
    match (existing, incoming) {
        (serde_yaml::Value::Mapping(mut base), serde_yaml::Value::Mapping(patch)) => {
            for (patch_key, patch_value) in patch {
                match base.remove(patch_key) {
                    Some(old) => {
                        let merged = merge_values(old, patch_value, key)?;
                        base.insert(patch_key.clone(), merged);
                    }
                    None => {
                        base.insert(patch_key.clone(), patch_value.clone());
                    }
                }
            }
            Ok(serde_yaml::Value::Mapping(base))
        }
        (serde_yaml::Value::Sequence(mut base), serde_yaml::Value::Sequence(patch)) => {
            for item in patch {
                let name = item.get("name").and_then(serde_yaml::Value::as_str);
                let position = name.and_then(|name| {
                    base.iter().position(|candidate| {
                        candidate.get("name").and_then(serde_yaml::Value::as_str) == Some(name)
                    })
                });
                match position {
                    Some(index) => {
                        base[index] = merge_values(base[index].clone(), item, key)?;
                    }
                    None => base.push(item.clone()),
                }
            }
            Ok(serde_yaml::Value::Sequence(base))
        }
        (existing, incoming)
            if !matches!(
                existing,
                serde_yaml::Value::Mapping(_) | serde_yaml::Value::Sequence(_)
            ) && !matches!(
                incoming,
                serde_yaml::Value::Mapping(_) | serde_yaml::Value::Sequence(_)
            ) =>
        {
            Ok(incoming.clone())
        }
        (existing, incoming) => Err(AppError::new(
            ErrorCode::ValidationFailed,
            format!(
                "merge key '{key}' type conflict: cannot merge {} into {}",
                yaml_kind(incoming),
                yaml_kind(&existing),
            ),
        )),
    }
}

fn yaml_kind(value: &serde_yaml::Value) -> &'static str {
    match value {
        serde_yaml::Value::Null => "null",
        serde_yaml::Value::Bool(_) => "boolean",
        serde_yaml::Value::Number(_) => "number",
        serde_yaml::Value::String(_) => "string",
        serde_yaml::Value::Sequence(_) => "sequence",
        serde_yaml::Value::Mapping(_) => "mapping",
        serde_yaml::Value::Tagged(_) => "tagged",
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BackupProfile {
    pub profile: Profile,
    pub yaml: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BackupBundle {
    pub schema_version: u32,
    pub settings: ApplicationSettings,
    pub selected_profile: Option<ProfileId>,
    pub profiles: Vec<BackupProfile>,
}

#[derive(Serialize, Deserialize)]
struct EncryptedBackupEnvelope {
    version: u32,
    salt: [u8; 16],
    nonce: [u8; 24],
    ciphertext: Vec<u8>,
}

pub fn encrypt_backup(bundle: &BackupBundle, passphrase: &str) -> Result<Vec<u8>, AppError> {
    validate_backup_passphrase(passphrase)?;
    validate_backup_bundle(bundle)?;
    let mut salt = [0_u8; 16];
    let mut nonce = [0_u8; 24];
    getrandom::fill(&mut salt).map_err(storage_error)?;
    getrandom::fill(&mut nonce).map_err(storage_error)?;
    let mut key = derive_backup_key(passphrase, &salt)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    let mut plaintext = serde_json::to_vec(bundle).map_err(storage_error)?;
    let encrypted = cipher.encrypt(
        XNonce::from_slice(&nonce),
        Payload {
            msg: &plaintext,
            aad: BACKUP_AAD,
        },
    );
    key.zeroize();
    plaintext.zeroize();
    let ciphertext = encrypted
        .map_err(|_| AppError::new(ErrorCode::StorageFailed, "backup encryption failed"))?;
    serde_json::to_vec(&EncryptedBackupEnvelope {
        version: BACKUP_VERSION,
        salt,
        nonce,
        ciphertext,
    })
    .map_err(storage_error)
}

pub fn decrypt_backup(bytes: &[u8], passphrase: &str) -> Result<BackupBundle, AppError> {
    validate_backup_passphrase(passphrase)?;
    let envelope: EncryptedBackupEnvelope = serde_json::from_slice(bytes).map_err(|error| {
        AppError::new(
            ErrorCode::ValidationFailed,
            format!("invalid encrypted backup: {error}"),
        )
    })?;
    if envelope.version != BACKUP_VERSION {
        return Err(AppError::new(
            ErrorCode::ValidationFailed,
            format!("unsupported backup version {}", envelope.version),
        ));
    }
    let mut key = derive_backup_key(passphrase, &envelope.salt)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    let decrypted = cipher.decrypt(
        XNonce::from_slice(&envelope.nonce),
        Payload {
            msg: &envelope.ciphertext,
            aad: BACKUP_AAD,
        },
    );
    key.zeroize();
    let mut plaintext = decrypted.map_err(|_| {
        AppError::new(
            ErrorCode::ValidationFailed,
            "backup passphrase is incorrect or backup was modified",
        )
    })?;
    let parsed = serde_json::from_slice(&plaintext).map_err(|error| {
        AppError::new(
            ErrorCode::ValidationFailed,
            format!("invalid decrypted backup: {error}"),
        )
    });
    plaintext.zeroize();
    let bundle: BackupBundle = parsed?;
    validate_backup_bundle(&bundle)?;
    Ok(bundle)
}

pub fn write_encrypted_backup(
    path: impl AsRef<Path>,
    bundle: &BackupBundle,
    passphrase: &str,
) -> Result<(), AppError> {
    let encrypted = encrypt_backup(bundle, passphrase)?;
    if let Some(parent) = path.as_ref().parent() {
        fs::create_dir_all(parent).map_err(storage_error)?;
    }
    atomic_write_private(path.as_ref(), &encrypted).map_err(storage_error)
}

fn derive_backup_key(passphrase: &str, salt: &[u8; 16]) -> Result<[u8; 32], AppError> {
    let params = Params::new(64 * 1024, 3, 1, Some(32)).map_err(storage_error)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0_u8; 32];
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(storage_error)?;
    Ok(key)
}

fn validate_backup_passphrase(passphrase: &str) -> Result<(), AppError> {
    if passphrase.chars().count() < 12 || passphrase.chars().any(|character| character == '\0') {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "backup passphrase must contain at least 12 characters and no NUL byte",
        ));
    }
    Ok(())
}

fn validate_backup_bundle(bundle: &BackupBundle) -> Result<(), AppError> {
    if bundle.schema_version != BACKUP_VERSION {
        return Err(AppError::new(
            ErrorCode::ValidationFailed,
            format!("unsupported backup schema {}", bundle.schema_version),
        ));
    }
    bundle.settings.validate()?;
    let manifest = Manifest {
        version: MANIFEST_VERSION,
        selected: bundle.selected_profile.clone(),
        profiles: bundle
            .profiles
            .iter()
            .map(|profile| profile.profile.clone())
            .collect(),
    };
    validate_manifest(&manifest)?;
    for profile in &bundle.profiles {
        validate_yaml(&profile.yaml)?;
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct SettingsFile {
    version: u32,
    settings: ApplicationSettings,
}

/// 明文设置导出:带版本号的 JSON,只含 ApplicationSettings 本身。
/// ApplicationSettings 不含 Keychain 密钥、订阅地址或运行凭据,
/// 导出文件因此可以跨机器迁移。
pub fn export_settings_json(settings: &ApplicationSettings) -> Result<Vec<u8>, AppError> {
    settings.validate()?;
    serde_json::to_vec_pretty(&SettingsFile {
        version: SETTINGS_VERSION,
        settings: settings.clone(),
    })
    .map_err(storage_error)
}

/// 解析导入文件:版本必须匹配,字段走与 UpdateApplicationSettings 相同的校验。
pub fn parse_settings_import(bytes: &[u8]) -> Result<ApplicationSettings, AppError> {
    let file: SettingsFile = serde_json::from_slice(bytes).map_err(|error| {
        AppError::new(
            ErrorCode::ValidationFailed,
            format!("invalid settings file: {error}"),
        )
    })?;
    if file.version != SETTINGS_VERSION {
        return Err(AppError::new(
            ErrorCode::ValidationFailed,
            format!("unsupported settings version {}", file.version),
        ));
    }
    file.settings.validate()?;
    Ok(file.settings)
}

/// 逐字段(old→new)差异列表,供导入前预览。按 JSON 顶层键比较,
/// ApplicationSettings 新增字段时自动纳入。
pub fn diff_application_settings(
    current: &ApplicationSettings,
    incoming: &ApplicationSettings,
) -> Vec<SettingsFieldChange> {
    let current = serde_json::to_value(current).unwrap_or_default();
    let incoming = serde_json::to_value(incoming).unwrap_or_default();
    let (Some(current), Some(incoming)) = (current.as_object(), incoming.as_object()) else {
        return Vec::new();
    };
    incoming
        .iter()
        .filter(|(field, new)| current.get(*field) != Some(*new))
        .map(|(field, new)| SettingsFieldChange {
            field: field.clone(),
            old: current.get(field).map_or_else(|| "null".into(), render_json_value),
            new: render_json_value(new),
        })
        .collect()
}

/// 导入预览:解析校验 + 与当前设置的差异。
pub fn settings_import_preview(
    current: &ApplicationSettings,
    bytes: &[u8],
) -> Result<SettingsImportPreview, AppError> {
    let settings = parse_settings_import(bytes)?;
    Ok(SettingsImportPreview {
        changes: diff_application_settings(current, &settings),
        settings,
    })
}

fn render_json_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[derive(Debug)]
pub struct FileSettingsStore {
    path: PathBuf,
    settings: ApplicationSettings,
}

impl FileSettingsStore {
    pub fn open(data_directory: impl AsRef<Path>) -> Result<Self, AppError> {
        let path = data_directory.as_ref().join("settings.json");
        let settings = if path.exists() {
            let file: SettingsFile =
                serde_json::from_slice(&fs::read(&path).map_err(storage_error)?)
                    .map_err(storage_error)?;
            if file.version != SETTINGS_VERSION {
                return Err(AppError::new(
                    ErrorCode::ValidationFailed,
                    format!("unsupported settings version {}", file.version),
                ));
            }
            file.settings.validate()?;
            file.settings
        } else {
            ApplicationSettings::default()
        };
        Ok(Self { path, settings })
    }

    pub fn get(&self) -> &ApplicationSettings {
        &self.settings
    }

    pub fn update(&mut self, settings: ApplicationSettings) -> Result<(), AppError> {
        settings.validate()?;
        let file = SettingsFile {
            version: SETTINGS_VERSION,
            settings: settings.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&file).map_err(storage_error)?;
        atomic_write_private(&self.path, &bytes).map_err(storage_error)?;
        self.settings = settings;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct Manifest {
    version: u32,
    selected: Option<ProfileId>,
    profiles: Vec<Profile>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            version: MANIFEST_VERSION,
            selected: None,
            profiles: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct FileProfileStore {
    root: PathBuf,
    manifest: Manifest,
    merge: MergeConfig,
}

impl FileProfileStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, AppError> {
        let root = root.into();
        fs::create_dir_all(root.join("profiles")).map_err(storage_error)?;
        fs::create_dir_all(root.join("snapshots")).map_err(storage_error)?;
        fs::create_dir_all(root.join("candidates")).map_err(storage_error)?;
        fs::create_dir_all(root.join("runtime")).map_err(storage_error)?;
        let path = root.join("profiles.json");
        let manifest = if path.exists() {
            let bytes = fs::read(&path).map_err(storage_error)?;
            let manifest: Manifest = serde_json::from_slice(&bytes).map_err(storage_error)?;
            validate_manifest(&manifest)?;
            manifest
        } else {
            Manifest::default()
        };
        let merge_path = root.join("merge.yaml");
        let merge = if merge_path.exists() {
            let bytes = fs::read_to_string(&merge_path).map_err(storage_error)?;
            let merge: MergeConfig = serde_yaml::from_str(&bytes).map_err(|error| {
                AppError::new(
                    ErrorCode::StorageFailed,
                    format!("invalid merge config: {error}"),
                )
            })?;
            merge.validate().map_err(|error| {
                AppError::new(
                    ErrorCode::StorageFailed,
                    format!("invalid merge config: {}", error.message),
                )
            })?;
            merge
        } else {
            MergeConfig::default()
        };
        Ok(Self {
            root,
            manifest,
            merge,
        })
    }

    pub fn list(&self) -> &[Profile] {
        &self.manifest.profiles
    }

    pub fn backup_bundle(&self, settings: ApplicationSettings) -> Result<BackupBundle, AppError> {
        let bundle = BackupBundle {
            schema_version: BACKUP_VERSION,
            settings,
            selected_profile: self.selected().cloned(),
            profiles: self
                .list()
                .iter()
                .map(|profile| {
                    Ok(BackupProfile {
                        profile: profile.clone(),
                        yaml: self.yaml(&profile.id)?,
                    })
                })
                .collect::<Result<Vec<_>, AppError>>()?,
        };
        validate_backup_bundle(&bundle)?;
        Ok(bundle)
    }

    pub fn restore_backup(
        &mut self,
        bundle: BackupBundle,
    ) -> Result<ApplicationSettings, AppError> {
        validate_backup_bundle(&bundle)?;
        let staging = self.root.join("restore-staging");
        let previous = self.root.join("restore-previous");
        remove_directory_if_present(&staging)?;
        remove_directory_if_present(&previous)?;
        fs::create_dir_all(&staging).map_err(storage_error)?;
        for profile in &bundle.profiles {
            atomic_write_private(
                &staging.join(format!("{}.yaml", profile.profile.id.as_str())),
                profile.yaml.as_bytes(),
            )
            .map_err(storage_error)?;
        }
        let new_manifest = Manifest {
            version: MANIFEST_VERSION,
            selected: bundle.selected_profile.clone(),
            profiles: bundle
                .profiles
                .iter()
                .map(|profile| profile.profile.clone())
                .collect(),
        };
        let manifest_bytes = serde_json::to_vec_pretty(&new_manifest).map_err(storage_error)?;
        let manifest_path = self.root.join("profiles.json");
        let manifest_previous = self.root.join("profiles.json.restore-previous");
        let profiles_path = self.root.join("profiles");

        match fs::remove_file(&manifest_previous) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(storage_error(error)),
        }

        fs::rename(&profiles_path, &previous).map_err(storage_error)?;
        if let Err(error) = fs::rename(&staging, &profiles_path) {
            let _ = fs::rename(&previous, &profiles_path);
            return Err(storage_error(error));
        }
        if manifest_path.exists()
            && let Err(error) = fs::rename(&manifest_path, &manifest_previous)
        {
            let _ = fs::rename(&profiles_path, &staging);
            let _ = fs::rename(&previous, &profiles_path);
            return Err(storage_error(error));
        }
        if let Err(error) = atomic_write(&manifest_path, &manifest_bytes) {
            let _ = fs::rename(&profiles_path, &staging);
            let _ = fs::rename(&previous, &profiles_path);
            let _ = fs::rename(&manifest_previous, &manifest_path);
            return Err(storage_error(error));
        }
        self.manifest = new_manifest;
        // The new profile set and manifest are committed at this point. Cleanup is
        // deliberately best-effort: a stale rollback copy must not make callers
        // believe the restore failed after the visible state has already changed.
        let _ = remove_directory_if_present(&previous);
        let _ = remove_directory_if_present(&staging);
        let _ = fs::remove_file(&manifest_previous);
        Ok(bundle.settings)
    }

    pub fn selected(&self) -> Option<&ProfileId> {
        self.manifest.selected.as_ref()
    }

    pub fn yaml(&self, id: &ProfileId) -> Result<String, AppError> {
        self.require_profile(id)?;
        fs::read_to_string(self.profile_path(id)).map_err(storage_error)
    }

    pub fn yaml_path(&self, id: &ProfileId) -> Result<PathBuf, AppError> {
        self.require_profile(id)?;
        Ok(self.profile_path(id))
    }

    pub fn merge(&self) -> &MergeConfig {
        &self.merge
    }

    /// 供界面编辑的 merge 配置 YAML 文本。
    pub fn merge_yaml(&self) -> Result<String, AppError> {
        serde_yaml::to_string(&self.merge).map_err(storage_error)
    }

    /// 解析并校验 merge 配置 YAML,原子写入后替换内存副本;失败时保持原配置。
    pub fn set_merge_yaml(&mut self, yaml: &str) -> Result<(), AppError> {
        let merge = if yaml.trim().is_empty() {
            MergeConfig::default()
        } else {
            serde_yaml::from_str(yaml).map_err(|error| {
                AppError::new(
                    ErrorCode::ValidationFailed,
                    format!("invalid merge config YAML: {error}"),
                )
            })?
        };
        merge.validate()?;
        let bytes = serde_yaml::to_string(&merge).map_err(storage_error)?;
        let previous = std::mem::replace(&mut self.merge, merge);
        if let Err(error) = atomic_write(&self.root.join("merge.yaml"), bytes.as_bytes()) {
            self.merge = previous;
            return Err(storage_error(error));
        }
        Ok(())
    }

    /// 源配置 + merge 配置 → 合并后的配置(不含私有 controller 注入),供界面预览。
    pub fn merged_yaml(&self, id: &ProfileId) -> Result<String, AppError> {
        let value = self.effective_value(id)?;
        serde_yaml::to_string(&value).map_err(storage_error)
    }

    /// 合并后的配置写入独立候选文件,供无运行凭据时的内核校验使用。
    pub fn prepare_merge_candidate(&self, id: &ProfileId) -> Result<CandidateConfig, AppError> {
        self.require_profile(id)?;
        let yaml = self.merged_yaml(id)?;
        let path = self
            .root
            .join("candidates")
            .join(format!("{}.merge.yaml", id.as_str()));
        atomic_write(&path, yaml.as_bytes()).map_err(storage_error)?;
        Ok(CandidateConfig {
            id: id.clone(),
            path,
        })
    }

    /// 源配置经 merge 增强后的结构化值。
    fn effective_value(&self, id: &ProfileId) -> Result<serde_yaml::Value, AppError> {
        let source = self.yaml(id)?;
        let value: serde_yaml::Value = serde_yaml::from_str(&source).map_err(|error| {
            AppError::new(
                ErrorCode::ValidationFailed,
                format!("invalid YAML: {error}"),
            )
        })?;
        apply_merge(&value, &self.merge)
    }

    pub fn materialize_runtime(
        &self,
        id: &ProfileId,
        controller: SocketAddr,
        secret: &str,
    ) -> Result<PathBuf, AppError> {
        let source = self.yaml(id)?;
        let runtime = render_runtime_yaml(&source, &self.merge, controller, secret)?;
        let path = self
            .root
            .join("runtime")
            .join(format!("{}.yaml", id.as_str()));
        atomic_write_private(&path, runtime.as_bytes()).map_err(storage_error)?;
        Ok(path)
    }

    pub fn system_proxy_endpoint(&self, id: &ProfileId) -> Result<ProxyEndpoint, AppError> {
        let value = self.effective_value(id)?;
        let mapping = value.as_mapping().ok_or_else(|| {
            AppError::new(
                ErrorCode::ValidationFailed,
                "profile YAML root must be a mapping",
            )
        })?;
        let port = ["mixed-port", "port"]
            .into_iter()
            .find_map(|key| {
                mapping
                    .get(serde_yaml::Value::String(key.into()))
                    .and_then(serde_yaml::Value::as_u64)
                    // Mihomo 语义：端口为 0 表示禁用该端口。mixed-port: 0 时
                    // 继续看 port；全部为 0 或无端口时给出明确错误，而不是
                    // 把 0 传给 ProxyEndpoint 产生误导性的校验错误。
                    .filter(|port| *port != 0)
            })
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::ValidationFailed,
                    "profile must define a non-zero mixed-port or port for system proxy",
                )
            })?;
        let port = u16::try_from(port).map_err(|_| {
            AppError::new(
                ErrorCode::ValidationFailed,
                "profile system proxy port is outside 1...65535",
            )
        })?;
        ProxyEndpoint::new("127.0.0.1", port)
    }

    pub fn system_proxy_socks_endpoint(
        &self,
        id: &ProfileId,
    ) -> Result<Option<ProxyEndpoint>, AppError> {
        let value = self.effective_value(id)?;
        let mapping = value.as_mapping().ok_or_else(|| {
            AppError::new(
                ErrorCode::ValidationFailed,
                "profile YAML root must be a mapping",
            )
        })?;
        let port = ["mixed-port", "socks-port"].into_iter().find_map(|key| {
            mapping
                .get(serde_yaml::Value::String(key.into()))
                .and_then(serde_yaml::Value::as_u64)
                // 同样跳过 0：mixed-port: 0 时回落到 socks-port。
                .filter(|port| *port != 0)
        });
        port.map(|port| {
            u16::try_from(port)
                .map_err(|_| {
                    AppError::new(
                        ErrorCode::ValidationFailed,
                        "profile SOCKS proxy port is outside 1...65535",
                    )
                })
                .and_then(|port| ProxyEndpoint::new("127.0.0.1", port))
        })
        .transpose()
    }

    pub fn prepare_runtime_candidate(
        &self,
        id: &ProfileId,
        yaml: &str,
        controller: SocketAddr,
        secret: &str,
    ) -> Result<CandidateConfig, AppError> {
        self.require_profile(id)?;
        let runtime = render_runtime_yaml(yaml, &self.merge, controller, secret)?;
        let path = self
            .root
            .join("candidates")
            .join(format!("{}.runtime.yaml", id.as_str()));
        atomic_write_private(&path, runtime.as_bytes()).map_err(storage_error)?;
        Ok(CandidateConfig {
            id: id.clone(),
            path,
        })
    }

    pub fn prepare_candidate(
        &self,
        id: &ProfileId,
        yaml: &str,
    ) -> Result<CandidateConfig, AppError> {
        self.require_profile(id)?;
        validate_yaml(yaml)?;
        let path = self
            .root
            .join("candidates")
            .join(format!("{}.yaml", id.as_str()));
        atomic_write(&path, yaml.as_bytes()).map_err(storage_error)?;
        Ok(CandidateConfig {
            id: id.clone(),
            path,
        })
    }

    pub fn commit_candidate(
        &mut self,
        candidate: CandidateConfig,
        now: i64,
    ) -> Result<(), AppError> {
        let yaml = fs::read_to_string(candidate.path()).map_err(storage_error)?;
        self.replace_yaml(candidate.id(), &yaml, now)
    }

    pub fn import(&mut self, profile: Profile, yaml: &str) -> Result<(), AppError> {
        validate_yaml(yaml)?;
        if self
            .manifest
            .profiles
            .iter()
            .any(|item| item.id == profile.id)
        {
            return Err(AppError::new(ErrorCode::Conflict, "profile already exists"));
        }

        let snapshot_path = self.snapshot_path(&profile.id);
        match fs::remove_file(snapshot_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(storage_error(error)),
        }
        atomic_write(&self.profile_path(&profile.id), yaml.as_bytes()).map_err(storage_error)?;
        self.manifest.profiles.push(profile);
        if let Err(error) = self.save_manifest() {
            let profile = self
                .manifest
                .profiles
                .pop()
                .expect("profile was just pushed");
            let _ = fs::remove_file(self.profile_path(&profile.id));
            return Err(error);
        }
        Ok(())
    }

    pub fn replace_yaml(&mut self, id: &ProfileId, yaml: &str, now: i64) -> Result<(), AppError> {
        validate_yaml(yaml)?;
        let index = self.profile_index(id)?;
        let old_yaml = self.yaml(id)?;
        atomic_write(&self.snapshot_path(id), old_yaml.as_bytes()).map_err(storage_error)?;
        atomic_write(&self.profile_path(id), yaml.as_bytes()).map_err(storage_error)?;

        let old_profile = self.manifest.profiles[index].clone();
        self.manifest.profiles[index].updated_at = now;
        self.manifest.profiles[index].next_update_at =
            next_run(&self.manifest.profiles[index].update_policy, now)?;
        self.manifest.profiles[index].consecutive_failures = 0;
        self.manifest.profiles[index].last_error = None;
        if let Err(error) = self.save_manifest() {
            self.manifest.profiles[index] = old_profile;
            let _ = atomic_write(&self.profile_path(id), old_yaml.as_bytes());
            return Err(error);
        }
        Ok(())
    }

    pub fn rollback_to_last_good(&mut self, id: &ProfileId, now: i64) -> Result<(), AppError> {
        self.require_profile(id)?;
        let snapshot = fs::read_to_string(self.snapshot_path(id)).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                AppError::new(ErrorCode::NotFound, "last good profile snapshot not found")
            } else {
                storage_error(error)
            }
        })?;
        validate_yaml(&snapshot)?;
        atomic_write(&self.profile_path(id), snapshot.as_bytes()).map_err(storage_error)?;

        let index = self.profile_index(id)?;
        let previous = self.manifest.profiles[index].clone();
        self.manifest.profiles[index].updated_at = now;
        self.manifest.profiles[index].next_update_at =
            next_run(&self.manifest.profiles[index].update_policy, now)?;
        if let Err(error) = self.save_manifest() {
            self.manifest.profiles[index] = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn select(&mut self, id: &ProfileId) -> Result<(), AppError> {
        self.require_profile(id)?;
        let previous = self.manifest.selected.replace(id.clone());
        if let Err(error) = self.save_manifest() {
            self.manifest.selected = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn clear_selection(&mut self) -> Result<(), AppError> {
        let previous = self.manifest.selected.take();
        if let Err(error) = self.save_manifest() {
            self.manifest.selected = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn delete(&mut self, id: &ProfileId) -> Result<(), AppError> {
        let index = self.profile_index(id)?;
        let profile_path = self.profile_path(id);
        let deleted_path = profile_path.with_extension("deleted");
        match fs::rename(&profile_path, &deleted_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(storage_error(error)),
        }

        let profile = self.manifest.profiles.remove(index);
        let previous_selected = self.manifest.selected.clone();
        if self.manifest.selected.as_ref() == Some(id) {
            self.manifest.selected = None;
        }
        if let Err(error) = self.save_manifest() {
            self.manifest.profiles.insert(index, profile);
            self.manifest.selected = previous_selected;
            let _ = fs::rename(&deleted_path, &profile_path);
            return Err(error);
        }
        let _ = fs::remove_file(deleted_path);
        let _ = fs::remove_file(self.snapshot_path(id));
        Ok(())
    }

    pub fn due_updates(&self, now: i64) -> Vec<&Profile> {
        self.manifest
            .profiles
            .iter()
            .filter(|profile| profile.next_update_at.is_some_and(|due| due <= now))
            .collect()
    }

    pub fn mark_update_failed(
        &mut self,
        id: &ProfileId,
        now: i64,
        error: &AppError,
    ) -> Result<(), AppError> {
        let index = self.profile_index(id)?;
        if !matches!(
            self.manifest.profiles[index].source,
            ProfileSource::Remote { .. }
        ) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "local profiles do not have subscription update failures",
            ));
        }
        let previous = self.manifest.profiles[index].clone();
        let profile = &mut self.manifest.profiles[index];
        profile.consecutive_failures = profile.consecutive_failures.saturating_add(1);
        profile.last_error = Some(error.message.clone());
        profile.next_update_at = match profile.update_policy {
            UpdatePolicy::Manual => None,
            UpdatePolicy::Interval { .. } => {
                let shift = profile.consecutive_failures.saturating_sub(1).min(6);
                let delay = 60_i64.saturating_mul(1_i64 << shift).min(3_600);
                Some(now.checked_add(delay).ok_or_else(|| {
                    AppError::new(ErrorCode::InvalidInput, "update retry time overflowed")
                })?)
            }
        };
        if let Err(error) = self.save_manifest() {
            self.manifest.profiles[index] = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn set_update_policy(
        &mut self,
        id: &ProfileId,
        update_policy: UpdatePolicy,
        now: i64,
    ) -> Result<(), AppError> {
        let index = self.profile_index(id)?;
        if !matches!(
            self.manifest.profiles[index].source,
            ProfileSource::Remote { .. }
        ) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "only remote profiles have an update policy",
            ));
        }
        let next_update_at = next_run(&update_policy, now)?;
        let previous = self.manifest.profiles[index].clone();
        let profile = &mut self.manifest.profiles[index];
        profile.update_policy = update_policy;
        profile.next_update_at = next_update_at;
        profile.consecutive_failures = 0;
        profile.last_error = None;
        if let Err(error) = self.save_manifest() {
            self.manifest.profiles[index] = previous;
            return Err(error);
        }
        Ok(())
    }

    fn require_profile(&self, id: &ProfileId) -> Result<(), AppError> {
        self.profile_index(id).map(|_| ())
    }

    fn profile_index(&self, id: &ProfileId) -> Result<usize, AppError> {
        self.manifest
            .profiles
            .iter()
            .position(|profile| &profile.id == id)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, "profile not found"))
    }

    fn profile_path(&self, id: &ProfileId) -> PathBuf {
        self.root
            .join("profiles")
            .join(format!("{}.yaml", id.as_str()))
    }

    fn snapshot_path(&self, id: &ProfileId) -> PathBuf {
        self.root
            .join("snapshots")
            .join(format!("{}.yaml", id.as_str()))
    }

    fn save_manifest(&self) -> Result<(), AppError> {
        let bytes = serde_json::to_vec_pretty(&self.manifest).map_err(storage_error)?;
        atomic_write(&self.root.join("profiles.json"), &bytes).map_err(storage_error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdateTrigger {
    Manual,
    Scheduled,
    Startup,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileUpdateJob {
    pub id: ProfileId,
    pub url: String,
    pub trigger: UpdateTrigger,
    pub user_agent: Option<String>,
}

#[derive(Default)]
pub struct UpdateScheduler {
    in_flight: HashSet<ProfileId>,
}

impl UpdateScheduler {
    pub fn claim_manual(
        &mut self,
        store: &FileProfileStore,
        id: &ProfileId,
    ) -> Result<ProfileUpdateJob, AppError> {
        let profile = store
            .list()
            .iter()
            .find(|profile| &profile.id == id)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, "profile not found"))?;
        self.claim(profile, UpdateTrigger::Manual)
    }

    pub fn claim_due(
        &mut self,
        store: &FileProfileStore,
        now: i64,
        trigger: UpdateTrigger,
    ) -> Result<Vec<ProfileUpdateJob>, AppError> {
        if trigger == UpdateTrigger::Manual {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "manual updates must claim a specific profile",
            ));
        }
        let due = store
            .due_updates(now)
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        due.iter()
            .map(|profile| self.claim(profile, trigger))
            .collect()
    }

    pub fn finish(&mut self, id: &ProfileId) {
        self.in_flight.remove(id);
    }

    fn claim(
        &mut self,
        profile: &Profile,
        trigger: UpdateTrigger,
    ) -> Result<ProfileUpdateJob, AppError> {
        let ProfileSource::Remote { url } = &profile.source else {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "only remote profiles can be updated",
            ));
        };
        if !self.in_flight.insert(profile.id.clone()) {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "profile update is already running",
            ));
        }
        Ok(ProfileUpdateJob {
            id: profile.id.clone(),
            url: url.clone(),
            trigger,
            user_agent: profile.user_agent.clone(),
        })
    }
}

#[derive(Debug)]
pub struct CandidateConfig {
    id: ProfileId,
    path: PathBuf,
}

impl CandidateConfig {
    pub fn id(&self) -> &ProfileId {
        &self.id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for CandidateConfig {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn next_run(policy: &UpdatePolicy, now: i64) -> Result<Option<i64>, AppError> {
    match policy {
        UpdatePolicy::Manual => Ok(None),
        UpdatePolicy::Interval { seconds: 0 } => Err(AppError::new(
            ErrorCode::InvalidInput,
            "update interval must be greater than zero",
        )),
        UpdatePolicy::Interval { seconds } => {
            let seconds = i64::try_from(*seconds).map_err(|_| {
                AppError::new(ErrorCode::InvalidInput, "update interval is too large")
            })?;
            now.checked_add(seconds).map(Some).ok_or_else(|| {
                AppError::new(ErrorCode::InvalidInput, "next update time overflowed")
            })
        }
    }
}

fn validate_manifest(manifest: &Manifest) -> Result<(), AppError> {
    if manifest.version != MANIFEST_VERSION {
        return Err(AppError::new(
            ErrorCode::StorageFailed,
            format!("unsupported profile manifest version: {}", manifest.version),
        ));
    }
    for (index, profile) in manifest.profiles.iter().enumerate() {
        next_run(&profile.update_policy, profile.updated_at).map_err(|error| {
            AppError::new(
                ErrorCode::StorageFailed,
                format!("invalid profile manifest: {}", error.message),
            )
        })?;
        if manifest.profiles[..index]
            .iter()
            .any(|other| other.id == profile.id)
        {
            return Err(AppError::new(
                ErrorCode::StorageFailed,
                "profile manifest contains duplicate ids",
            ));
        }
    }
    if let Some(selected) = &manifest.selected
        && !manifest
            .profiles
            .iter()
            .any(|profile| &profile.id == selected)
    {
        return Err(AppError::new(
            ErrorCode::StorageFailed,
            "selected profile does not exist in manifest",
        ));
    }
    Ok(())
}

fn validate_yaml(yaml: &str) -> Result<(), AppError> {
    let value: serde_yaml::Value = serde_yaml::from_str(yaml).map_err(|error| {
        AppError::new(
            ErrorCode::ValidationFailed,
            format!("invalid YAML: {error}"),
        )
    })?;
    if !value.is_mapping() {
        return Err(AppError::new(
            ErrorCode::ValidationFailed,
            "profile YAML root must be a mapping",
        ));
    }
    Ok(())
}

fn render_runtime_yaml(
    yaml: &str,
    merge: &MergeConfig,
    controller: SocketAddr,
    secret: &str,
) -> Result<String, AppError> {
    if !controller.ip().is_loopback() || secret.is_empty() || secret.chars().any(char::is_control) {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "runtime controller must be loopback and secret must be valid",
        ));
    }
    let value: serde_yaml::Value = serde_yaml::from_str(yaml).map_err(|error| {
        AppError::new(
            ErrorCode::ValidationFailed,
            format!("invalid YAML: {error}"),
        )
    })?;
    let mut value = apply_merge(&value, merge)?;
    let mapping = value.as_mapping_mut().ok_or_else(|| {
        AppError::new(
            ErrorCode::ValidationFailed,
            "profile YAML root must be a mapping",
        )
    })?;
    mapping.insert(
        serde_yaml::Value::String("external-controller".into()),
        serde_yaml::Value::String(controller.to_string()),
    );
    mapping.insert(
        serde_yaml::Value::String("secret".into()),
        serde_yaml::Value::String(secret.to_owned()),
    );
    serde_yaml::to_string(&value).map_err(storage_error)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    let mut file = fs::File::create(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(temporary, path)
}

fn remove_directory_if_present(path: &Path) -> Result<(), AppError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(storage_error(error)),
    }
}

fn atomic_write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(temporary, path)
}

fn storage_error(error: impl fmt::Display) -> AppError {
    AppError::new(ErrorCode::StorageFailed, error.to_string())
}

use std::fmt;

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "verge-config-{name}-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn profile(id: &str, policy: UpdatePolicy) -> Profile {
        Profile::new(
            ProfileId::parse(id).unwrap(),
            "Daily",
            ProfileSource::Remote {
                url: "https://example.com/profile.yaml".into(),
            },
            policy,
            1_000,
                    None)
        .unwrap()
    }

    #[test]
    fn settings_are_versioned_validated_and_persisted() {
        let directory = TestDir::new("settings");
        let mut store = FileSettingsStore::open(&directory.0).unwrap();
        assert_eq!(store.get(), &ApplicationSettings::default());
        let settings = ApplicationSettings {
            theme: verge_domain::ThemePreference::Dark,
            language: "zh-CN".into(),
            log_limit: 1_000,
            launch_at_login: true,
            global_hotkey: Some("CmdOrCtrl+Shift+V".into()),
        };
        store.update(settings.clone()).unwrap();
        assert_eq!(
            FileSettingsStore::open(&directory.0).unwrap().get(),
            &settings
        );

        let invalid = ApplicationSettings {
            log_limit: 99,
            ..settings
        };
        assert_eq!(
            store.update(invalid).unwrap_err().code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn encrypted_backup_round_trips_and_rejects_wrong_password_or_tampering() {
        let directory = TestDir::new("encrypted-backup");
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        let id = ProfileId::parse("private").unwrap();
        store
            .import(
                Profile::new(
                    id.clone(),
                    "Private",
                    ProfileSource::Remote {
                        url: "https://user:secret@example.com/sub".into(),
                    },
                    UpdatePolicy::Manual,
                    1_000,
                    None)
                .unwrap(),
                "mixed-port: 7890\nsecret: private-yaml-secret\n",
            )
            .unwrap();
        store.select(&id).unwrap();
        let bundle = store.backup_bundle(ApplicationSettings::default()).unwrap();
        let encrypted = encrypt_backup(&bundle, "correct horse battery staple").unwrap();
        let encoded = String::from_utf8_lossy(&encrypted);
        assert!(!encoded.contains("private-yaml-secret"));
        assert!(!encoded.contains("user:secret"));
        assert_eq!(
            decrypt_backup(&encrypted, "correct horse battery staple").unwrap(),
            bundle
        );
        assert_eq!(
            decrypt_backup(&encrypted, "incorrect password")
                .unwrap_err()
                .code,
            ErrorCode::ValidationFailed
        );
        let mut tampered = encrypted;
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(decrypt_backup(&tampered, "correct horse battery staple").is_err());
    }

    #[test]
    fn validated_backup_restore_replaces_profiles_and_selection_transactionally() {
        let source_directory = TestDir::new("backup-source");
        let mut source = FileProfileStore::open(&source_directory.0).unwrap();
        let restored_id = ProfileId::parse("restored").unwrap();
        source
            .import(
                Profile::new(
                    restored_id.clone(),
                    "Restored",
                    ProfileSource::Local,
                    UpdatePolicy::Manual,
                    2_000,
                    None)
                .unwrap(),
                "mixed-port: 7891\n",
            )
            .unwrap();
        source.select(&restored_id).unwrap();
        let settings = ApplicationSettings {
            language: "zh-CN".into(),
            ..ApplicationSettings::default()
        };
        let bundle = source.backup_bundle(settings.clone()).unwrap();

        let destination_directory = TestDir::new("backup-destination");
        let mut destination = FileProfileStore::open(&destination_directory.0).unwrap();
        destination
            .import(profile("old", UpdatePolicy::Manual), "mixed-port: 7890\n")
            .unwrap();
        fs::write(
            destination_directory
                .0
                .join("profiles.json.restore-previous"),
            b"stale rollback manifest",
        )
        .unwrap();
        assert_eq!(destination.restore_backup(bundle).unwrap(), settings);
        assert_eq!(destination.list().len(), 1);
        assert_eq!(destination.selected(), Some(&restored_id));
        assert_eq!(
            destination.yaml(&restored_id).unwrap(),
            "mixed-port: 7891\n"
        );
        assert!(FileProfileStore::open(&destination_directory.0).is_ok());
        assert!(
            !destination_directory
                .0
                .join("profiles.json.restore-previous")
                .exists()
        );

        let invalid = BackupBundle {
            schema_version: BACKUP_VERSION,
            settings: ApplicationSettings::default(),
            selected_profile: Some(ProfileId::parse("missing").unwrap()),
            profiles: Vec::new(),
        };
        assert!(destination.restore_backup(invalid).is_err());
        assert_eq!(destination.selected(), Some(&restored_id));
    }

    #[test]
    fn persists_import_selection_and_raw_yaml() {
        let directory = TestDir::new("persistence");
        let id = ProfileId::parse("daily").unwrap();
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        store
            .import(profile("daily", UpdatePolicy::Manual), "mode: rule\n")
            .unwrap();
        store.select(&id).unwrap();

        let reopened = FileProfileStore::open(&directory.0).unwrap();
        assert_eq!(reopened.selected(), Some(&id));
        assert_eq!(reopened.yaml(&id).unwrap(), "mode: rule\n");
    }

    #[test]
    fn invalid_replacement_keeps_last_good_yaml() {
        let directory = TestDir::new("rollback");
        let id = ProfileId::parse("daily").unwrap();
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        store
            .import(profile("daily", UpdatePolicy::Manual), "mode: rule\n")
            .unwrap();

        let error = store
            .replace_yaml(&id, "- not-a-mapping\n", 2_000)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::ValidationFailed);
        assert_eq!(store.yaml(&id).unwrap(), "mode: rule\n");
    }

    #[test]
    fn scheduler_returns_only_due_remote_profiles() {
        let directory = TestDir::new("scheduler");
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        store
            .import(profile("manual", UpdatePolicy::Manual), "mode: direct\n")
            .unwrap();
        store
            .import(
                profile("scheduled", UpdatePolicy::Interval { seconds: 60 }),
                "mode: rule\n",
            )
            .unwrap();

        assert!(store.due_updates(1_059).is_empty());
        assert_eq!(store.due_updates(1_060)[0].id.as_str(), "scheduled");
    }

    #[test]
    fn update_scheduler_deduplicates_jobs_and_persists_failure_backoff() {
        let directory = TestDir::new("update-scheduler");
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        let id = ProfileId::parse("remote").unwrap();
        store
            .import(
                Profile::new(
                    id.clone(),
                    "Remote",
                    ProfileSource::Remote {
                        url: "https://example.com/profile.yaml".into(),
                    },
                    UpdatePolicy::Interval { seconds: 300 },
                    100,
                    None)
                .unwrap(),
                "mode: rule\n",
            )
            .unwrap();
        let mut scheduler = UpdateScheduler::default();
        let job = scheduler.claim_manual(&store, &id).unwrap();
        assert_eq!(job.trigger, UpdateTrigger::Manual);
        assert_eq!(
            scheduler.claim_manual(&store, &id).unwrap_err().code,
            ErrorCode::Conflict
        );
        scheduler.finish(&id);

        store
            .mark_update_failed(
                &id,
                200,
                &AppError::new(ErrorCode::CoreUnavailable, "offline"),
            )
            .unwrap();
        let profile = &store.list()[0];
        assert_eq!(profile.consecutive_failures, 1);
        assert_eq!(profile.next_update_at, Some(260));
        assert_eq!(profile.last_error.as_deref(), Some("offline"));
        assert!(
            scheduler
                .claim_due(&store, 259, UpdateTrigger::Scheduled)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            scheduler
                .claim_due(&store, 260, UpdateTrigger::Startup)
                .unwrap()[0]
                .trigger,
            UpdateTrigger::Startup
        );
        scheduler.finish(&id);
        store
            .set_update_policy(&id, UpdatePolicy::Interval { seconds: 900 }, 300)
            .unwrap();
        assert_eq!(store.list()[0].next_update_at, Some(1_200));
        assert_eq!(store.list()[0].consecutive_failures, 0);
        assert_eq!(store.list()[0].last_error, None);
    }

    #[test]
    fn deleting_selected_profile_clears_selection() {
        let directory = TestDir::new("delete");
        let id = ProfileId::parse("daily").unwrap();
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        store
            .import(profile("daily", UpdatePolicy::Manual), "mode: rule\n")
            .unwrap();
        store.select(&id).unwrap();
        store.delete(&id).unwrap();

        assert!(store.selected().is_none());
        assert!(store.list().is_empty());
    }

    #[test]
    fn rolls_back_to_snapshot_after_a_valid_but_unhealthy_candidate() {
        let directory = TestDir::new("snapshot");
        let id = ProfileId::parse("daily").unwrap();
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        store
            .import(profile("daily", UpdatePolicy::Manual), "mode: rule\n")
            .unwrap();
        store.replace_yaml(&id, "mode: global\n", 2_000).unwrap();

        store.rollback_to_last_good(&id, 2_001).unwrap();

        assert_eq!(store.yaml(&id).unwrap(), "mode: rule\n");
        assert_eq!(store.list()[0].updated_at, 2_001);
    }

    #[test]
    fn rejects_manifest_with_path_traversal_profile_id() {
        let directory = TestDir::new("unsafe-manifest");
        fs::write(
            directory.0.join("profiles.json"),
            r#"{
                "version": 1,
                "selected": null,
                "profiles": [{
                    "id": "../escape",
                    "name": "Unsafe",
                    "source": { "type": "local" },
                    "update_policy": { "type": "manual" },
                    "updated_at": 1000,
                    "next_update_at": null
                }]
            }"#,
        )
        .unwrap();

        let error = FileProfileStore::open(&directory.0).unwrap_err();
        assert_eq!(error.code, ErrorCode::StorageFailed);
        assert!(error.message.contains("profile id"));
    }

    #[test]
    fn rejects_manifest_with_missing_selected_profile() {
        let directory = TestDir::new("missing-selected");
        fs::write(
            directory.0.join("profiles.json"),
            r#"{
                "version": 1,
                "selected": "missing",
                "profiles": []
            }"#,
        )
        .unwrap();

        let error = FileProfileStore::open(&directory.0).unwrap_err();
        assert_eq!(error.code, ErrorCode::StorageFailed);
        assert!(error.message.contains("selected profile"));
    }

    #[test]
    fn reimport_does_not_inherit_deleted_profile_snapshot() {
        let directory = TestDir::new("stale-snapshot");
        let id = ProfileId::parse("daily").unwrap();
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        store
            .import(profile("daily", UpdatePolicy::Manual), "mode: rule\n")
            .unwrap();
        store.replace_yaml(&id, "mode: global\n", 2_000).unwrap();
        store.delete(&id).unwrap();
        store
            .import(profile("daily", UpdatePolicy::Manual), "mode: direct\n")
            .unwrap();

        let error = store.rollback_to_last_good(&id, 3_000).unwrap_err();
        assert_eq!(error.code, ErrorCode::NotFound);
        assert_eq!(store.yaml(&id).unwrap(), "mode: direct\n");
    }

    #[test]
    fn candidate_is_isolated_until_commit_and_cleaned_afterward() {
        let directory = TestDir::new("candidate");
        let id = ProfileId::parse("daily").unwrap();
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        store
            .import(profile("daily", UpdatePolicy::Manual), "mode: rule\n")
            .unwrap();

        let candidate = store.prepare_candidate(&id, "mode: global\n").unwrap();
        let candidate_path = candidate.path().to_owned();
        assert_eq!(store.yaml(&id).unwrap(), "mode: rule\n");
        assert!(candidate_path.is_file());

        store.commit_candidate(candidate, 2_000).unwrap();
        assert_eq!(store.yaml(&id).unwrap(), "mode: global\n");
        assert!(!candidate_path.exists());
    }

    #[test]
    fn runtime_config_injects_private_controller_without_changing_profile() {
        let directory = TestDir::new("runtime");
        let id = ProfileId::parse("daily").unwrap();
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        let source = "mode: rule\nexternal-controller: 127.0.0.1:1\nsecret: old\n";
        store
            .import(profile("daily", UpdatePolicy::Manual), source)
            .unwrap();

        let path = store
            .materialize_runtime(&id, "127.0.0.1:45678".parse().unwrap(), "install-secret")
            .unwrap();
        let runtime: serde_yaml::Value =
            serde_yaml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(runtime["external-controller"], "127.0.0.1:45678");
        assert_eq!(runtime["secret"], "install-secret");
        assert_eq!(store.yaml(&id).unwrap(), source);
    }

    #[test]
    fn system_proxy_endpoint_prefers_mixed_port_and_rejects_missing_port() {
        let directory = TestDir::new("proxy-endpoint");
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        let mixed = ProfileId::parse("mixed").unwrap();
        store
            .import(
                Profile::new(
                    mixed.clone(),
                    "Mixed",
                    ProfileSource::Local,
                    UpdatePolicy::Manual,
                    100,
                    None)
                .unwrap(),
                "mixed-port: 7893\nport: 7890\n",
            )
            .unwrap();
        assert_eq!(
            store.system_proxy_endpoint(&mixed).unwrap(),
            ProxyEndpoint::new("127.0.0.1", 7893).unwrap()
        );

        let missing = ProfileId::parse("missing").unwrap();
        store
            .import(
                Profile::new(
                    missing.clone(),
                    "Missing",
                    ProfileSource::Local,
                    UpdatePolicy::Manual,
                    100,
                    None)
                .unwrap(),
                "mode: rule\n",
            )
            .unwrap();
        assert_eq!(
            store.system_proxy_endpoint(&missing).unwrap_err().code,
            ErrorCode::ValidationFailed
        );
    }

    #[test]
    fn system_proxy_socks_endpoint_prefers_mixed_port_and_is_optional() {
        let directory = TestDir::new("socks-endpoint");
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        fn import(store: &mut FileProfileStore, id: &str, yaml: &str) -> ProfileId {
            store
                .import(
                    Profile::new(
                        ProfileId::parse(id).unwrap(),
                        id,
                        ProfileSource::Local,
                        UpdatePolicy::Manual,
                        100,
                    None)
                    .unwrap(),
                    yaml,
                )
                .unwrap();
            ProfileId::parse(id).unwrap()
        }

        let mixed = import(&mut store, "mixed", "mixed-port: 7893\nsocks-port: 7891\n");
        assert_eq!(
            store.system_proxy_socks_endpoint(&mixed).unwrap(),
            Some(ProxyEndpoint::new("127.0.0.1", 7893).unwrap())
        );

        let socks_only = import(&mut store, "socks-only", "port: 7890\nsocks-port: 7891\n");
        assert_eq!(
            store.system_proxy_socks_endpoint(&socks_only).unwrap(),
            Some(ProxyEndpoint::new("127.0.0.1", 7891).unwrap())
        );

        let http_only = import(&mut store, "http-only", "port: 7890\n");
        assert_eq!(
            store.system_proxy_socks_endpoint(&http_only).unwrap(),
            None
        );
    }

    fn merge_config(yaml: &str) -> MergeConfig {
        serde_yaml::from_str(yaml).unwrap()
    }

    fn source_value(yaml: &str) -> serde_yaml::Value {
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn merge_override_merge_and_remove_top_level_keys() {
        let source = source_value("mode: rule\nlog-level: info\ndns:\n  enable: false\n  ipv6: false\n");
        let merge = merge_config(
            "rules:\n  - key: mode\n    op: override\n    value: global\n  - key: dns\n    op: merge\n    value:\n      enable: true\n  - key: log-level\n    op: remove\n",
        );
        let merged = apply_merge(&source, &merge).unwrap();
        assert_eq!(merged["mode"], "global");
        assert_eq!(merged["dns"]["enable"], true);
        assert_eq!(merged["dns"]["ipv6"], false);
        assert!(merged.get("log-level").is_none());
    }

    #[test]
    fn merge_sequence_ops_prepend_append_and_name_merge() {
        let source = source_value(
            "rules:\n  - DOMAIN,example.com,DIRECT\nproxies:\n  - name: a\n    server: old\n    port: 1\n",
        );
        let merge = merge_config(
            "rules:\n  - key: rules\n    op: prepend\n    items:\n      - DOMAIN,first.example.com,REJECT\n  - key: rules\n    op: append\n    items:\n      - MATCH,PROXY\n  - key: proxies\n    op: merge\n    value:\n      - name: a\n        server: new\n      - name: b\n        server: added\n",
        );
        let merged = apply_merge(&source, &merge).unwrap();
        let rules = merged["rules"].as_sequence().unwrap();
        assert_eq!(rules[0], "DOMAIN,first.example.com,REJECT");
        assert_eq!(rules[1], "DOMAIN,example.com,DIRECT");
        assert_eq!(rules[2], "MATCH,PROXY");
        let proxies = merged["proxies"].as_sequence().unwrap();
        assert_eq!(proxies.len(), 2);
        assert_eq!(proxies[0]["server"], "new");
        assert_eq!(proxies[0]["port"], 1);
        assert_eq!(proxies[1]["name"], "b");
    }

    #[test]
    fn merge_sequence_ops_accept_missing_target_as_empty() {
        let source = source_value("mode: rule\n");
        let merge = merge_config(
            "rules:\n  - key: rules\n    op: append\n    items:\n      - MATCH,DIRECT\n  - key: new-key\n    op: merge\n    value: true\n",
        );
        let merged = apply_merge(&source, &merge).unwrap();
        assert_eq!(merged["rules"].as_sequence().unwrap().len(), 1);
        assert_eq!(merged["new-key"], true);
    }

    #[test]
    fn merge_rejects_type_conflicts_and_reserved_keys() {
        let source = source_value("mode: rule\nrules:\n  - MATCH,DIRECT\n");
        let conflict = merge_config(
            "rules:\n  - key: rules\n    op: merge\n    value:\n      nested: true\n",
        );
        let error = apply_merge(&source, &conflict).unwrap_err();
        assert_eq!(error.code, ErrorCode::ValidationFailed);
        assert!(error.message.contains("type conflict"));

        let prepend_scalar = merge_config(
            "rules:\n  - key: mode\n    op: prepend\n    items:\n      - x\n",
        );
        assert_eq!(
            apply_merge(&source, &prepend_scalar).unwrap_err().code,
            ErrorCode::ValidationFailed
        );

        for reserved in ["external-controller", "secret"] {
            let merge = merge_config(&format!(
                "rules:\n  - key: {reserved}\n    op: override\n    value: x\n"
            ));
            assert_eq!(
                apply_merge(&source, &merge).unwrap_err().code,
                ErrorCode::InvalidInput
            );
        }
        let bad_key = merge_config("rules:\n  - key: dns.servers\n    op: remove\n");
        assert_eq!(
            apply_merge(&source, &bad_key).unwrap_err().code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn merge_config_persists_and_round_trips() {
        let directory = TestDir::new("merge-persistence");
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        assert_eq!(store.merge(), &MergeConfig::default());
        store
            .set_merge_yaml("rules:\n  - key: mode\n    op: override\n    value: global\n")
            .unwrap();

        let reopened = FileProfileStore::open(&directory.0).unwrap();
        assert_eq!(reopened.merge().rules.len(), 1);
        assert!(reopened.merge_yaml().unwrap().contains("override"));

        assert_eq!(
            store
                .set_merge_yaml("rules: [not-a-rule]\n")
                .unwrap_err()
                .code,
            ErrorCode::ValidationFailed
        );
        assert_eq!(
            store
                .set_merge_yaml("rules:\n  - key: secret\n    op: remove\n")
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
        assert_eq!(store.merge().rules.len(), 1);
        // 清空文本回到默认空配置。
        store.set_merge_yaml("\n").unwrap();
        assert_eq!(store.merge(), &MergeConfig::default());
    }

    #[test]
    fn settings_export_contains_only_versioned_settings_without_secrets() {
        let settings = ApplicationSettings {
            theme: verge_domain::ThemePreference::Dark,
            language: "zh-CN".into(),
            log_limit: 1_000,
            ..ApplicationSettings::default()
        };
        let bytes = export_settings_json(&settings).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let object = value.as_object().unwrap();
        assert_eq!(
            object.keys().map(String::as_str).collect::<Vec<_>>(),
            ["settings", "version"]
        );
        assert_eq!(value["version"], 1);
        assert_eq!(
            value["settings"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["global_hotkey", "language", "launch_at_login", "log_limit", "theme"]
        );
        let text = String::from_utf8(bytes.clone()).unwrap();
        for forbidden in ["secret", "keychain", "password", "token"] {
            assert!(!text.to_lowercase().contains(forbidden));
        }
        assert_eq!(parse_settings_import(&bytes).unwrap(), settings);
    }

    #[test]
    fn settings_import_rejects_version_mismatch_and_invalid_values() {
        let wrong_version = br#"{"version": 99, "settings": {"theme": "dark", "language": "en", "log_limit": 500}}"#;
        assert_eq!(
            parse_settings_import(wrong_version).unwrap_err().code,
            ErrorCode::ValidationFailed
        );
        let invalid = br#"{"version": 1, "settings": {"theme": "dark", "language": "en", "log_limit": 99}}"#;
        assert_eq!(
            parse_settings_import(invalid).unwrap_err().code,
            ErrorCode::InvalidInput
        );
        assert_eq!(
            parse_settings_import(b"not json").unwrap_err().code,
            ErrorCode::ValidationFailed
        );
    }

    #[test]
    fn settings_import_preview_lists_field_changes() {
        let current = ApplicationSettings::default();
        let incoming = ApplicationSettings {
            theme: verge_domain::ThemePreference::Dark,
            language: "zh-CN".into(),
            log_limit: 500,
            ..ApplicationSettings::default()
        };
        let bytes = export_settings_json(&incoming).unwrap();
        let preview = settings_import_preview(&current, &bytes).unwrap();
        assert_eq!(preview.settings, incoming);
        assert_eq!(
            preview.changes,
            vec![
                SettingsFieldChange {
                    field: "language".into(),
                    old: "en".into(),
                    new: "zh-CN".into(),
                },
                SettingsFieldChange {
                    field: "theme".into(),
                    old: "system".into(),
                    new: "dark".into(),
                },
            ]
        );
        assert!(diff_application_settings(&incoming, &incoming).is_empty());
    }

    #[test]
    fn corrupt_merge_file_fails_open() {
        let directory = TestDir::new("merge-corrupt");
        fs::write(directory.0.join("merge.yaml"), "rules: 1\n").unwrap();
        let error = FileProfileStore::open(&directory.0).unwrap_err();
        assert_eq!(error.code, ErrorCode::StorageFailed);
        assert!(error.message.contains("merge config"));
    }

    #[test]
    fn runtime_materialization_applies_merge_before_private_injection() {
        let directory = TestDir::new("merge-runtime");
        let id = ProfileId::parse("daily").unwrap();
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        let source = "mode: rule\nmixed-port: 7890\nrules:\n  - MATCH,DIRECT\n";
        store
            .import(profile("daily", UpdatePolicy::Manual), source)
            .unwrap();
        store
            .set_merge_yaml(
                "rules:\n  - key: rules\n    op: prepend\n    items:\n      - DOMAIN,example.com,REJECT\n",
            )
            .unwrap();

        let path = store
            .materialize_runtime(&id, "127.0.0.1:45678".parse().unwrap(), "install-secret")
            .unwrap();
        let runtime = fs::read_to_string(path).unwrap();
        let value: serde_yaml::Value = serde_yaml::from_str(&runtime).unwrap();
        assert_eq!(
            value["rules"].as_sequence().unwrap()[0],
            "DOMAIN,example.com,REJECT"
        );
        assert_eq!(value["external-controller"], "127.0.0.1:45678");
        assert_eq!(value["secret"], "install-secret");
        // 源配置文件不被 merge 改动。
        assert_eq!(store.yaml(&id).unwrap(), source);
        // 合并预览不含私有注入字段。
        let merged = store.merged_yaml(&id).unwrap();
        assert!(merged.contains("DOMAIN,example.com,REJECT"));
        assert!(!merged.contains("install-secret"));
    }

    #[test]
    fn system_proxy_endpoint_skips_zero_mixed_port_and_uses_port() {
        let directory = TestDir::new("proxy-endpoint-zero-mixed");
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        let id = ProfileId::parse("zero-mixed").unwrap();
        store
            .import(
                Profile::new(
                    id.clone(),
                    "Zero Mixed",
                    ProfileSource::Local,
                    UpdatePolicy::Manual,
                    100,
                    None,
                )
                .unwrap(),
                "mixed-port: 0\nport: 7890\n",
            )
            .unwrap();
        assert_eq!(
            store.system_proxy_endpoint(&id).unwrap(),
            ProxyEndpoint::new("127.0.0.1", 7890).unwrap()
        );
    }

    #[test]
    fn system_proxy_endpoint_rejects_all_zero_ports_with_clear_error() {
        let directory = TestDir::new("proxy-endpoint-all-zero");
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        let id = ProfileId::parse("all-zero").unwrap();
        store
            .import(
                Profile::new(
                    id.clone(),
                    "All Zero",
                    ProfileSource::Local,
                    UpdatePolicy::Manual,
                    100,
                    None,
                )
                .unwrap(),
                "mixed-port: 0\nport: 0\n",
            )
            .unwrap();
        let error = store.system_proxy_endpoint(&id).unwrap_err();
        assert_eq!(error.code, ErrorCode::ValidationFailed);
        assert!(
            error.message.contains("non-zero"),
            "错误信息应提示 non-zero：{}",
            error.message
        );
    }

    #[test]
    fn system_proxy_socks_endpoint_skips_zero_mixed_port() {
        let directory = TestDir::new("socks-endpoint-zero-mixed");
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        let id = ProfileId::parse("zero-mixed-socks").unwrap();
        store
            .import(
                Profile::new(
                    id.clone(),
                    "Zero Mixed Socks",
                    ProfileSource::Local,
                    UpdatePolicy::Manual,
                    100,
                    None,
                )
                .unwrap(),
                "mixed-port: 0\nsocks-port: 7891\n",
            )
            .unwrap();
        assert_eq!(
            store.system_proxy_socks_endpoint(&id).unwrap(),
            Some(ProxyEndpoint::new("127.0.0.1", 7891).unwrap())
        );
    }

    #[test]
    fn system_proxy_endpoint_reads_merged_port() {
        let directory = TestDir::new("merge-proxy-endpoint");
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        let id = ProfileId::parse("daily").unwrap();
        store
            .import(profile("daily", UpdatePolicy::Manual), "mixed-port: 7890\n")
            .unwrap();
        store
            .set_merge_yaml(
                "rules:\n  - key: mixed-port\n    op: override\n    value: 7899\n",
            )
            .unwrap();
        assert_eq!(
            store.system_proxy_endpoint(&id).unwrap(),
            ProxyEndpoint::new("127.0.0.1", 7899).unwrap()
        );
    }
// 临时诊断：各种 YAML 变体下 system_proxy_endpoint 的解析结果
#[test]
fn endpoint_parsing_never_yields_zero_port_errors() {
    // 回归：任何合法/常见 YAML 写法都不应产生 "proxy endpoint must have a
    // valid host and non-zero port"（该错误只应在端口为 0 时出现，而 0 端口
    // 在 Mihomo 语义里表示禁用，应回落或给出明确配置错误）。
    let directory = TestDir::new("endpoint-parsing-matrix");
    let mut store = FileProfileStore::open(&directory.0).unwrap();
    let cases: [(&str, &str, Option<u16>); 6] = [
        ("mixed-normal", "mixed-port: 7890\nmode: rule\n", Some(7890)),
        ("mixed-zero-port-fallback", "mixed-port: 0\nport: 7890\n", Some(7890)),
        ("port-only", "port: 7890\n", Some(7890)),
        ("hex-port", "mixed-port: 0x1ED2\n", Some(7890)),
        ("mixed-zero-only", "mixed-port: 0\nmode: rule\n", None),
        ("missing-port", "mode: rule\n", None),
    ];
    for (id, yaml, expected) in cases {
        let pid = ProfileId::parse(id).unwrap();
        store
            .import(
                Profile::new(
                    pid.clone(),
                    id,
                    ProfileSource::Local,
                    UpdatePolicy::Manual,
                    100,
                    None,
                )
                .unwrap(),
                yaml,
            )
            .unwrap();
        match store.system_proxy_endpoint(&pid) {
            Ok(endpoint) => {
                assert_eq!(Some(endpoint.port), expected, "[{id}] 端口不符合预期");
            }
            Err(error) => {
                assert_eq!(error.code, ErrorCode::ValidationFailed, "[{id}] 错误码");
                assert!(
                    !error.message.contains("proxy endpoint must have a valid host"),
                    "[{id}] 不应出现 ProxyEndpoint 校验错误：{}",
                    error.message
                );
                assert_eq!(expected, None, "[{id}] 预期有端口却报错：{}", error.message);
            }
        }
    }
}

}
