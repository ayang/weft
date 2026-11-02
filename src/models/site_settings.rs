use chrono::{DateTime, FixedOffset, Local};
use loco_rs::prelude::*;

pub use super::_entities::site_settings::{ActiveModel, Column, Entity, Model};

#[async_trait::async_trait]
impl ActiveModelBehavior for ActiveModel {
    async fn before_save<C>(self, _db: &C, insert: bool) -> std::result::Result<Self, DbErr>
    where
        C: ConnectionTrait,
    {
        if !insert && self.updated_at.is_unchanged() {
            let mut this = self;
            this.updated_at = ActiveValue::Set(now_tz());
            Ok(this)
        } else {
            Ok(self)
        }
    }
}

impl Model {
    /// # Errors
    /// Database failure.
    pub async fn singleton(db: &DatabaseConnection) -> ModelResult<Option<Self>> {
        Ok(Entity::find().order_by_asc(Column::Id).one(db).await?)
    }

    /// Save the singleton personalization row without touching content.
    ///
    /// # Errors
    /// Database failure.
    #[allow(clippy::too_many_arguments)]
    pub async fn save(
        db: &DatabaseConnection,
        title: &str,
        brand: &str,
        tagline: &str,
        description: &str,
        author: &str,
        base_url: &str,
        hero_image: &str,
        posts_per_page: i64,
    ) -> ModelResult<Self> {
        if let Some(existing) = Self::singleton(db).await? {
            let mut active: ActiveModel = existing.into();
            active.title = ActiveValue::Set(title.to_string());
            active.brand = ActiveValue::Set(brand.to_string());
            active.tagline = ActiveValue::Set(tagline.to_string());
            active.description = ActiveValue::Set(description.to_string());
            active.author = ActiveValue::Set(author.to_string());
            active.base_url = ActiveValue::Set(base_url.to_string());
            active.hero_image = ActiveValue::Set(
                (!hero_image.trim().is_empty()).then(|| hero_image.trim().to_string()),
            );
            active.posts_per_page = ActiveValue::Set(posts_per_page);
            active.updated_at = ActiveValue::Set(now_tz());
            return Ok(active.update(db).await?);
        }
        Ok(ActiveModel {
            title: ActiveValue::set(title.to_string()),
            brand: ActiveValue::set(brand.to_string()),
            tagline: ActiveValue::set(tagline.to_string()),
            description: ActiveValue::set(description.to_string()),
            author: ActiveValue::set(author.to_string()),
            base_url: ActiveValue::set(base_url.to_string()),
            hero_image: ActiveValue::set(
                (!hero_image.trim().is_empty()).then(|| hero_image.trim().to_string()),
            ),
            posts_per_page: ActiveValue::set(posts_per_page),
            ..Default::default()
        }
        .insert(db)
        .await?)
    }
}

fn now_tz() -> DateTime<FixedOffset> {
    Local::now().fixed_offset()
}
