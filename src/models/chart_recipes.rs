use loco_rs::prelude::*;
use serde_json::json;

pub use super::_entities::chart_recipes::{ActiveModel, Column, Entity, Model};

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
    /// Database failure.
    pub async fn all(db: &DatabaseConnection) -> ModelResult<Vec<Self>> {
        Ok(Entity::find().order_by_asc(Column::Name).all(db).await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn find_by_name(db: &DatabaseConnection, name: &str) -> ModelResult<Option<Self>> {
        Ok(Entity::find().filter(Column::Name.eq(name)).one(db).await?)
    }

    /// Seed a few reusable chart recipes once. Configuration-like data, safe
    /// to run on every boot; it is a no-op after the first insert.
    ///
    /// # Errors
    /// Database failure.
    pub async fn seed_defaults(db: &DatabaseConnection) -> ModelResult<()> {
        if Entity::find().count(db).await? > 0 {
            return Ok(());
        }
        let bar_template = json!({
            "tooltip": {"trigger": "axis"},
            "grid": {"left": 40, "right": 20, "top": 32, "bottom": 32},
            "xAxis": {"type": "category"},
            "yAxis": {"type": "value"},
            "dataset": {"source": []},
            "series": [{"type": "bar", "encode": {"x": "label", "y": "value"}}]
        });
        let line_template = json!({
            "tooltip": {"trigger": "axis"},
            "grid": {"left": 40, "right": 20, "top": 32, "bottom": 32},
            "xAxis": {"type": "category"},
            "yAxis": {"type": "value"},
            "dataset": {"source": []},
            "series": [{"type": "line", "smooth": true, "encode": {"x": "label", "y": "value"}}]
        });
        let recipes = [
            (
                "发布节奏",
                "每月发布文章数量，适合用 SQL 聚合 posts.published_at。",
                "给定 SQLite 只读查询，返回两列：label（月份）和 value（数量）。",
                json!({"type":"array","items":{"label":"string","value":"number"}}),
                bar_template.clone(),
            ),
            (
                "标签分布",
                "按标签统计已发布文章数量。",
                "给定 SQLite 只读查询，返回两列：label（标签名）和 value（文章数）。",
                json!({"type":"array","items":{"label":"string","value":"number"}}),
                bar_template.clone(),
            ),
            (
                "阅读时长趋势",
                "按发布时间展示文章阅读时长。",
                "给定 SQLite 只读查询，返回两列：label（日期）和 value（分钟）。",
                json!({"type":"array","items":{"label":"string","value":"number"}}),
                line_template,
            ),
        ];
        for (name, description, prompt, schema, render) in recipes {
            ActiveModel {
                name: ActiveValue::set(name.to_string()),
                description: ActiveValue::set(Some(description.to_string())),
                prompt_pattern: ActiveValue::set(prompt.to_string()),
                data_schema: ActiveValue::set(schema),
                render_template: ActiveValue::set(render),
                created_by: ActiveValue::set(Some("system".to_string())),
                ..Default::default()
            }
            .insert(db)
            .await?;
        }
        Ok(())
    }
}
