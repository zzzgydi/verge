use gpui_kit::{AssetSource, SharedString};
use std::borrow::Cow;

gpui_kit::assets::icon_assets!(ExtraIcons, [RefreshCw]);

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        if path == "branding/logo.svg" {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../../../assets/branding/logo.svg"
            ))));
        }
        if let Some(icon) = ExtraIcons.load(path)? {
            return Ok(Some(icon));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut assets = gpui_kit::assets::Assets.list(path)?;
        if "branding/logo.svg".starts_with(path) {
            assets.push("branding/logo.svg".into());
        }
        Ok(assets)
    }
}
