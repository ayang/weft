use loco_rs::prelude::*;

pub use super::_entities::embeddings::{ActiveModel, Column, Entity, Model};

#[async_trait::async_trait]
impl ActiveModelBehavior for ActiveModel {
    async fn before_save<C>(self, _db: &C, insert: bool) -> std::result::Result<Self, DbErr>
    where
        C: ConnectionTrait,
    {
        if !insert && self.updated_at.is_unchanged() {
            let mut this = self;
            this.updated_at = ActiveValue::Set(chrono::Utc::now().into());
            Ok(this)
        } else {
            Ok(self)
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChunkInput {
    pub index: i64,
    pub content: String,
    pub embedding: Option<Vec<u8>>,
}

impl Model {
    /// # Errors
    /// Database failure.
    pub async fn for_post(db: &DatabaseConnection, post_id: i64) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .filter(Column::PostId.eq(post_id))
            .order_by_asc(Column::ChunkIndex)
            .all(db)
            .await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn replace_for_post(
        db: &DatabaseConnection,
        post_id: i64,
        model: &str,
        chunks: &[ChunkInput],
    ) -> ModelResult<u64> {
        let txn = db.begin().await?;
        Entity::delete_many()
            .filter(Column::PostId.eq(post_id))
            .exec(&txn)
            .await?;
        let mut inserted = 0_u64;
        for chunk in chunks {
            ActiveModel {
                post_id: ActiveValue::set(post_id),
                chunk_index: ActiveValue::set(chunk.index),
                content: ActiveValue::set(chunk.content.clone()),
                embedding: ActiveValue::set(chunk.embedding.clone()),
                model: ActiveValue::set(model.to_string()),
                ..Default::default()
            }
            .insert(&txn)
            .await?;
            inserted += 1;
        }
        txn.commit().await?;
        Ok(inserted)
    }

    /// # Errors
    /// Database failure.
    pub async fn count_for_post(db: &DatabaseConnection, post_id: i64) -> ModelResult<u64> {
        Ok(Entity::find()
            .filter(Column::PostId.eq(post_id))
            .count(db)
            .await?)
    }
}
