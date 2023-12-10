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
            "trojan" => OutboundOptions::Trojan(TrojanOptions::from_clash(v)?),
            "wireguard" => OutboundOptions::WireGuard(WireGuardOptions::from_clash(v)?),
            "hysteria" => OutboundOptions::Hysteria(HysteriaOptions::from_clash(v)?),
            "vless" => OutboundOptions::VLESS(VLESSOptions::from_clash(v)?),
            "hysteria2" => OutboundOptions::Hysteria2(Hysteria2Options::from_clash(v)?),
            "tuic" => OutboundOptions::TUIC(TUICOptions::from_clash(v)?),
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

impl FromClash for VMessOptions {
    fn from_clash(v: &serde_yaml::Value) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        let m = v.as_mapping().okr("invalid object")?;
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

impl FromClash for TrojanOptions {
    fn from_clash(v: &serde_yaml::Value) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        let m = v.as_mapping().okr("invalid object")?;
        let m = MapWrapper(m);

        Ok(TrojanOptions {
            dialer_options: DialerOptions::default(),
            server_options: ServerOptions::from_clash(&v)?,
            password: m.get_str("password")?,
            network: m.fc_network(),
            tls: OutboundTLSOptions::from_clash(&v).ok(),
            transport: V2RayTransportOptions::from_clash(v).ok(),
            multiplex: None,
        })
    }
}

impl FromClash for WireGuardOptions {
    fn from_clash(v: &serde_yaml::Value) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        let m = v.as_mapping().okr("invalid object")?;
        let m = MapWrapper(m);

        Ok(WireGuardOptions {
            dialer_options: DialerOptions::default(),
            server_options: ServerOptions::from_clash(&v)?,
            system_interface: None,
            interface_name: None,
            gso: None,
            gso_max_size: None,
            local_address: m.or_vec_str("allowed-ips").unwrap_or(vec![]),
            peers: None,
            peer_public_key: m.get_str("public-key")?,
            private_key: m.get_str("private-key")?,
            pre_shared_key: m.or_str("pre-shared-key"),
            reserved: m.or_vec_u8("reserved"),
            workers: None,
            mtu: m.or_u32("mtu"),
            network: m.fc_network(),
        })
    }
}

impl FromClash for HysteriaOptions {
    fn from_clash(v: &serde_yaml::Value) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        let m = v.as_mapping().okr("invalid object")?;
        let m = MapWrapper(m);

        let (up_mbps, up) = match m.get_i32("up") {
            Ok(up) => (Some(up), None),
            Err(_) => (None, m.or_str("up")),
        };

        let (down_mbps, down) = match m.get_i32("down") {
            Ok(down) => (Some(down), None),
            Err(_) => (None, m.or_str("down")),
        };

        Ok(HysteriaOptions {
            dialer_options: DialerOptions::default(),
            server_options: ServerOptions::from_clash(&v)?,
            up,
            up_mbps,
            down,
            down_mbps,
            obfs: m.or_str("obfs"),
            auth: None,
            auth_str: m.or_str("auth_str").or_else(|| m.or_str("auth-str")),
            recv_window_conn: m
                .or_u64("recv_window_conn")
                .or_else(|| m.or_u64("recv-window-conn")),
            recv_window: m.or_u64("recv_window").or_else(|| m.or_u64("recv-window")),
            disable_mtu_discovery: m.or_bool("disable_mtu_discovery"),
            network: m.fc_network(),
            tls: OutboundTLSOptions::from_clash(&v).ok(),
        })
    }
}

impl FromClash for VLESSOptions {
    fn from_clash(v: &serde_yaml::Value) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        let m = v.as_mapping().okr("invalid object")?;
        let m = MapWrapper(m);

        Ok(VLESSOptions {
            dialer_options: DialerOptions::default(),
            server_options: ServerOptions::from_clash(&v)?,
            uuid: m.get_str("uuid")?,
            flow: m.or_str("flow"),
            network: m.fc_network(),
            tls: OutboundTLSOptions::from_clash(&v).ok(),
            multiplex: None, // 根据实际情况进行调整
            transport: V2RayTransportOptions::from_clash(v).ok(),
            packet_encoding: m.or_str("packet_encoding"),
        })
    }
}

impl FromClash for Hysteria2Options {
    fn from_clash(v: &serde_yaml::Value) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        let m = v.as_mapping().okr("invalid object")?;
        let m = MapWrapper(m);

        fn parse_speed_mbps(map: &MapWrapper, key: &str) -> Option<i32> {
            map.or_i32(key).or_else(|| {
                map.or_str(key)
                    .and_then(|v| v.trim_end_matches(" Mbps").parse::<i32>().ok())
            })
        }

        let obfs = match (m.or_str("obfs"), m.or_str("obfs-password")) {
            (None, None) => None,
            t @ _ => Some(Hysteria2Obfs {
                type_field: t.0,
                password: t.1,
            }),
        };

        Ok(Hysteria2Options {
            dialer_options: DialerOptions::default(),
            server_options: ServerOptions::from_clash(&v)?,
            up_mbps: parse_speed_mbps(&m, "up"),
            down_mbps: parse_speed_mbps(&m, "down"),
            obfs,
            password: m.get_str("password")?,
            network: m.fc_network(),
            tls: OutboundTLSOptions::from_clash(&v).ok(),
            brutal_debug: false, // 根据实际情况进行调整
        })
    }
}

impl FromClash for TUICOptions {
    fn from_clash(v: &serde_yaml::Value) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        let m = v.as_mapping().okr("invalid object")?;
        let m = MapWrapper(m);

        Ok(TUICOptions {
            dialer_options: DialerOptions::default(),
            server_options: ServerOptions::from_clash(&v)?,
            uuid: m.or_str("uuid"),
            password: m.or_str("password"),
            congestion_control: m.or_str("congestion-controller"),
            udp_relay_mode: m.or_str("udp-relay-mode"),
            udp_over_stream: m.or_bool("udp-over-stream").unwrap_or_default(),
            zero_rtt_handshake: m.or_bool("zero-rtt-handshake").unwrap_or_default(),
            heartbeat: m.or_str("heartbeat-interval").map(|s| format!("{}ms", s)),
            network: m.fc_network(),
            tls: OutboundTLSOptions::from_clash(&v).ok(),
        })
    }
}

#[test]
fn test_c2sb() {
    let yaml = include_str!("../../tests/config2.yaml");

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
