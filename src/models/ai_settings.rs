use chrono::{DateTime, FixedOffset, Local};
use loco_rs::prelude::*;

pub use super::_entities::ai_settings::{ActiveModel, Column, Entity, Model};

#[async_trait::async_trait]
impl ActiveModelBehavior for ActiveModel {
    async fn before_save<C>(self, _db: &C, insert: bool) -> std::result::Result<Self, DbErr>
    where
        C: ConnectionTrait,
    {
        if !insert && self.updated_at.is_unchanged() {
            let mut this = self;
            this.updated_at = ActiveValue::Set(now_tz());
            Ok(this)
        } else {
            Ok(self)
        }
    }
}

impl Model {
    /// # Errors
    /// Database failure.
    pub async fn active(db: &DatabaseConnection) -> ModelResult<Option<Self>> {
        Ok(Entity::find()
            .filter(Column::Enabled.eq(true))
            .order_by_desc(Column::UpdatedAt)
            .order_by_desc(Column::Id)
            .one(db)
            .await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn singleton(db: &DatabaseConnection) -> ModelResult<Option<Self>> {
        Ok(Entity::find().order_by_asc(Column::Id).one(db).await?)
    }

    /// Create or update the singleton runtime AI configuration.
    ///
    /// # Errors
    /// Database failure.
    pub async fn save(
        db: &DatabaseConnection,
        provider: &str,
        base_url: &str,
        model: &str,
        api_key: Option<String>,
        enabled: bool,
    ) -> ModelResult<Self> {
        let existing = Self::singleton(db).await?;
        if let Some(existing) = existing {
            let mut active: ActiveModel = existing.into();
            active.provider = ActiveValue::Set(provider.to_string());
            active.base_url = ActiveValue::Set(base_url.to_string());
            active.model = ActiveValue::Set(model.to_string());
            if let Some(api_key) = api_key {
                active.api_key = ActiveValue::Set(Some(api_key));
            }
            active.enabled = ActiveValue::Set(enabled);
            active.updated_at = ActiveValue::Set(now_tz());
            return Ok(active.update(db).await?);
        }
        Ok(ActiveModel {
            provider: ActiveValue::set(provider.to_string()),
            base_url: ActiveValue::set(base_url.to_string()),
            model: ActiveValue::set(model.to_string()),
            api_key: ActiveValue::set(api_key),
            enabled: ActiveValue::set(enabled),
            ..Default::default()
        }
        .insert(db)
        .await?)
    }
}

fn now_tz() -> DateTime<FixedOffset> {
    Local::now().fixed_offset()
}
