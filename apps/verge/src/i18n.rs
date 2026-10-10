//! JSON locales are validated and embedded as static tables by build.rs.

/// A known locale; additional JSON files are discovered at build time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lang {
    ZhCn,
    En,
    Other(&'static str),
}

impl Lang {
    pub fn from_code(code: &str) -> Self {
        match code {
            "zh-CN" => Self::ZhCn,
            "en" => Self::En,
            _ => LOCALES
                .iter()
                .find(|locale| locale.code == code)
                .map_or(Self::En, |locale| Self::Other(locale.code)),
        }
    }

    fn code(self) -> &'static str {
        match self {
            Self::ZhCn => "zh-CN",
            Self::En => "en",
            Self::Other(code) => code,
        }
    }
}

struct Locale {
    code: &'static str,
    name: &'static str,
    entries: &'static [(&'static str, &'static str)],
}

include!(concat!(env!("OUT_DIR"), "/locales.rs"));

pub fn supported_languages() -> impl Iterator<Item = (&'static str, &'static str)> {
    LOCALES.iter().map(|locale| (locale.code, locale.name))
}

pub fn supports_language(code: &str) -> bool {
    LOCALES.iter().any(|locale| locale.code == code)
}

fn lookup(locale: &Locale, key: &str) -> Option<&'static str> {
    locale
        .entries
        .binary_search_by_key(&key, |(entry, _)| entry)
        .ok()
        .map(|index| locale.entries[index].1)
}

