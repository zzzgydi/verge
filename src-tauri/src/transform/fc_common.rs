use super::{from_clash::*, map_wrapper::MapWrapper};
use crate::entities::singbox::*;

impl FromClash for ServerOptions {
    fn from_clash(v: &serde_yaml::Value) -> anyhow::Result<Self> {
        let m = v.as_mapping().okr("invalid object")?;
        let m = MapWrapper(m);

        Ok(ServerOptions {
            server: m.get_str("server")?,
            server_port: m.get_u16("port")?,
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

// 为 V2RayTransportOptions 实现 FromClash trait
impl FromClash for V2RayTransportOptions {
    fn from_clash(v: &serde_yaml::Value) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        let m = v.as_mapping().okr("invalid object")?;
        let m = MapWrapper(m);

        match m.or_str("network").as_deref() {
            Some("ws") => Ok(V2RayTransportOptions::Websocket(
                V2RayWebsocketOptions::from_clash(v)?,
            )),
            Some("http") => Ok(V2RayTransportOptions::HTTP(V2RayHTTPOptions::from_clash(
                v,
            )?)),
            Some("grpc") => Ok(V2RayTransportOptions::GRPC(V2RayGRPCOptions::from_clash(
                v,
            )?)),
            // TODO h2
            n @ _ => Err(anyhow::Error::msg(format!(
                "unsupported network type `{}`",
                n.unwrap_or_default()
            ))),
        }
    }
}

impl FromClash for V2RayWebsocketOptions {
    fn from_clash(v: &serde_yaml::Value) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        let m = v.as_mapping().okr("invalid object")?;
        let m = MapWrapper(m);

        let ws = m.get_map("ws-opts")?;

        Ok(V2RayWebsocketOptions {
            path: ws.or_str("path"),
            headers: ws.fc_headers(),
            max_early_data: ws.or_u32("max-early-data"),
            early_data_header_name: ws.or_str("early-data-header-name"),
        })
    }
}

impl FromClash for V2RayHTTPOptions {
    fn from_clash(v: &serde_yaml::Value) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        let m = v.as_mapping().okr("invalid object")?;
        let m = MapWrapper(m);

        let h = m.get_map("http-opts")?;

        Ok(V2RayHTTPOptions {
            host: h.or_vec_str("host"),
            path: h.or_str("path"),
            method: h.or_str("method"),
            headers: h.fc_headers(),
            idle_timeout: None,
            ping_timeout: None,
        })
    }
}

impl FromClash for V2RayGRPCOptions {
    fn from_clash(v: &serde_yaml::Value) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        let m = v.as_mapping().okr("invalid object")?;
        let m = MapWrapper(m);

        let g = m.get_map("grpc-opts")?;

        Ok(V2RayGRPCOptions {
            service_name: g.or_str("grpc-service-name"),
            idle_timeout: None,
            ping_timeout: None,
            permit_without_stream: None,
        })
    }
}
