use loco_rs::prelude::*;

pub use super::_entities::medias::{ActiveModel, Column, Entity, Model};

#[async_trait::async_trait]
impl ActiveModelBehavior for ActiveModel {}

impl Model {
    /// # Errors
    /// Database failure.
    pub async fn recent(db: &DatabaseConnection, limit: u64) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .order_by_desc(Column::CreatedAt)
            .limit(limit)
            .all(db)
            .await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn search(
        db: &DatabaseConnection,
        query: &str,
        kind: Option<&str>,
        limit: u64,
    ) -> ModelResult<Vec<Self>> {
        let mut find = Entity::find();
        let query = query.trim();
        if !query.is_empty() {
            find = find.filter(Column::OriginalName.like(format!("%{query}%")));
        }
        if let Some(kind) = kind.filter(|kind| *kind == "image" || *kind == "video") {
            find = find.filter(Column::Kind.eq(kind));
        }
        Ok(find
            .order_by_desc(Column::CreatedAt)
            .limit(limit.clamp(1, 200))
            .all(db)
            .await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn create_media(
        db: &DatabaseConnection,
        filename: &str,
        original_name: &str,
        mime: &str,
        kind: &str,
        size: i64,
    ) -> ModelResult<Self> {
        Ok(ActiveModel {
            filename: ActiveValue::set(filename.to_string()),
            original_name: ActiveValue::set(original_name.to_string()),
            mime: ActiveValue::set(mime.to_string()),
            kind: ActiveValue::set(kind.to_string()),
            size: ActiveValue::set(size),
            ..Default::default()
        }
        .insert(db)
        .await?)
    }
}
