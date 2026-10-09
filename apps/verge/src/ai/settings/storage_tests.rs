use super::*;
use crate::{
    config::{
        FileSettingsStore, export_portable_settings, export_settings_json, parse_portable_settings,
        parse_settings_import, tests::TestDir,
    },
    domain::{ApplicationSettings, CoreNetworkSettings, ThemePreference},
};
use std::{fs, os::unix::fs::PermissionsExt};

fn config() -> ProviderConfig {
    ProviderConfig {
        base_url: "https://example.test/v1".into(),
        model: "storage-test".into(),
        ..Default::default()
    }
}

fn application() -> ApplicationSettings {
    ApplicationSettings {
        language: "en".into(),
        theme: ThemePreference::Dark,
        log_limit: 1234,
        ..Default::default()
    }
}

#[test]
fn saves_preserve_general_settings_and_proxy_intent_with_or_without_main_file() {
    for existing in [false, true] {
        let directory = TestDir::new("ai-shared-main");
        let mut general = FileSettingsStore::open(&directory.0).unwrap();
        if existing {
            general.update(application()).unwrap();
            general.set_system_proxy_enabled(true).unwrap();
        }
        let expected = general.get().clone();
        let saved = save(
            &directory.0,
            &SavedConfig::default(),
            config(),
            Secret("shared-test-key".into()),
            false,
        )
        .unwrap();
        let loaded = load(&directory.0).unwrap();
        assert_eq!(loaded.config, config());
        assert_eq!(loaded.api_key, saved.api_key);
        let general = FileSettingsStore::open(&directory.0).unwrap();
        assert_eq!(general.get(), &expected);
        assert_eq!(general.system_proxy_enabled(), existing);
    }
}

#[test]
fn separate_ai_file_is_not_read_migrated_or_deleted() {
    let directory = TestDir::new("ai-current-only");
    let path = directory.0.join("ai.json");
    let bytes =
        serde_json::to_vec(&serde_json::json!({"config": config(), "api_key": "unused-test-key"}))
            .unwrap();
    fs::write(&path, &bytes).unwrap();
    let loaded = load(&directory.0).unwrap();
    assert_eq!(loaded.config, ProviderConfig::default());
    assert!(!loaded.has_key());
    assert!(!directory.0.join("settings.json").exists());
    save(&directory.0, &loaded, config(), Secret::default(), false).unwrap();
    assert!(!load(&directory.0).unwrap().has_key());
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[test]
fn maximum_pac_script_survives_ai_save_and_restart_after_json_escaping() {
    let directory = TestDir::new("ai-large-settings");
    let mut general = FileSettingsStore::open(&directory.0).unwrap();
    let mut settings = application();
    settings.system_proxy.pac_script = format!("/*{}*/", "\u{000b}".repeat(64 * 1024 - 4));
    general.update(settings.clone()).unwrap();
    let saved = save(
        &directory.0,
        &SavedConfig::default(),
        config(),
        Secret("large-settings-test-key".into()),
        false,
    )
    .unwrap();
    assert_eq!(
        FileSettingsStore::open(&directory.0).unwrap().get(),
        &settings
    );
    assert_eq!(load(&directory.0).unwrap().api_key, saved.api_key);
}

#[test]
fn invalid_ai_is_retained_without_blocking_general_settings() {
    let directory = TestDir::new("ai-invalid-storage");
    let mut general = FileSettingsStore::open(&directory.0).unwrap();
    general.update(application()).unwrap();
    let path = directory.0.join("settings.json");
    let before = fs::read(&path).unwrap();
    let invalid = serde_json::json!({"config": "invalid-test-key", "api_key": "test-key"});
    let mut json: serde_json::Value = serde_json::from_slice(&before).unwrap();
    json["ai"] = invalid.clone();
    fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    let error = load(&directory.0).err().unwrap();
    assert!(!format!("{error:?}").contains("test-key"));
    let mut general = FileSettingsStore::open(&directory.0).unwrap();
    assert!(!format!("{general:?}").contains("test-key"));
    let mut changed = application();
    changed.language = "zh-CN".into();
    general.update(changed).unwrap();
    general.set_system_proxy_enabled(true).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(json["ai"], invalid);
    assert_eq!(json["settings"]["language"], "zh-CN");
    assert_eq!(json["system_proxy_enabled"], true);
}

#[test]
fn export_and_import_keep_local_ai_and_proxy_intent_out_of_portable_settings() {
    let directory = TestDir::new("ai-export-import");
    let mut general = FileSettingsStore::open(&directory.0).unwrap();
    general.set_system_proxy_enabled(true).unwrap();
    let saved = save(
        &directory.0,
        &SavedConfig::default(),
        config(),
        Secret("local-test-key".into()),
        false,
    )
    .unwrap();
    let exports = [
        export_settings_json(&application()).unwrap(),
        export_portable_settings(&application(), &CoreNetworkSettings::default()).unwrap(),
    ];
    for bytes in exports {
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(json.get("ai").is_none());
        assert!(json.get("system_proxy_enabled").is_none());
        assert!(!String::from_utf8_lossy(&bytes).contains("local-test-key"));
        if json["version"] == 1 {
            assert_eq!(parse_settings_import(&bytes).unwrap(), application());
        }
        let (incoming, _) = parse_portable_settings(&bytes).unwrap();
        general.update(incoming).unwrap();
        assert_eq!(load(&directory.0).unwrap().api_key, saved.api_key);
        let reopened = FileSettingsStore::open(&directory.0).unwrap();
        assert!(reopened.system_proxy_enabled());
        assert_eq!(reopened.get(), &application());
    }
    let mut json: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.0.join("settings.json")).unwrap()).unwrap();
    json["version"] = "local-test-key".into();
    let error = parse_settings_import(&serde_json::to_vec(&json).unwrap()).unwrap_err();
    assert!(!format!("{error:?}").contains("local-test-key"));
}

#[test]
fn concurrent_ai_general_and_proxy_saves_preserve_all_fields() {
    use std::sync::Barrier;
    let directory = TestDir::new("ai-concurrent-save");
    let barrier = Barrier::new(4);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let mut general = FileSettingsStore::open(&directory.0).unwrap();
            for index in 0..16 {
                barrier.wait();
                let mut settings = application();
                settings.log_limit += index;
                general.update(settings).unwrap();
                barrier.wait();
            }
        });
        scope.spawn(|| {
            let mut general = FileSettingsStore::open(&directory.0).unwrap();
            for index in 0..16 {
                barrier.wait();
                general.set_system_proxy_enabled(index % 2 == 0).unwrap();
                barrier.wait();
            }
        });
        scope.spawn(|| {
            for index in 0..16 {
                barrier.wait();
                save(
                    &directory.0,
                    &SavedConfig::default(),
                    config(),
                    Secret(format!("round-{index}-test-key")),
                    false,
                )
                .unwrap();
                barrier.wait();
            }
        });
        for index in 0..16 {
            barrier.wait();
            barrier.wait();
            let general = FileSettingsStore::open(&directory.0).unwrap();
            assert_eq!(general.get().log_limit, application().log_limit + index);
            assert_eq!(general.get().theme, ThemePreference::Dark);
            assert_eq!(general.system_proxy_enabled(), index % 2 == 0);
            assert_eq!(
                load(&directory.0).unwrap().api_key.0,
                format!("round-{index}-test-key")
            );
        }
    });
    assert_eq!(
        fs::metadata(directory.0.join("settings.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}
