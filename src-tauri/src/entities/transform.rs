use super::{common::*, map_wrapper::MapWrapper, outbound::*, OptionToResult};

pub trait FromClash {
    fn from_clash(v: &serde_yaml::Value) -> anyhow::Result<Self>
    where
        Self: Sized;
}

impl FromClash for Outbound {
    fn from_clash(v: &serde_yaml::Value) -> anyhow::Result<Self> {
        let m = v.as_mapping().okr("invalid object")?;
        let m = MapWrapper(m);

        let r#type = m.get_str("type")?;
        let tag = m.get_str("name")?;

        let option = match r#type.as_str() {
            "socks" => OutboundOptions::Socks(SocksOptions::from_clash(v)?),
            "http" => OutboundOptions::HTTP(HTTPOptions::from_clash(v)?),
            s @ _ => anyhow::bail!("invalid type `{s}`"),
        };

        Ok(Outbound {
            r#type,
            tag,
            option,
        })
    }
}

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

impl FromClash for SocksOptions {
    fn from_clash(v: &serde_yaml::Value) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        let m = v.as_mapping().okr("invalid object")?;
        let m = MapWrapper(m);

        Ok(SocksOptions {
            dialer_options: DialerOptions::default(),
            server_options: ServerOptions::from_clash(&v)?,
            version: Some("5".to_string()),
            username: m.or_str("username"),
            password: m.or_str("password"),
            network: match m.or_bool("udp") {
                Some(true) => None,
                _ => Some("tcp".to_string()),
            },
            udp_over_tcp: None,
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

impl FromClash for HTTPOptions {
    fn from_clash(v: &serde_yaml::Value) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        let m = v.as_mapping().okr("invalid object")?;
        let m = MapWrapper(m);

        Ok(HTTPOptions {
            dialer_options: DialerOptions::default(),
            server_options: ServerOptions::from_clash(&v)?,
            username: m.or_str("username"),
            password: m.or_str("password"),
            tls: OutboundTLSOptions::from_clash(&v).ok(),
            path: None,
            headers: None,
        })
    }
}

#[test]
fn test_c2sb() {
    let yaml = include_str!("../../tests/config1.yaml");

    let v: serde_yaml::Mapping = serde_yaml::from_str(yaml).unwrap();
    let proxies = v["proxies"].as_sequence().unwrap();

    for proxy in proxies {
        // println!("{:?} => ", proxy["name"].as_str().unwrap());
        match Outbound::from_clash(&proxy) {
            Ok(o) => println!("{:?}", o),
            Err(e) => println!("Err: {:?}", e),
        }
    }
}
