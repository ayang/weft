use chrono::{DateTime, FixedOffset, Local};
use loco_rs::prelude::*;
use serde_json::Value;

pub use super::_entities::chart_data_caches::{ActiveModel, Column, Entity, Model};

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
    pub async fn find_for_chart(
        db: &DatabaseConnection,
        chart_spec_id: i64,
    ) -> ModelResult<Option<Self>> {
        Ok(Entity::find()
            .filter(Column::ChartSpecId.eq(chart_spec_id))
            .one(db)
            .await?)
    }

    /// Insert or replace the cache row for a chart.
    ///
    /// # Errors
    /// Database failure.
    pub async fn upsert(
        db: &DatabaseConnection,
        chart_spec_id: i64,
        payload: &Value,
        etag: &str,
        ttl_secs: u64,
    ) -> ModelResult<Self> {
        let now = now_tz();
        let expires_at =
            Some(now + chrono::Duration::seconds(i64::try_from(ttl_secs).unwrap_or(300)));
        if let Some(existing) = Self::find_for_chart(db, chart_spec_id).await? {
            let mut active: ActiveModel = existing.into();
            active.payload = ActiveValue::set(payload.clone());
            active.etag = ActiveValue::set(etag.to_string());
            active.fetched_at = ActiveValue::set(now);
            active.expires_at = ActiveValue::set(expires_at);
            active.updated_at = ActiveValue::set(now);
            return Ok(active.update(db).await?);
        }
        Ok(ActiveModel {
            chart_spec_id: ActiveValue::set(chart_spec_id),
            payload: ActiveValue::set(payload.clone()),
            etag: ActiveValue::set(etag.to_string()),
            fetched_at: ActiveValue::set(now),
            expires_at: ActiveValue::set(expires_at),
            ..Default::default()
        }
        .insert(db)
        .await?)
    }

    #[must_use]
    pub fn is_fresh(&self) -> bool {
        self.expires_at
            .is_some_and(|expires| expires > Local::now().fixed_offset())
    }

    /// Delete cache rows that have been stale for more than `days` days.
    ///
    /// # Errors
    /// Database failure.
    pub async fn delete_stale(db: &DatabaseConnection, days: i64) -> ModelResult<u64> {
        let cutoff = now_tz() - chrono::Duration::days(days.max(1));
        let result = Entity::delete_many()
            .filter(Column::ExpiresAt.lt(cutoff))
            .exec(db)
            .await?;
        Ok(result.rows_affected)
    }
}

fn now_tz() -> DateTime<FixedOffset> {
    Local::now().fixed_offset()
}