fn resolve(locale: Option<&Locale>, english: &Locale, key: &'static str) -> &'static str {
    if let Some(value) = locale.and_then(|locale| lookup(locale, key)) {
        return value;
    }
    if let Some(value) = lookup(english, key) {
        return value;
    }
    debug_assert!(false, "missing English i18n key: {key}");
    key
}

/// Missing translations use English; missing English keys are programming errors.
pub fn tr(lang: Lang, key: &'static str) -> &'static str {
    let english = LOCALES.iter().find(|locale| locale.code == "en").unwrap();
    let localized = LOCALES.iter().find(|locale| locale.code == lang.code());
    resolve(localized, english, key)
}

/// Insert named fields in one pass so user-provided text is never re-interpolated.
pub fn fmt(lang: Lang, key: &'static str, fields: &[(&str, &str)]) -> String {
    let mut remaining = tr(lang, key);
    let mut result = String::new();
    while let Some((prefix, rest)) = remaining.split_once('{') {
        result.push_str(prefix);
        let Some((name, suffix)) = rest.split_once('}') else {
            result.push('{');
            remaining = rest;
            break;
        };
        if let Some((_, value)) = fields.iter().find(|(field, _)| *field == name) {
            result.push_str(value);
        } else {
            debug_assert!(false, "missing i18n field {name} for {key}");
            result.push('{');
            result.push_str(name);
            result.push('}');
        }
        remaining = suffix;
    }
    result.push_str(remaining);
    result
}

/// “标题 · 名称”式的对话框/Sheet 标题（两种语言结构一致）。
pub fn fmt_titled(lang: Lang, key: &'static str, name: &str) -> String {
    format!("{} · {name}", tr(lang, key))
}

/// 删除配置确认弹窗标题。
pub fn fmt_delete_profile_title(lang: Lang, name: &str) -> String {
    fmt(lang, "dialog.delete_profile_title", &[("name", name)])
}

/// YAML Sheet 保存确认弹窗标题。
pub fn fmt_save_yaml_title(lang: Lang, id: &str) -> String {
    fmt(lang, "dialog.save_yaml_title", &[("id", id)])
}

/// 应用更新确认弹窗标题。
pub fn fmt_update_app_title(lang: Lang, version: &str) -> String {
    fmt(lang, "dialog.update_app_title", &[("version", version)])
}

/// 恢复默认值确认弹窗标题（中英文语序不同）。
pub fn fmt_reset_scope_title(lang: Lang, scope_label: &str) -> String {
    fmt(lang, "dialog.reset_scope_title", &[("scope", scope_label)])
}

/// Sheet 加载失败的内联错误（冒号后换行接后端英文 message）。
pub fn fmt_load_failed(lang: Lang, key: &'static str, message: &str) -> String {
    fmt(
        lang,
        "dialog.load_failed",
        &[("label", tr(lang, key)), ("message", message)],
    )
}

/// 设置导入预览的字段差异行。
pub fn fmt_field_change(lang: Lang, field: &str, old: &str, new: &str) -> String {
    fmt(
        lang,
        "settings.field_change",
        &[("field", field), ("old", old), ("new", new)],
    )
}

/// 状态栏连接数。
pub fn fmt_statusbar_connections(lang: Lang, count: usize) -> String {
    fmt(
        lang,
        "statusbar.connections",
        &[("count", &count.to_string())],
    )
}

/// 连接页汇总行。
pub fn fmt_connections_summary(lang: Lang, count: usize, upload: &str, download: &str) -> String {
    fmt(
        lang,
        "connections.summary",
        &[
            ("count", &count.to_string()),
            ("upload", upload),
            ("download", download),
        ],
    )
}

/// 日志页级别过滤按钮文案。
pub fn fmt_logs_filter(lang: Lang, level_label: &str) -> String {
    fmt(lang, "logs.filter_label", &[("level", level_label)])
}

/// 特权 Helper 状态行。
pub fn fmt_helper_ready(lang: Lang, protocol_version: u32) -> String {
    fmt(
        lang,
        "settings.helper.ready",
        &[("version", &protocol_version.to_string())],
    )
}

pub fn fmt_helper_incompatible(lang: Lang, message: &str) -> String {
    fmt(
        lang,
        "settings.helper.incompatible",
        &[("message", message)],
    )
}

/// Mihomo 内核已安装版本行。
pub fn fmt_core_installed(lang: Lang, version: &str) -> String {
    fmt(lang, "settings.core.installed", &[("version", version)])
}

/// 应用更新最新版本行。
pub fn fmt_app_latest(lang: Lang, version: &str, update_available: bool) -> String {
    let key = if update_available {
        "settings.app.latest_available"
    } else {
        "settings.app.latest_current"
    };
    fmt(lang, key, &[("version", version)])
}

/// 应用更新待重启行。
pub fn fmt_pending_restart(lang: Lang, version: &str) -> String {
    fmt(
        lang,
        "settings.app.pending_version",
        &[("version", version)],
    )
}

/// 诊断/设置导出完成后的路径回显。
pub fn fmt_exported(lang: Lang, path: &str) -> String {
    fmt(lang, "export.completed", &[("path", path)])
}

/// 全局错误条标题。
pub fn fmt_alert_title(lang: Lang, code_debug: &str) -> String {
    format!("{} · {code_debug}", tr(lang, "alert.op_failed"))
}

/// 失败 toast 正文：后端英文 message + 本地化恢复指引。
pub fn fmt_toast_error(lang: Lang, message: &str, hint: Option<&str>) -> String {
    hint.map_or_else(
        || message.to_owned(),
        |hint| {
            fmt(
                lang,
                "toast.error_hint",
                &[("message", message), ("hint", hint)],
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locales_have_english_base_and_sorted_keys() {
        let english = LOCALES.iter().find(|locale| locale.code == "en").unwrap();
        for locale in LOCALES {
            assert!(locale.entries.windows(2).all(|pair| pair[0].0 < pair[1].0));
            for (key, value) in locale.entries {
                assert!(!value.is_empty(), "{key} has an empty translation");
                assert!(
                    lookup(english, key).is_some(),
                    "{key} has no English translation"
                );
            }
        }
    }

    #[test]
    fn missing_locale_entry_falls_back_to_english() {
        let english = Locale {
            code: "en",
            name: "English",
            entries: &[("a", "Hello"), ("b", "Goodbye")],
        };
        let partial = Locale {
            code: "test",
            name: "Test",
            entries: &[("a", "Hola")],
        };
        assert_eq!(resolve(Some(&partial), &english, "a"), "Hola");
        assert_eq!(resolve(Some(&partial), &english, "b"), "Goodbye");
        assert_eq!(resolve(None, &english, "a"), "Hello");
    }

    #[test]
    fn tr_looks_up_both_languages() {
        assert_eq!(tr(Lang::ZhCn, "home.title"), "概览");
        assert_eq!(tr(Lang::En, "home.title"), "Overview");
        assert_eq!(tr(Lang::ZhCn, "profiles.import"), "导入配置");
        assert_eq!(tr(Lang::En, "profiles.import"), "Import Profile");
    }

    #[test]
    fn lang_maps_settings_language_codes() {
        assert_eq!(Lang::from_code("zh-CN"), Lang::ZhCn);
        assert_eq!(Lang::from_code("en"), Lang::En);
        assert_eq!(Lang::from_code("anything-else"), Lang::En);
        assert!(supports_language("en"));
        assert!(supports_language("zh-CN"));
        let english = LOCALES.iter().find(|locale| locale.code == "en").unwrap();
        for locale in LOCALES {
            let lang = Lang::from_code(locale.code);
            assert_eq!(lang.code(), locale.code);
            for &(key, english_text) in english.entries {
                assert_eq!(tr(lang, key), lookup(locale, key).unwrap_or(english_text));
            }
        }
    }

    #[test]
    fn fmt_helpers_follow_language_word_order() {
        assert_eq!(
            fmt_reset_scope_title(Lang::ZhCn, "网络"),
            "恢复网络默认值？"
        );
        assert_eq!(
            fmt_reset_scope_title(Lang::En, "Network"),
            "Reset Network to defaults?"
        );
        assert_eq!(
            fmt_connections_summary(Lang::En, 3, "1.0 KB", "2.0 KB"),
            "3 connections · ↑ 1.0 KB · ↓ 2.0 KB"
        );
        assert_eq!(
            fmt_toast_error(Lang::En, "boom", Some("try again")),
            "boom. try again"
        );
        assert_eq!(fmt_toast_error(Lang::ZhCn, "boom", None), "boom");
        assert_eq!(
            fmt(Lang::En, "dialog.delete_profile_title", &[("name", "{id}")]),
            "Delete \"{id}\"?"
        );
    }
}
