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
            "ss" => OutboundOptions::Shadowsocks(ShadowsocksOptions::from_clash(v)?),
            "vmess" => OutboundOptions::VMess(VMessOptions::from_clash(v)?),
            s @ _ => anyhow::bail!("invalid type `{s}`"),
        };

        Ok(Outbound { tag, option })
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
            network: m.fc_network(),
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
impl FromClash for ShadowsocksOptions {
    fn from_clash(v: &serde_yaml::Value) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        let m = v.as_mapping().okr("invalid object")?;
        let m = MapWrapper(m);

        let plugin_opts = m.or_map("plugin-opts").and_then(|opts| {
            Some(format!(
                "obfs={};obfs-host={}",
                opts.or_str("mode")?,
                opts.or_str("host")?
            ))
        });

        Ok(ShadowsocksOptions {
            dialer_options: DialerOptions::default(),
            server_options: ServerOptions::from_clash(&v)?,
            method: m.get_str("cipher")?,
            password: m.get_str("password")?,
            plugin: m.or_str("plugin"), //  obfs-local, v2ray-plugin
            plugin_opts,
            network: m.fc_network(),
            // TODO multiplex need to be impl
            udp_over_tcp: None,
            multiplex: None,
        })
    }
}
// 为 VMessOptions 实现 FromClash trait
impl FromClash for VMessOptions {
    fn from_clash(v: &serde_yaml::Value) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        let m = v
            .as_mapping()
            .ok_or_else(|| anyhow::Error::msg("invalid object"))?;
        let m = MapWrapper(m);

        Ok(VMessOptions {
            dialer_options: DialerOptions::default(),
            server_options: ServerOptions::from_clash(&v)?,
            uuid: m.get_str("uuid")?,
            security: m.get_str("cipher")?,
            alter_id: m.or_i32("alterId"),
            network: m.fc_network(),
            global_padding: None,
            authenticated_length: None,
            tls: OutboundTLSOptions::from_clash(&v).ok(),
            transport: V2RayTransportOptions::from_clash(v).ok(),
            packet_encoding: None,
            multiplex: None,
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
