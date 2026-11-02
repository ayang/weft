use chrono::Utc;
use loco_rs::{bgworker::BackgroundWorker, prelude::*};

use crate::{
    charts,
    models::chart_specs,
    workers::refresh_chart::{Worker, WorkerArgs},
};

pub struct ChartsRefreshDue;

#[async_trait]
impl Task for ChartsRefreshDue {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "charts:refresh_due".to_string(),
            detail: "Find charts whose refresh_cron is due and enqueue RefreshChartWorker. Safe to run every minute.".to_string(),
        }
    }

    async fn run(&self, ctx: &AppContext, _vars: &task::Vars) -> Result<()> {
        let charts = chart_specs::Model::with_refresh_cron(&ctx.db).await?;
        let now = Utc::now();
        let mut enqueued = 0_usize;
        for chart in charts {
            match charts::is_due(&chart, now) {
                Ok(true) => {
                    let worker = Worker::build(ctx);
                    match worker.perform(WorkerArgs { chart_id: chart.id }).await {
                        Ok(()) => enqueued += 1,
                        Err(error) => {
                            tracing::warn!(chart_id = chart.id, error = %error, "scheduled chart refresh failed");
                        }
                    }
                }
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(chart_id = chart.id, error = %error, "invalid chart refresh_cron");
                }
            }
        }
        tracing::info!(refreshed = enqueued, "chart refresh scan complete");
        Ok(())
    }
}
