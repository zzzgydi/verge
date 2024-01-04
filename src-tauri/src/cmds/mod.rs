mod dto;
use self::dto::*;
use crate::entities::Profile;
use crate::services::storage::Storage;
use crate::utils::{download_subscription, get_uid, DownloadOption};
use crate::wrap_err;

#[tauri::command]
pub fn list_profiles() -> CmdResult<Vec<Profile>> {
    let db = Storage::global();
    let profiles = wrap_err!(db.get::<Vec<Profile>>("profiles"))?.unwrap_or(vec![]);
    Ok(profiles)
}

#[tauri::command]
pub async fn import_profile(req: ImportProfileReq) -> CmdResult {
    log::info!("import profile: {:?}", req);

    let option = DownloadOption {
        user_agent: Some("clash-verge/1.4.0".to_string()),
        proxy_scheme: None,
    };
    let result = wrap_err!(download_subscription(&req.url, Some(option)).await)?;

    log::info!("download subscription: {:?}", result);

    let now = chrono::Local::now().timestamp() as usize;
    let name = result
        .name
        .unwrap_or("Profile".to_string())
        .trim()
        .to_string();

    let _name = name.replace(" ", "_");
    let file_name = format!("{now}-{_name}.yaml");
    let profile = Profile {
        id: get_uid(),
        name,
        file_name,
        format: "clash-yaml".to_string(),
        url: Some(req.url.to_string()),
        updated_at: Some(now),
        subsciption: result.info,
        ..Profile::default()
    };

    let db = Storage::global();
    let mut profiles = wrap_err!(db.get::<Vec<Profile>>("profiles"))?.unwrap_or(vec![]);
    profiles.push(profile);
    wrap_err!(db.set("profiles", &profiles))?;
    Ok(())
}
