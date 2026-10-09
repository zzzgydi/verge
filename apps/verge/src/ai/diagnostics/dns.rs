use super::*;
use crate::domain::CoreNetworkSettings;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    patch: serde_json::Map<String, Value>,
}

impl Session {
    pub(super) fn preview_dns(&mut self, args: &str) -> Result<Value, AppError> {
        let request: Request = parse(args)?;
        let mut client = self.client()?;
        let runtime = proposals::RuntimeBaseline::new(
            client.mode()?,
            client.network_settings()?,
            client.proxy_groups()?,
        );
        if runtime.network.tun_enabled {
            return Err(error("Disable TUN before changing DNS"));
        }
        let config = self.config()?;
        let previous_runtime = proposals::digest(config.profiles.runtime_bytes()?);
        let mut preview = self.snapshot()?;
        let selected = preview
            .store
            .selected()
            .cloned()
            .ok_or_else(|| error("Select a profile first"))?;
        let settings = preview.store.network_settings()?;
        let next = patch_settings(&settings, &request.patch)?;
        let ids = preview
            .store
            .list()
            .iter()
            .map(|p| p.id.clone())
            .collect::<Vec<_>>();
        let mut originals = Vec::new();
        for id in &ids {
            self.check_cancel()?;
            let path = preview
                .store
                .materialize_runtime(id, config.controller, &config.secret)?;
            originals.push((
                id.clone(),
                crate::config::read_bounded(&path, 16 * 1024 * 1024)
                    .map_err(|_| error("Preview read failed"))?,
            ));
        }
        preview.store.set_network_override(Some(next.clone()))?;
        let mut changes = Vec::new();
        let mut candidate = Vec::new();
        for (id, before) in originals {
            self.check_cancel()?;
            let path = preview
                .store
                .materialize_runtime(&id, config.controller, &config.secret)?;
            let after = crate::config::read_bounded(&path, 16 * 1024 * 1024)
                .map_err(|_| error("Preview read failed"))?;
            let current_changes = dns_diff(&before, &after, &request.patch)?;
            validate(&config.binary, &path, &config.working_dir, &self.cancel)?;
            if id == selected {
                candidate = after;
            }
            let label = tools::label(id.as_str());
            changes.extend(current_changes.into_iter().map(|mut c| {
                c.path = format!("{label} / {}", c.path);
                c
            }));
        }
        if changes.is_empty() {
            return Err(error("DNS patch has no effect"));
        }
        changes.push(Change {path:"scope".into(),before:if settings.dns_override {"Global DNS override"} else {"Profile DNS"}.into(),after:format!("Global DNS override for {} profiles; other DNS fields inherited from selected profile", ids.len())});
        let baseline = preview.baseline.clone();
        let yaml = preview.store.merge_yaml()?;
        let digest = proposals::digest(&candidate);
        let evidence = self.record("preview_dns", json!({"changes":changes,"mihomo_validated":true,"validated_profiles":ids.len(),"applied":false}))?;
        self.propose(
            Action::Merge {
                network: Some(next),
                yaml,
                preview: Box::new(preview),
                selected,
                candidate,
                runtime_digest: digest,
                previous_runtime,
                runtime,
            },
            baseline,
            "dns",
            "DNS".into(),
            changes,
            &evidence,
        )
    }
}

