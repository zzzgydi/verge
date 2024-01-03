use crate::utils::dirs;
use anyhow::Result;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use serde_json::{from_slice, to_vec};
use sled::Db;

#[derive(Debug, Clone)]
pub struct Storage {
    db: Db,
}

impl Storage {
    pub fn global() -> &'static Storage {
        static SERVICE: OnceCell<Storage> = OnceCell::new();

        SERVICE.get_or_init(|| {
            let app_dir = dirs::app_dir().expect("failed to get app dir");
            let db_path = app_dir.join("verge.db");

            Storage {
                db: sled::open(db_path).expect("failed to open verge db"),
            }
        })
    }

    pub fn get<T: for<'de> Deserialize<'de>>(&self, key: &str) -> Result<Option<T>> {
        match self.db.get(key)? {
            Some(ivec) => {
                let deserialized: T = from_slice(&ivec)?;
                Ok(Some(deserialized))
            }
            None => Ok(None),
        }
    }

    pub fn set<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        let serialized = to_vec(value)?;
        self.db.insert(key, serialized)?;
        Ok(())
    }
}
