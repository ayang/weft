use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    ai,
    models::{ai_sessions, embeddings, posts},
    util,
};

pub struct Worker {
    pub ctx: AppContext,
}

#[derive(Deserialize, Debug, Serialize)]
pub struct WorkerArgs {
    pub post_id: i64,
}

#[async_trait]
impl BackgroundWorker<WorkerArgs> for Worker {
    fn build(ctx: &AppContext) -> Self {
        Self { ctx: ctx.clone() }
    }

    fn class_name() -> String {
        "IndexPostWorker".to_string()
    }

    async fn perform(&self, args: WorkerArgs) -> Result<()> {
        let started = std::time::Instant::now();
        let post = posts::Model::find_by_id(&self.ctx.db, args.post_id).await?;
        if post.status != posts::STATUS_PUBLISHED {
            return Ok(());
        }
        let ai_settings = ai::effective_ai_settings(&self.ctx).await?;
        let session = ai_sessions::Model::start(
            &self.ctx.db,
            Some(post.id),
            "index",
            "local",
            "keyword-chunks-v1",
            serde_json::json!([{"role": "system", "content": "index post chunks for retrieval"}]),
        )
        .await?;
        let chunks = util::chunk_text(&post.body_md, 900, 120);
        let model = if ai_settings.embeddings_enabled && !ai_settings.is_offline() {
            // The interface is in place; a vector backend can fill these columns
            // without changing the retrieval call sites.
            ai_settings.embedding_model.clone()
        } else {
            "keyword-fallback".to_string()
        };
        let inputs = chunks
            .into_iter()
            .enumerate()
            .map(|(index, content)| embeddings::ChunkInput {
                index: i64::try_from(index).unwrap_or(0),
                content,
                embedding: None,
            })
            .collect::<Vec<_>>();
        let count =
            embeddings::Model::replace_for_post(&self.ctx.db, post.id, &model, &inputs).await?;
        session
            .complete(
                &self.ctx.db,
                serde_json::json!({"chunks": count, "model": model}),
                0,
                0,
                0.0,
                None,
            )
            .await?;
        tracing::info!(
            post_id = post.id,
            chunks = count,
            model = %model,
            elapsed_ms = started.elapsed().as_millis(),
            "post indexed"
        );
        crate::controllers::llms::invalidate(&self.ctx).await;
        Ok(())
    }
}