fn patch_settings(
    settings: &CoreNetworkSettings,
    patch: &serde_json::Map<String, Value>,
) -> Result<CoreNetworkSettings, AppError> {
    if patch.is_empty() || patch.len() > 5 {
        return Err(error("Empty or oversized DNS patch"));
    }
    let mut dns: serde_yaml::Value =
        serde_yaml::from_str(&settings.dns_yaml).map_err(|_| error("Invalid DNS configuration"))?;
    let mapping = dns
        .as_mapping_mut()
        .ok_or_else(|| error("DNS must be a mapping"))?;
    for (key, value) in patch {
        let valid = match key.as_str() {
            "enable" | "ipv6" => value.is_boolean(),
            "enhanced-mode" => value
                .as_str()
                .is_some_and(|s| ["fake-ip", "redir-host"].contains(&s)),
            "nameserver" | "fallback" => value.as_array().is_some_and(|values| {
                !values.is_empty()
                    && values.len() <= 4
                    && values
                        .iter()
                        .all(|v| v.as_str().is_some_and(valid_resolver))
            }),
            _ => false,
        };
        if !valid {
            return Err(error("Unsupported DNS patch field or resolver"));
        }
        mapping.insert(
            serde_yaml::Value::String(key.clone()),
            serde_yaml::to_value(value).map_err(|_| error("Invalid DNS patch"))?,
        );
    }
    let mut next = settings.clone();
    next.dns_override = true;
    next.dns_yaml = serde_yaml::to_string(&dns).map_err(|_| error("Invalid DNS patch"))?;
    next.validate()?;
    Ok(next)
}
fn valid_resolver(value: &str) -> bool {
    if value.len() > 256
        || value.chars().any(char::is_whitespace)
        || value.chars().any(char::is_control)
    {
        return false;
    }
    if value.parse::<std::net::IpAddr>().is_ok()
        || value
            .parse::<std::net::SocketAddr>()
            .is_ok_and(|address| address.port() != 0)
    {
        return true;
    }
    reqwest::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == "/dns-query"
    })
}
fn dns_diff(
    before: &[u8],
    after: &[u8],
    patch: &serde_json::Map<String, Value>,
) -> Result<Vec<Change>, AppError> {
    let mut before: serde_yaml::Value =
        serde_yaml::from_slice(before).map_err(|_| error("Invalid DNS preview"))?;
    let mut after: serde_yaml::Value =
        serde_yaml::from_slice(after).map_err(|_| error("Invalid DNS preview"))?;
    let mut old_dns = before
        .as_mapping_mut()
        .ok_or_else(|| error("Invalid preview"))?
        .remove("dns")
        .unwrap_or(serde_yaml::Value::Mapping(Default::default()));
    let mut new_dns = after
        .as_mapping_mut()
        .ok_or_else(|| error("Invalid preview"))?
        .remove("dns")
        .unwrap_or(serde_yaml::Value::Mapping(Default::default()));
    // The settings writer materializes these Mihomo defaults. An explicit default
    // is equivalent to an omitted field, but a real port/flag change is rejected.
    for value in [&mut before, &mut after] {
        let mapping = value.as_mapping_mut().unwrap();
        for (key, default) in [
            ("allow-lan", serde_yaml::Value::Bool(false)),
            ("ipv6", serde_yaml::Value::Bool(false)),
            ("unified-delay", serde_yaml::Value::Bool(false)),
            ("log-level", serde_yaml::Value::String("info".into())),
            ("mixed-port", serde_yaml::Value::Number(0.into())),
        ] {
            mapping
                .entry(serde_yaml::Value::String(key.into()))
                .or_insert(default);
        }
    }
    if before != after {
        return Err(error("DNS override changes other settings; edit manually"));
    }
    let mut changes = Vec::new();
    for key in patch.keys() {
        let old = old_dns
            .as_mapping_mut()
            .ok_or_else(|| error("Invalid DNS mapping"))?
            .remove(key.as_str());
        let new = new_dns
            .as_mapping_mut()
            .ok_or_else(|| error("Invalid DNS mapping"))?
            .remove(key.as_str());
        let expected = serde_yaml::to_value(&patch[key]).map_err(|_| error("Invalid patch"))?;
        if new.as_ref() != Some(&expected) {
            return Err(error(
                "Scripts changed the requested DNS value; edit manually",
            ));
        }
        if old != new {
            let render = |v: Option<serde_yaml::Value>, safe: bool| -> String {
                let Some(v) = v else {
                    return "unset".into();
                };
                if safe {
                    return serde_yaml::to_string(&v)
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                }
                let mut m = serde_yaml::Mapping::new();
                m.insert(serde_yaml::Value::String(key.clone()), v);
                super::network::dns_summary(&serde_yaml::Value::Mapping(m))[key].to_string()
            };
            changes.push(Change {
                path: format!("dns.{key}"),
                before: render(old, false),
                after: render(new, true),
            });
        }
    }
    if old_dns != new_dns {
        return Err(error(
            "DNS override would replace other profile DNS fields; edit manually",
        ));
    }
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dns_patch_preserves_unrelated_settings_and_rejects_hidden_effects() {
        let settings = CoreNetworkSettings {
            dns_yaml: "enable: true\nnameserver: [1.1.1.1]\nfake-ip-filter: ['+.test']\n".into(),
            ..Default::default()
        };
        let patch =
            json!({"nameserver":["https://dns.example/dns-query"],"enhanced-mode":"redir-host"})
                .as_object()
                .unwrap()
                .clone();
        let next = patch_settings(&settings, &patch).unwrap();
        assert_eq!(next.mixed_port, settings.mixed_port);
        assert_eq!(next.external_controller, settings.external_controller);
        assert!(next.dns_yaml.contains("+.test"));
        for value in [
            json!({"listen":"0.0.0.0:53"}),
            json!({"nameserver":["https://user:password@dns.example/dns-query"]}),
            json!({"nameserver":["https://dns.example/dns-query?token=hidden"]}),
            json!({"nameserver":["https://dns.example/private"]}),
            json!({"enable":"true"}),
            json!({"fallback":[]}),
        ] {
            assert!(patch_settings(&settings, value.as_object().unwrap()).is_err());
        }
        let patch = json!({"enable":true}).as_object().unwrap().clone();
        let before = b"mode: rule\ndns: {enable: false, nameserver: [1.1.1.1]}\n";
        let after = b"mode: rule\ndns: {enable: true, nameserver: [1.1.1.1]}\n";
        assert_eq!(dns_diff(before, after, &patch).unwrap().len(), 1);
        assert!(dns_diff(before, b"mode: global\ndns: {enable: true}\n", &patch).is_err());
        assert!(
            dns_diff(
                before,
                b"mode: rule\ndns: {enable: true, nameserver: [8.8.8.8]}\n",
                &patch
            )
            .is_err()
        );
        assert!(dns_diff(before, before, &patch).is_err());
    }
}
