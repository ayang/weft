use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};

pub struct Worker {
    pub ctx: AppContext,
}

#[derive(Deserialize, Debug, Serialize)]
pub struct WorkerArgs {
    pub chart_id: i64,
}

#[async_trait]
impl BackgroundWorker<WorkerArgs> for Worker {
    fn build(ctx: &AppContext) -> Self {
        Self { ctx: ctx.clone() }
    }

    fn class_name() -> String {
        "RefreshChartWorker".to_string()
    }

    async fn perform(&self, args: WorkerArgs) -> Result<()> {
        crate::charts::refresh_chart(&self.ctx, args.chart_id).await?;
        Ok(())
    }
}
