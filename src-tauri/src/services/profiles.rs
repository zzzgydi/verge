use crate::entities::Profile;
use anyhow::Result;
use once_cell::sync::OnceCell;

use super::storage::Storage;

pub struct Profiles {}

impl Profiles {
    pub fn global() -> &'static Profiles {
        static SERVICE: OnceCell<Profiles> = OnceCell::new();

        SERVICE.get_or_init(|| Profiles {})
    }

    pub fn create_profile(&self, profile: Profile) -> Result<()> {
        let db: &Storage = Storage::global();

        let mut profiles = db.get::<Vec<Profile>>("profiles")?.unwrap_or(vec![]);
        profiles.push(profile);

        db.set("profiles", &profiles)?;
        Ok(())
    }
}
