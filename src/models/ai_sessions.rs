use chrono::{DateTime, FixedOffset, Local};
use loco_rs::prelude::*;
use serde_json::Value;

pub use super::_entities::ai_sessions::{ActiveModel, Column, Entity, Model};

pub const STATUS_QUEUED: &str = "queued";
pub const STATUS_RUNNING: &str = "running";
pub const STATUS_SUCCEEDED: &str = "succeeded";
pub const STATUS_FAILED: &str = "failed";

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
    /// Record a session *before* the provider is called, so a crash leaves an
    /// auditable `running` row instead of a silent gap.
    ///
    /// # Errors
    /// Database failure.
    pub async fn start(
        db: &DatabaseConnection,
        post_id: Option<i64>,
        purpose: &str,
        provider: &str,
        model: &str,
        messages: Value,
    ) -> ModelResult<Self> {
        Ok(ActiveModel {
            post_id: ActiveValue::set(post_id),
            purpose: ActiveValue::set(purpose.to_string()),
            provider: ActiveValue::set(provider.to_string()),
            model: ActiveValue::set(model.to_string()),
            messages: ActiveValue::set(messages),
            tool_calls: ActiveValue::set(None),
            status: ActiveValue::set(STATUS_RUNNING.to_string()),
            input_tokens: ActiveValue::set(None),
            output_tokens: ActiveValue::set(None),
            cost_estimate: ActiveValue::set(None),
            error: ActiveValue::set(None),
            completed_at: ActiveValue::set(None),
            ..Default::default()
        }
        .insert(db)
        .await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn complete(
        &self,
        db: &DatabaseConnection,
        response: Value,
        input_tokens: i64,
        output_tokens: i64,
        cost_estimate: f64,
        tool_calls: Option<Value>,
    ) -> ModelResult<Self> {
        let mut messages = self.messages.clone();
        if let Some(array) = messages.as_array_mut() {
            array.push(serde_json::json!({"role": "assistant", "content": response}));
        }
        let mut active: ActiveModel = self.clone().into();
        let now = now_tz();
        active.messages = ActiveValue::set(messages);
        active.tool_calls = ActiveValue::set(tool_calls);
        active.status = ActiveValue::set(STATUS_SUCCEEDED.to_string());
        active.input_tokens = ActiveValue::set(Some(input_tokens));
        active.output_tokens = ActiveValue::set(Some(output_tokens));
        active.cost_estimate = ActiveValue::set(Some(cost_estimate));
        active.error = ActiveValue::set(None);
        active.completed_at = ActiveValue::set(Some(now));
        active.updated_at = ActiveValue::set(now);
        Ok(active.update(db).await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn fail(&self, db: &DatabaseConnection, error: &str) -> ModelResult<Self> {
        let mut active: ActiveModel = self.clone().into();
        let now = now_tz();
        active.status = ActiveValue::set(STATUS_FAILED.to_string());
        active.error = ActiveValue::set(Some(crate::util::truncate_chars(error, 2000)));
        active.completed_at = ActiveValue::set(Some(now));
        active.updated_at = ActiveValue::set(now);
        Ok(active.update(db).await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn find_by_id(db: &DatabaseConnection, id: i64) -> ModelResult<Self> {
        Entity::find_by_id(id)
            .one(db)
            .await?
            .ok_or(ModelError::EntityNotFound)
    }

    /// # Errors
    /// Database failure.
    pub async fn recent(db: &DatabaseConnection, limit: i64) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .order_by_desc(Column::Id)
            .limit(u64::try_from(limit.max(1)).unwrap_or(20))
            .all(db)
            .await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn for_post(db: &DatabaseConnection, post_id: i64) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .filter(Column::PostId.eq(post_id))
            .order_by_desc(Column::Id)
            .limit(30)
            .all(db)
            .await?)
    }

    /// Count failed sessions for a post/purpose, used to cap automatic retries.
    ///
    /// # Errors
    /// Database failure.
    pub async fn failed_count_for_post(
        db: &DatabaseConnection,
        post_id: i64,
        purpose: &str,
    ) -> ModelResult<u64> {
        Ok(Entity::find()
            .filter(Column::PostId.eq(post_id))
            .filter(Column::Purpose.eq(purpose))
            .filter(Column::Status.eq(STATUS_FAILED))
            .count(db)
            .await?)
    }

    /// Sum of estimated cost over the last 24h, used as a hard budget gate.
    ///
    /// # Errors
    /// Database failure.
    pub async fn cost_last_24h(db: &DatabaseConnection) -> ModelResult<f64> {
        let since = now_tz() - chrono::Duration::hours(24);
        let rows = Entity::find()
            .filter(Column::CreatedAt.gte(since))
            .all(db)
            .await?;
        Ok(rows.iter().filter_map(|row| row.cost_estimate).sum())
    }
}

fn now_tz() -> DateTime<FixedOffset> {
    Local::now().fixed_offset()
}
