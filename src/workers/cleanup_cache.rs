use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::models::{ai_sessions, chart_data_caches};

pub struct Worker {
    pub ctx: AppContext,
}

#[derive(Deserialize, Debug, Serialize)]
pub struct WorkerArgs {}

#[async_trait]
impl BackgroundWorker<WorkerArgs> for Worker {
    fn build(ctx: &AppContext) -> Self {
        Self { ctx: ctx.clone() }
    }

    fn class_name() -> String {
        "CleanupCacheWorker".to_string()
    }

    async fn perform(&self, _args: WorkerArgs) -> Result<()> {
        let stale = chart_data_caches::Model::delete_stale(&self.ctx.db, 30).await?;
        let cutoff = chrono::Local::now().fixed_offset() - chrono::Duration::days(180);
        let old_sessions = ai_sessions::Entity::delete_many()
            .filter(ai_sessions::Column::CreatedAt.lt(cutoff))
            .filter(ai_sessions::Column::Status.eq(ai_sessions::STATUS_SUCCEEDED))
            .exec(&self.ctx.db)
            .await?
            .rows_affected;
        tracing::info!(
            stale_chart_cache = stale,
            old_ai_sessions = old_sessions,
            "cache cleanup complete"
        );
        Ok(())
    }
}
