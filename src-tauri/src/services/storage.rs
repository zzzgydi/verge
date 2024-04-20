use crate::utils::dirs;
use anyhow::Result;
use once_cell::sync::OnceCell;
use sea_orm::{entity::prelude::*, Database, DatabaseConnection, QuerySelect};
use serde::{Deserialize, Serialize};
use serde_json::{from_slice, to_vec};
use sled::Db;

#[derive(Debug, Clone)]
pub struct Storage {
    db: Db,
    db_conn: DatabaseConnection,
}

impl Storage {
    pub fn global() -> &'static Storage {
        static SERVICE: OnceCell<Storage> = OnceCell::new();

        SERVICE.get_or_init(|| {
            let app_dir = dirs::app_dir().expect("failed to get app dir");
            let db_path = app_dir.join("verge.db");

            let db_conn = tauri::async_runtime::block_on(async {
                Database::connect(&format!("sqlite://{}?mode=rwc", db_path.to_string_lossy())).await
            })
            .expect("failed to connect to database");

            Storage {
                db: sled::open(db_path).expect("failed to open verge db"),
                db_conn,
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

    pub fn get_conn(&self) -> &DatabaseConnection {
        &self.db_conn
    }

    pub async fn get_profiles(
        &self,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<crate::entities::models::profiles::Model>> {
        use crate::entities::models::profiles::Entity as Profile;

        let profiles = Profile::find()
            .limit(limit)
            .offset(offset)
            .all(&self.db_conn)
            .await?;

        Ok(profiles)
    }
}
