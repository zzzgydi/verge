//! Narrow proxy preference patches. The saved PAC script stays local and immutable.
use super::*;
use crate::domain::{DEFAULT_PROXY_BYPASS, SystemProxySettings};
use crate::platform::{MacSystemProxy, SystemProxyRunner};

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Patch {
    #[serde(default)]
    add_bypass: Vec<String>,
    #[serde(default)]
    remove_bypass: Vec<String>,
    #[serde(default)]
    reset_bypass: bool,
    use_default_bypass: Option<bool>,
    pac_mode: Option<bool>,
    guard_enabled: Option<bool>,
    guard_interval_secs: Option<u64>,
}

impl Session {
    pub(super) fn proxy_settings_tool(
        &mut self,
        name: &str,
        args: &str,
    ) -> Result<Value, AppError> {
        if name == "system_proxy_settings" {
            let _: Empty = parse(args)?;
            let settings = self
                .context
                .network
                .proxy_settings
                .as_ref()
                .ok_or_else(|| error("Proxy preferences unavailable"))?;
            let data = json!({"pac_mode":settings.pac_mode,"guard_enabled":settings.guard_enabled,"guard_interval_secs":settings.guard_interval_secs,
                "use_default_bypass":settings.use_default_bypass,"custom_bypass":display_entries(&settings.bypass),"effective_bypass":display_entries(&settings.effective_bypass()),
                "owned":self.context.network.proxy_owned,"startup_enabled":self.context.network.proxy_saved_enabled,"captured_at":self.context.network.captured_at,
                "scope":"saved preferences; bypass applies to manual HTTP/HTTPS/SOCKS proxy, PAC uses its saved script; no TUN rule changes","pac_script_omitted":true});
            return Ok(json!(self.record(name, data)?));
        }
        if self.context.network.dev_mode {
            return Err(error("System proxy settings are unavailable in Dev"));
        }
        let patch: Patch = parse(args)?;
        let previous = self
            .context
            .network
            .proxy_settings
            .clone()
            .ok_or_else(|| error("Proxy preferences unavailable"))?;
        let settings = apply_patch(&previous, patch)?;
        let mut changes = diff(&previous, &settings);
        if changes.is_empty() {
            return Err(error("Proxy preferences already match"));
        }
        let state = if self.context.network.proxy_owned {
            Some(
                MacSystemProxy::new(SystemProxyRunner, self.context.recovery_path.clone())
                    .state(&self.context.services)?,
            )
        } else {
            None
        };
        changes.push(Change {
            path: "scope".into(),
            before: String::new(),
            after: if state.is_some() {
                "Apply to the currently managed system proxy and save for future use"
            } else {
                "Save preferences for the next enable; leave system proxy disabled"
            }
            .into(),
        });
        let baseline = self.config()?.profiles.preview_digest()?;
        let evidence = self.record(name,json!({"changes":changes,"applied":false,"pac_script_omitted":true,"bypass_scope":"manual proxy only; PAC routing uses the saved script; TUN rules unchanged"}))?;
        self.propose(
            Action::ProxySettings {
                previous: Box::new(previous),
                settings: Box::new(settings),
                state,
                saved_enabled: self.context.network.proxy_saved_enabled,
            },
            baseline,
            "proxy_settings",
            "System proxy settings".into(),
            changes,
            &evidence,
        )
    }
}

