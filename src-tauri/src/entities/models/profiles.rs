use sea_orm::{entity::prelude::*, FromJsonQueryResult};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, DeriveEntityModel)]
#[sea_orm(table_name = "profiles")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub name: String,
    pub file_name: String,
    pub format: String,
    pub url: Option<String>,
    pub user_agent: Option<String>,
    pub headers: Option<serde_json::Value>,
    pub subsciption: Option<SubscriptionInfo>,
    pub created_at: Option<chrono::NaiveDateTime>,
    pub updated_at: Option<chrono::NaiveDateTime>,
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Deserialize, Serialize, FromJsonQueryResult)]
pub struct SubscriptionInfo {
    pub upload: usize,
    pub download: usize,
    pub total: usize,
    pub expire: usize,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
