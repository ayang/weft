use std::str::FromStr;

use chrono::{DateTime, FixedOffset, Local};
use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::models::chart_data_caches;

pub use super::_entities::chart_specs::{ActiveModel, Column, Entity, Model};

pub const STATUS_ACTIVE: &str = "active";
pub const STATUS_PAUSED: &str = "paused";
pub const STATUS_ERROR: &str = "error";

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

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ChartParams {
    pub title: String,
    pub intent: String,
    pub kind: String,
    pub data_source_type: String,
    pub data_source_config: String,
    pub render_config: String,
    pub refresh_cron: String,
    pub status: String,
}

impl Model {
    /// # Errors
    /// `EntityNotFound` when the chart does not exist.
    pub async fn find_by_id(db: &DatabaseConnection, id: i64) -> ModelResult<Self> {
        Entity::find_by_id(id)
            .one(db)
            .await?
            .ok_or(ModelError::EntityNotFound)
    }

    /// # Errors
    /// Database failure.
    pub async fn for_post(db: &DatabaseConnection, post_id: i64) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .filter(Column::PostId.eq(post_id))
            .filter(Column::Status.ne(STATUS_PAUSED))
            .order_by_asc(Column::Id)
            .all(db)
            .await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn all_for_admin(db: &DatabaseConnection) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .order_by_desc(Column::UpdatedAt)
            .order_by_desc(Column::Id)
            .all(db)
            .await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn with_refresh_cron(db: &DatabaseConnection) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .filter(Column::Status.ne(STATUS_PAUSED))
            .filter(Column::RefreshCron.is_not_null())
            .order_by_asc(Column::Id)
            .all(db)
            .await?)
    }

    /// # Errors
    /// Database failure, invalid JSON, or oversized spec.
    pub async fn create_chart(
        db: &DatabaseConnection,
        post_id: i64,
        params: &ChartParams,
    ) -> ModelResult<Self> {
        let title = params.title.trim();
        if title.is_empty() {
            return Err(ModelError::msg("chart title is required"));
        }
        validate_source(&params.data_source_type)?;
        validate_cron(&params.refresh_cron)?;
        let data_source_config =
            parse_json_object(&params.data_source_config, "data_source_config")?;
        let render_config = parse_render_config(&params.render_config)?;
        let status = match params.status.trim() {
            "" => STATUS_ACTIVE,
            STATUS_ACTIVE => STATUS_ACTIVE,
            STATUS_PAUSED => STATUS_PAUSED,
            other => {
                return Err(ModelError::msg(&format!("invalid chart status `{other}`")));
            }
        };
        Ok(ActiveModel {
            post_id: ActiveValue::set(post_id),
            title: ActiveValue::set(title.to_string()),
            intent: ActiveValue::set(non_empty(&params.intent)),
            kind: ActiveValue::set(params.kind.trim().to_string()),
            data_source_type: ActiveValue::set(params.data_source_type.trim().to_string()),
            data_source_config: ActiveValue::set(data_source_config),
            render_config: ActiveValue::set(render_config),
            refresh_cron: ActiveValue::set(non_empty(&params.refresh_cron)),
            last_run_at: ActiveValue::set(None),
            last_success_at: ActiveValue::set(None),
            status: ActiveValue::set(status.to_string()),
            error_message: ActiveValue::set(None),
            failure_count: ActiveValue::set(0),
            next_attempt_at: ActiveValue::set(None),
            ..Default::default()
        }
        .insert(db)
        .await?)
    }

    /// # Errors
    /// Database failure, invalid JSON, or oversized spec.
    pub async fn update_chart(
        &self,
        db: &DatabaseConnection,
        params: &ChartParams,
    ) -> ModelResult<Self> {
        let title = params.title.trim();
        if title.is_empty() {
            return Err(ModelError::msg("chart title is required"));
        }
        validate_source(&params.data_source_type)?;
        validate_cron(&params.refresh_cron)?;
        let data_source_config =
            parse_json_object(&params.data_source_config, "data_source_config")?;
        let render_config = parse_render_config(&params.render_config)?;
        let status = match params.status.trim() {
            "" => self.status.clone(),
            STATUS_ACTIVE => STATUS_ACTIVE.to_string(),
            STATUS_PAUSED => STATUS_PAUSED.to_string(),
            other => return Err(ModelError::msg(&format!("invalid chart status `{other}`"))),
        };
        let mut active: ActiveModel = self.clone().into();
        active.title = ActiveValue::set(title.to_string());
        active.intent = ActiveValue::set(non_empty(&params.intent));
        active.kind = ActiveValue::set(params.kind.trim().to_string());
        active.data_source_type = ActiveValue::set(params.data_source_type.trim().to_string());
        active.data_source_config = ActiveValue::set(data_source_config);
        active.render_config = ActiveValue::set(render_config);
        active.refresh_cron = ActiveValue::set(non_empty(&params.refresh_cron));
        active.status = ActiveValue::set(status);
        active.updated_at = ActiveValue::set(now_tz());
        Ok(active.update(db).await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn record_success(&self, db: &DatabaseConnection) -> ModelResult<Self> {
        let mut active: ActiveModel = self.clone().into();
        let now = now_tz();
        active.status = ActiveValue::set(STATUS_ACTIVE.to_string());
        active.last_run_at = ActiveValue::set(Some(now));
        active.last_success_at = ActiveValue::set(Some(now));
        active.failure_count = ActiveValue::set(0);
        active.next_attempt_at = ActiveValue::set(None);
        active.error_message = ActiveValue::set(None);
        active.updated_at = ActiveValue::set(now);
        Ok(active.update(db).await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn record_failure(
        &self,
        db: &DatabaseConnection,
        error: &str,
        max_attempts: i64,
    ) -> ModelResult<Self> {
        let failures = self.failure_count.saturating_add(1);
        let shift = u32::try_from(failures.min(6)).unwrap_or(6);
        let backoff_secs = 60_i64.saturating_mul(1_i64 << shift).min(3600);
        let now = now_tz();
        let mut active: ActiveModel = self.clone().into();
        active.last_run_at = ActiveValue::set(Some(now));
        active.failure_count = ActiveValue::set(failures);
        active.status = ActiveValue::set(STATUS_ERROR.to_string());
        active.error_message = ActiveValue::set(Some(crate::util::truncate_chars(error, 1000)));
        active.next_attempt_at = ActiveValue::set(
            (failures < max_attempts).then_some(now + chrono::Duration::seconds(backoff_secs)),
        );
        active.updated_at = ActiveValue::set(now);
        Ok(active.update(db).await?)
    }

    /// # Errors
    /// Database failure or invalid status.
    pub async fn set_status(&self, db: &DatabaseConnection, status: &str) -> ModelResult<Self> {
        if !matches!(status, STATUS_ACTIVE | STATUS_PAUSED) {
            return Err(ModelError::msg("invalid chart status"));
        }
        let mut active: ActiveModel = self.clone().into();
        active.status = ActiveValue::set(status.to_string());
        active.updated_at = ActiveValue::set(now_tz());
        Ok(active.update(db).await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn delete_chart(&self, db: &DatabaseConnection) -> ModelResult<()> {
        let txn = db.begin().await?;
        chart_data_caches::Entity::delete_many()
            .filter(chart_data_caches::Column::ChartSpecId.eq(self.id))
            .exec(&txn)
            .await?;
        Entity::delete_by_id(self.id).exec(&txn).await?;
        txn.commit().await?;
        Ok(())
    }

    #[must_use]
    pub fn render_config(&self) -> &Value {
        &self.render_config
    }
}

fn validate_source(value: &str) -> ModelResult<()> {
    match value.trim() {
        "static" | "sql" | "http" => Ok(()),
        other => Err(ModelError::msg(&format!(
            "unsupported data_source_type `{other}`; expected static, sql or http"
        ))),
    }
}

fn validate_cron(value: &str) -> ModelResult<()> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(());
    }
    cron::Schedule::from_str(value)
        .map(|_| ())
        .map_err(|err| ModelError::msg(&format!("invalid refresh_cron: {err}")))
}

fn parse_json_object(value: &str, field: &str) -> ModelResult<Value> {
    let value = value.trim();
    let parsed: Value = if value.is_empty() {
        Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_str(value)
            .map_err(|err| ModelError::msg(&format!("invalid {field} JSON: {err}")))?
    };
    if !parsed.is_object() {
        return Err(ModelError::msg(&format!("{field} must be a JSON object")));
    }
    Ok(parsed)
}

fn parse_render_config(value: &str) -> ModelResult<Value> {
    let parsed = parse_json_object(value, "render_config")?;
    if parsed.to_string().len() > 256 * 1024 {
        return Err(ModelError::msg("render_config is too large (max 256 KiB)"));
    }
    Ok(parsed)
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn now_tz() -> DateTime<FixedOffset> {
    Local::now().fixed_offset()
}
