use super::{from_clash::*, map_wrapper::MapWrapper};
use crate::entities::{common::*, outbound::*};

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
