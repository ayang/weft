use loco_rs::prelude::*;

use crate::models::{post_tags, posts};

pub use super::_entities::tags::{ActiveModel, Column, Entity, Model};

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

impl Model {
    /// # Errors
    /// `EntityNotFound` when the tag does not exist.
    pub async fn find_by_slug(db: &DatabaseConnection, slug: &str) -> ModelResult<Self> {
        Entity::find()
            .filter(Column::Slug.eq(slug))
            .one(db)
            .await?
            .ok_or(ModelError::EntityNotFound)
    }

    /// # Errors
    /// Database failure.
    pub async fn published_posts(&self, db: &DatabaseConnection) -> ModelResult<Vec<posts::Model>> {
        let post_ids = post_tags::Entity::find()
            .filter(post_tags::Column::TagId.eq(self.id))
            .all(db)
            .await?
            .into_iter()
            .map(|link| link.post_id)
            .collect::<Vec<_>>();
        if post_ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(posts::Entity::find()
            .filter(posts::Column::Id.is_in(post_ids))
            .filter(posts::Column::Status.eq(posts::STATUS_PUBLISHED))
            .order_by_desc(posts::Column::PublishedAt)
            .all(db)
            .await?)
    }
}