fn validate_entry(entry: &str) -> Result<(), AppError> {
    if entry.len() > 253 {
        return Err(error("Bypass entry exceeds 253 bytes"));
    }
    SystemProxySettings {
        bypass: vec![entry.into()],
        ..Default::default()
    }
    .validate()
    .map_err(|_| error("Use a domain, wildcard, IP, CIDR or <local> bypass entry"))
}
fn apply_patch(
    previous: &SystemProxySettings,
    patch: Patch,
) -> Result<SystemProxySettings, AppError> {
    if patch.add_bypass.len() + patch.remove_bypass.len() > 32 {
        return Err(error("At most 32 bypass changes per proposal"));
    }
    if patch.reset_bypass
        && (!patch.add_bypass.is_empty()
            || !patch.remove_bypass.is_empty()
            || patch.use_default_bypass.is_some())
    {
        return Err(error("Reset bypass cannot be combined with bypass edits"));
    }
    for entry in patch.add_bypass.iter().chain(&patch.remove_bypass) {
        validate_entry(entry)?;
    }
    if patch.add_bypass.iter().any(|add| {
        patch
            .remove_bypass
            .iter()
            .any(|remove| add.eq_ignore_ascii_case(remove))
    }) {
        return Err(error("Cannot add and remove the same bypass entry"));
    }
    let mut next = previous.clone();
    if patch.reset_bypass {
        next.bypass.clear();
        next.use_default_bypass = true;
    }
    if let Some(value) = patch.use_default_bypass {
        next.use_default_bypass = value;
    }
    if !patch.remove_bypass.is_empty() {
        let effective = next.effective_bypass();
        if patch.remove_bypass.iter().any(|entry| {
            !effective
                .iter()
                .any(|current| current.eq_ignore_ascii_case(entry))
        }) {
            return Err(error("Bypass entry to remove was not found"));
        }
        let removes_default = patch.remove_bypass.iter().any(|entry| {
            DEFAULT_PROXY_BYPASS
                .iter()
                .any(|default| default.eq_ignore_ascii_case(entry))
        }) && (next.use_default_bypass || next.bypass.is_empty());
        if removes_default {
            if patch.use_default_bypass == Some(true) {
                return Err(error(
                    "Cannot remove a default entry while requesting default bypass",
                ));
            }
            // Materialize the remaining defaults so removing one does not remove all others.
            next.bypass = effective;
            next.use_default_bypass = false;
        }
        next.bypass.retain(|entry| {
            !patch
                .remove_bypass
                .iter()
                .any(|remove| entry.eq_ignore_ascii_case(remove))
        });
    }
    for entry in patch.add_bypass {
        if !next
            .effective_bypass()
            .iter()
            .any(|current| current.eq_ignore_ascii_case(&entry))
        {
            next.bypass.push(entry);
        }
    }
    // The application intentionally falls back to defaults for an empty custom list.
    if patch.remove_bypass.iter().any(|entry| {
        next.effective_bypass()
            .iter()
            .any(|current| current.eq_ignore_ascii_case(entry))
    }) {
        return Err(error(
            "An empty bypass list restores defaults; keep at least one bypass entry",
        ));
    }
    if let Some(value) = patch.pac_mode {
        next.pac_mode = value;
    }
    if let Some(value) = patch.guard_enabled {
        next.guard_enabled = value;
    }
    if let Some(value) = patch.guard_interval_secs {
        next.guard_interval_secs = value;
    }
    next.validate()
        .map_err(|_| error("Invalid proxy preferences"))?;
    Ok(next)
}

fn display_entries(entries: &[String]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| {
            if validate_entry(entry).is_ok() && tools::label(entry) != "[redacted]" {
                entry.clone()
            } else {
                "[redacted]".into()
            }
        })
        .collect()
}
fn diff(previous: &SystemProxySettings, next: &SystemProxySettings) -> Vec<Change> {
    let mut changes = Vec::new();
    for (path, before, after) in [
        (
            "proxy.bypass",
            display_entries(&previous.bypass).join("\n"),
            display_entries(&next.bypass).join("\n"),
        ),
        (
            "proxy.effective_bypass",
            display_entries(&previous.effective_bypass()).join("\n"),
            display_entries(&next.effective_bypass()).join("\n"),
        ),
        (
            "proxy.use_default_bypass",
            previous.use_default_bypass.to_string(),
            next.use_default_bypass.to_string(),
        ),
        (
            "proxy.pac_mode",
            previous.pac_mode.to_string(),
            next.pac_mode.to_string(),
        ),
        (
            "proxy.guard_enabled",
            previous.guard_enabled.to_string(),
            next.guard_enabled.to_string(),
        ),
        (
            "proxy.guard_interval_secs",
            previous.guard_interval_secs.to_string(),
            next.guard_interval_secs.to_string(),
        ),
    ] {
        if before != after {
            changes.push(Change {
                path: path.into(),
                before,
                after,
            });
        }
    }
    changes
}

