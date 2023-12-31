use anyhow::{bail, Result};
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::entities::SubscriptionInfo;

#[derive(Default, Debug, Clone)]
pub struct DownloadOption {
    pub user_agent: Option<String>,
    pub proxy_scheme: Option<String>,
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct SubscriptionResult {
    pub data: String,
    pub name: Option<String>,
    pub info: Option<SubscriptionInfo>,
}

/// ## Remote type
/// create a new item from url
pub async fn download_subscription(
    url: &str,
    option: Option<DownloadOption>,
) -> Result<SubscriptionResult> {
    let opt_ref = option.as_ref();

    let mut builder = reqwest::ClientBuilder::new().use_rustls_tls().no_proxy();

    // 使用代理
    if let Some(proxy_scheme) = opt_ref.and_then(|o| o.proxy_scheme.clone()) {
        if let Ok(proxy) = reqwest::Proxy::http(&proxy_scheme) {
            builder = builder.proxy(proxy);
        }
        if let Ok(proxy) = reqwest::Proxy::https(&proxy_scheme) {
            builder = builder.proxy(proxy);
        }
        if let Ok(proxy) = reqwest::Proxy::all(&proxy_scheme) {
            builder = builder.proxy(proxy);
        }
    }

    if let Some(ua) = opt_ref.and_then(|o| o.user_agent.clone()) {
        builder = builder.user_agent(ua);
    }

    let resp = builder.build()?.get(url).send().await?;

    let status_code = resp.status();
    if !reqwest::StatusCode::is_success(&status_code) {
        bail!("failed to fetch remote profile with status {status_code}")
    }

    let header = resp.headers();

    // parse the Subscription UserInfo
    let extra = match header.get("Subscription-UserInfo") {
        Some(value) => {
            let sub_info = value.to_str().unwrap_or("");

            Some(SubscriptionInfo {
                upload: parse_str(sub_info, "upload=").unwrap_or(0),
                download: parse_str(sub_info, "download=").unwrap_or(0),
                total: parse_str(sub_info, "total=").unwrap_or(0),
                expire: parse_str(sub_info, "expire=").unwrap_or(0),
            })
        }
        None => None,
    };

    // parse the Content-Disposition
    let filename = match header.get("Content-Disposition") {
        Some(value) => {
            let filename = value.to_str().unwrap_or("");
            parse_str::<String>(filename, "filename=").or_else(|| {
                parse_str::<String>(filename, "filename*=").map(|v| {
                    match v.trim().split_once("''") {
                        Some((_, v)) => percent_decode_str(v)
                            .decode_utf8()
                            .unwrap_or(v.into())
                            .into(),
                        None => v.into(),
                    }
                })
            })
        }
        None => None,
    };

    // parse the profile-update-interval
    // let option = match header.get("profile-update-interval") {
    //     Some(value) => match value.to_str().unwrap_or("").parse::<u64>() {
    //         Ok(val) => Some(PrfOption {
    //             update_interval: Some(val * 60), // hour -> min
    //             ..PrfOption::default()
    //         }),
    //         Err(_) => None,
    //     },
    //     None => None,
    // };

    // let uid = help::get_uid("r");
    // let file = format!("{uid}.yaml");
    // let name = name.unwrap_or(filename.unwrap_or("Remote File".into()));
    let data = resp.text_with_charset("utf-8").await?;

    // process the charset "UTF-8 with BOM"
    let data = data.trim_start_matches('\u{feff}');

    Ok(SubscriptionResult {
        data: data.to_string(),
        name: filename,
        info: extra,
    })
}

/// parse the string
/// xxx=123123; => 123123
pub fn parse_str<T: FromStr>(target: &str, key: &str) -> Option<T> {
    target.find(key).and_then(|idx| {
        let idx = idx + key.len();
        let value = &target[idx..];

        match value.split(';').nth(0) {
            Some(value) => value.trim().parse(),
            None => value.trim().parse(),
        }
        .ok()
    })
}
