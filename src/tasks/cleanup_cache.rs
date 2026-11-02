use loco_rs::{bgworker::BackgroundWorker, prelude::*};

use crate::workers::cleanup_cache::{Worker, WorkerArgs};

pub struct CleanupCache;

#[async_trait]
impl Task for CleanupCache {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "cache:cleanup".to_string(),
            detail: "Delete stale chart data (older than 30 days) and completed AI sessions older than 180 days.".to_string(),
        }
    }

    async fn run(&self, ctx: &AppContext, _vars: &task::Vars) -> Result<()> {
        Worker::build(ctx).perform(WorkerArgs {}).await?;
        Ok(())
    }
}
