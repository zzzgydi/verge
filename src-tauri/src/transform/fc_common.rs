use super::{from_clash::*, map_wrapper::MapWrapper};
use crate::entities::common::*;

impl FromClash for ServerOptions {
    fn from_clash(v: &serde_yaml::Value) -> anyhow::Result<Self> {
        let m = v.as_mapping().okr("invalid object")?;
        let m = MapWrapper(m);

        Ok(ServerOptions {
            server: m.get_str("server")?,
            server_port: m.get_u64("port")? as u16,
        })
    }
}

impl FromClash for OutboundRealityOptions {
    fn from_clash(v: &serde_yaml::Value) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        let m = v.as_mapping().okr("invalid object")?;
        let m = MapWrapper(m);

        Ok(OutboundRealityOptions {
            enabled: Some(true),
            public_key: m.or_str("public-key"),
            short_id: m.or_str("short-id"),
        })
    }
}

impl FromClash for OutboundTLSOptions {
    fn from_clash(v: &serde_yaml::Value) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        let vm = v.as_mapping().okr("invalid object")?;
        let m = MapWrapper(vm);

        let enabled = m.has("tls")
            || m.has("sni")
            || m.has("alpn")
            || m.has("skip-cert-verify")
            || m.has("servername")
            || m.has("reality-opts");

        if enabled {
            let reality = vm
                .get("reality-opts")
                .and_then(|v| OutboundRealityOptions::from_clash(v).ok());

            return Ok(OutboundTLSOptions {
                enabled: Some(enabled),
                disable_sni: Some(m.or_str("sni").is_none()),
                server_name: m.or_str("sni").or_else(|| m.or_str("servername")),
                insecure: m.or_bool("skip-cert-verify"),
                alpn: m.or_vec_str("alpn"),
                utls: None,
                certificate_path: None,
                certificate: None,
                reality,
                ..OutboundTLSOptions::default()
            });
        }

        Ok(OutboundTLSOptions::default())
    }
}