pub(super) fn schemas() -> Vec<Value> {
    vec![
        json!({"type":"function","function":{"name":"system_proxy_settings","description":"Read captured saved bypass, PAC mode and proxy guard preferences. PAC script is omitted; bypass only affects manual proxy, not PAC or TUN rules.","parameters":{"type":"object","properties":{},"additionalProperties":false}}}),
        json!({"type":"function","function":{"name":"preview_system_proxy_settings","description":"Preview incremental bypass additions/removals, reset to defaults, PAC mode or guard preferences. At most 32 entries per proposal. Removing a default retains the other defaults as custom entries. Reset excludes other bypass edits. Empty bypass falls back to defaults. Requires separate user confirmation. Applies now if proxy is owned; otherwise only saves. Does not enable proxy, edit host/PAC script, disable validation or change TUN routing.","parameters":{"type":"object","properties":{"add_bypass":{"type":"array","items":{"type":"string"},"maxItems":32},"remove_bypass":{"type":"array","items":{"type":"string"},"maxItems":32},"reset_bypass":{"type":"boolean"},"use_default_bypass":{"type":"boolean"},"pac_mode":{"type":"boolean"},"guard_enabled":{"type":"boolean"},"guard_interval_secs":{"type":"integer","minimum":1,"maximum":86400}},"additionalProperties":false}}}),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    fn patch(previous: &SystemProxySettings, json: &str) -> Result<SystemProxySettings, AppError> {
        apply_patch(previous, parse(json)?)
    }
    #[test]
    fn bypass_edits_preserve_unmentioned_values_and_defaults() {
        let original = SystemProxySettings {
            host: "localhost".into(),
            pac_script: "function FindProxyForURL(){return 'DIRECT';}".into(),
            ..Default::default()
        };
        let added=patch(&original,r#"{"add_bypass":["*.example.com","10.20.0.0/16","*.example.com"],"guard_enabled":true,"guard_interval_secs":15}"#).unwrap();
        assert_eq!(added.bypass, vec!["*.example.com", "10.20.0.0/16"]);
        assert_eq!(added.host, original.host);
        assert_eq!(added.pac_script, original.pac_script);
        assert!(added.effective_bypass().contains(&"localhost".into()));
        let removed = patch(&added, r#"{"remove_bypass":["localhost","*.EXAMPLE.com"]}"#).unwrap();
        assert!(!removed.use_default_bypass);
        assert!(!removed.effective_bypass().contains(&"localhost".into()));
        assert!(!removed.effective_bypass().contains(&"*.example.com".into()));
        assert!(removed.effective_bypass().contains(&"127.0.0.1".into()));
        assert!(removed.effective_bypass().contains(&"10.20.0.0/16".into()));
        let reset = patch(&removed, r#"{"reset_bypass":true,"pac_mode":true}"#).unwrap();
        assert_eq!(reset.effective_bypass(), original.effective_bypass());
        assert!(reset.bypass.is_empty());
        assert!(reset.pac_mode);
        assert!(reset.guard_enabled);
        assert_eq!(reset.guard_interval_secs, 15);
    }
    #[test]
    fn malformed_and_ambiguous_patches_are_rejected_even_if_validation_is_disabled() {
        let original = SystemProxySettings {
            validate_bypass: false,
            ..Default::default()
        };
        for request in [
            r#"{"add_bypass":["https://site.test/path"]}"#,
            r#"{"add_bypass":["--help"]}"#,
            r#"{"add_bypass":["a;rm"]}"#,
            r#"{"add_bypass":["10.0.0.1/99"]}"#,
            r#"{"add_bypass":["a\nb"]}"#,
            r#"{"add_bypass":["site.test"],"remove_bypass":["SITE.test"]}"#,
            r#"{"reset_bypass":true,"add_bypass":["site.test"]}"#,
            r#"{"remove_bypass":["localhost"],"use_default_bypass":true}"#,
            r#"{"guard_interval_secs":0}"#,
            r#"{"guard_interval_secs":86401}"#,
            r#"{"remove_bypass":["missing.test"]}"#,
            r#"{"host":"remote.test"}"#,
            r#"{"pac_script":"arbitrary"}"#,
            r#"{"validate_bypass":false}"#,
            r#"{"approval":true}"#,
        ] {
            assert!(patch(&original, request).is_err(), "{request}");
        }
        let request = json!({"remove_bypass":DEFAULT_PROXY_BYPASS}).to_string();
        assert!(patch(&original, &request).is_err());
    }
    #[test]
    fn changed_entries_and_pac_script_never_leak_unsafe_settings() {
        let original = SystemProxySettings {
            bypass: vec!["https://private.test/?token=hidden".into()],
            validate_bypass: false,
            pac_script: "private-script-secret".into(),
            ..Default::default()
        };
        let next = patch(&original, r#"{"reset_bypass":true}"#).unwrap();
        let rendered = serde_json::to_string(&diff(&original, &next)).unwrap();
        for secret in ["private.test", "token=hidden", "private-script-secret"] {
            assert!(!rendered.contains(secret));
        }
        assert_eq!(next.pac_script, original.pac_script);
    }
}
