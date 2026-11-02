use loco_rs::prelude::*;

pub use super::_entities::categories::{ActiveModel, Column, Entity, Model};

#[async_trait::async_trait]
impl ActiveModelBehavior for ActiveModel {}

impl Model {
    /// # Errors
    /// Database failure.
    pub async fn all(db: &DatabaseConnection) -> ModelResult<Vec<Self>> {
        Ok(Entity::find().order_by_asc(Column::Name).all(db).await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn find_by_slug(db: &DatabaseConnection, slug: &str) -> ModelResult<Option<Self>> {
        Ok(Entity::find().filter(Column::Slug.eq(slug)).one(db).await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn create_category(
        db: &DatabaseConnection,
        name: &str,
        slug: &str,
    ) -> ModelResult<Self> {
        Ok(ActiveModel {
            name: ActiveValue::set(name.to_string()),
            slug: ActiveValue::set(slug.to_string()),
            description: ActiveValue::set(None),
            ..Default::default()
        }
        .insert(db)
        .await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn clear_all(db: &DatabaseConnection) -> ModelResult<()> {
        Entity::delete_many().exec(db).await?;
        Ok(())
    }
}
