use loco_rs::{bgworker::BackgroundWorker, prelude::*};

use crate::{
    models::{embeddings, posts},
    workers::index_post::{Worker as IndexWorker, WorkerArgs as IndexArgs},
};

pub struct PostsEnrichDue;

#[async_trait]
impl Task for PostsEnrichDue {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "posts:enrich_due".to_string(),
            detail: "Recover published posts missing a search index. Enqueues at most 5 per run."
                .to_string(),
        }
    }

    async fn run(&self, ctx: &AppContext, _vars: &task::Vars) -> Result<()> {
        let posts = posts::Model::published(&ctx.db, 30).await?;
        let mut count = 0_usize;
        for post in posts {
            if count >= 5 {
                break;
            }
            if embeddings::Model::count_for_post(&ctx.db, post.id).await? == 0 {
                let worker = IndexWorker::build(ctx);
                if let Err(error) = worker.perform(IndexArgs { post_id: post.id }).await {
                    tracing::warn!(post_id = post.id, error = %error, "scheduled index failed");
                }
                count += 1;
            }
        }
        tracing::info!(processed = count, "post enrichment scan complete");
        Ok(())
    }
}
