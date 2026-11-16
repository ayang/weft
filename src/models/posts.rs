use chrono::{DateTime, FixedOffset, Local};
use loco_rs::prelude::*;
use sea_orm::{sea_query::Expr, DatabaseBackend, DatabaseTransaction, Statement};
use serde::{Deserialize, Serialize};

use crate::{
    models::{ai_sessions, chart_data_caches, chart_specs, embeddings, post_tags, tags},
    util,
};

pub use super::_entities::posts::{self, ActiveModel, Column, Entity, Model};

pub const STATUS_DRAFT: &str = "draft";
pub const STATUS_REVIEW: &str = "review";
pub const STATUS_PUBLISHED: &str = "published";
pub const VALID_STATUSES: [&str; 3] = [STATUS_DRAFT, STATUS_REVIEW, STATUS_PUBLISHED];

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

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct PostParams {
    pub title: String,
    pub slug: String,
    pub summary: String,
    pub body_md: String,
    pub status: String,
    pub pinned: bool,
    pub cover_image: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Partial update used by the Agent API. Omitted fields keep their current
/// value; empty strings clear nullable columns.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct PostPatch {
    pub title: Option<String>,
    pub slug: Option<String>,
    pub summary: Option<String>,
    pub body_md: Option<String>,
    pub status: Option<String>,
    pub pinned: Option<bool>,
    pub cover_image: Option<String>,
    pub tags: Option<Vec<String>>,
    /// `replace` (default), `append`, or `prepend`, relative to the current
    /// body. Only meaningful when `body_md` is present.
    pub body_mode: Option<String>,
    /// Optimistic concurrency guard: the `updated_at` value the caller read.
    /// The update is a single conditional statement, so a concurrent edit
    /// cannot slip between the check and the write.
    pub expected_updated_at: Option<String>,
}

/// Result of a partial update. `Conflict` means the conditional update matched
/// no row because another writer changed `updated_at` first.
#[derive(Debug)]
pub enum PostWriteError {
    Model(ModelError),
    Conflict,
}

impl From<ModelError> for PostWriteError {
    fn from(value: ModelError) -> Self {
        Self::Model(value)
    }
}

impl PostPatch {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.slug.is_none()
            && self.summary.is_none()
            && self.body_md.is_none()
            && self.status.is_none()
            && self.pinned.is_none()
            && self.cover_image.is_none()
            && self.tags.is_none()
            && self.body_mode.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagCount {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub count: i64,
}

impl Model {
    /// # Errors
    /// `EntityNotFound` when the id does not exist.
    pub async fn find_by_id(db: &DatabaseConnection, id: i64) -> ModelResult<Self> {
        Entity::find_by_id(id)
            .one(db)
            .await?
            .ok_or(ModelError::EntityNotFound)
    }

    /// # Errors
    /// `EntityNotFound` when the slug does not exist.
    pub async fn find_by_slug(db: &DatabaseConnection, slug: &str) -> ModelResult<Self> {
        Entity::find()
            .filter(Column::Slug.eq(slug))
            .one(db)
            .await?
            .ok_or(ModelError::EntityNotFound)
    }

    /// # Errors
    /// `EntityNotFound` when the slug does not exist or is not published.
    pub async fn find_published_by_slug(db: &DatabaseConnection, slug: &str) -> ModelResult<Self> {
        Entity::find()
            .filter(Column::Slug.eq(slug))
            .filter(Column::Status.eq(STATUS_PUBLISHED))
            .one(db)
            .await?
            .ok_or(ModelError::EntityNotFound)
    }

    /// # Errors
    /// Database failure.
    pub async fn published(db: &DatabaseConnection, limit: i64) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .filter(Column::Status.eq(STATUS_PUBLISHED))
            .order_by_desc(Column::Pinned)
            .order_by_desc(Column::PublishedAt)
            .order_by_desc(Column::Id)
            .limit(u64::try_from(limit.max(1)).unwrap_or(20))
            .all(db)
            .await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn all_published(db: &DatabaseConnection) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .filter(Column::Status.eq(STATUS_PUBLISHED))
            .order_by_desc(Column::PublishedAt)
            .order_by_desc(Column::Id)
            .all(db)
            .await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn pinned_published(db: &DatabaseConnection) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .filter(Column::Status.eq(STATUS_PUBLISHED))
            .filter(Column::Pinned.eq(true))
            .order_by_desc(Column::PublishedAt)
            .limit(3)
            .all(db)
            .await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn all_for_admin(db: &DatabaseConnection) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .order_by_desc(Column::UpdatedAt)
            .order_by_desc(Column::Id)
            .all(db)
            .await?)
    }

    /// List posts for the Agent API. `published` is the only status that is
    /// publicly readable; `draft`, `review`, and `all` require an authenticated
    /// key (enforced by the controller).
    ///
    /// # Errors
    /// Database failure or an unknown status filter.
    pub async fn list_for_agent(db: &DatabaseConnection, status: &str) -> ModelResult<Vec<Self>> {
        match status.trim() {
            "" | STATUS_PUBLISHED => Self::all_published(db).await,
            "all" => Self::all_for_admin(db).await,
            STATUS_DRAFT | STATUS_REVIEW => Ok(Entity::find()
                .filter(Column::Status.eq(status.trim()))
                .order_by_desc(Column::UpdatedAt)
                .order_by_desc(Column::Id)
                .all(db)
                .await?),
            other => Err(ModelError::msg(&format!(
                "invalid status `{other}`; expected published, draft, review or all"
            ))),
        }
    }

    /// # Errors
    /// Database failure.
    pub async fn search_published(
        db: &DatabaseConnection,
        query: &str,
        limit: i64,
    ) -> ModelResult<Vec<Self>> {
        let needle = format!("%{}%", query.trim());
        let condition = Condition::any()
            .add(Column::Title.like(&needle))
            .add(Column::Summary.like(&needle))
            .add(Column::BodyMd.like(&needle));
        Ok(Entity::find()
            .filter(Column::Status.eq(STATUS_PUBLISHED))
            .filter(condition)
            .order_by_desc(Column::PublishedAt)
            .limit(u64::try_from(limit.max(1)).unwrap_or(20))
            .all(db)
            .await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn published_in_category(
        db: &DatabaseConnection,
        category_id: i64,
        limit: i64,
    ) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .filter(Column::Status.eq(STATUS_PUBLISHED))
            .filter(Column::CategoryId.eq(category_id))
            .order_by_desc(Column::PublishedAt)
            .order_by_desc(Column::Id)
            .limit(u64::try_from(limit.max(1)).unwrap_or(20))
            .all(db)
            .await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn count_published_in_category(
        db: &DatabaseConnection,
        category_id: i64,
    ) -> ModelResult<u64> {
        Ok(Entity::find()
            .filter(Column::Status.eq(STATUS_PUBLISHED))
            .filter(Column::CategoryId.eq(category_id))
            .count(db)
            .await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn clear_categories(db: &DatabaseConnection) -> ModelResult<()> {
        db.execute_unprepared("UPDATE posts SET category_id = NULL")
            .await?;
        Ok(())
    }

    /// # Errors
    /// Database failure.
    pub async fn set_category(
        db: &DatabaseConnection,
        post_id: i64,
        category_id: Option<i64>,
    ) -> ModelResult<()> {
        let post = Self::find_by_id(db, post_id).await?;
        let mut active: ActiveModel = post.into();
        active.category_id = ActiveValue::set(category_id);
        active.update(db).await?;
        Ok(())
    }

    /// # Errors
    /// Database failure.
    pub async fn count_published(db: &DatabaseConnection) -> ModelResult<u64> {
        Ok(Entity::find()
            .filter(Column::Status.eq(STATUS_PUBLISHED))
            .count(db)
            .await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn count_by_status(db: &DatabaseConnection, status: &str) -> ModelResult<u64> {
        Ok(Entity::find()
            .filter(Column::Status.eq(status))
            .count(db)
            .await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn tags(&self, db: &DatabaseConnection) -> ModelResult<Vec<tags::Model>> {
        let rows = post_tags::Entity::find()
            .filter(post_tags::Column::PostId.eq(self.id))
            .find_also_related(tags::Entity)
            .all(db)
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(_, tag)| tag)
            .collect::<Vec<_>>())
    }

    /// # Errors
    /// Database failure.
    pub async fn related(&self, db: &DatabaseConnection, limit: i64) -> ModelResult<Vec<Self>> {
        let tag_ids = post_tags::Entity::find()
            .filter(post_tags::Column::PostId.eq(self.id))
            .all(db)
            .await?
            .into_iter()
            .map(|link| link.tag_id)
            .collect::<Vec<_>>();
        if tag_ids.is_empty() {
            return self.recent_other(db, limit).await;
        }
        let post_ids = post_tags::Entity::find()
            .filter(post_tags::Column::TagId.is_in(tag_ids))
            .filter(post_tags::Column::PostId.ne(self.id))
            .all(db)
            .await?
            .into_iter()
            .map(|link| link.post_id)
            .collect::<Vec<_>>();
        if post_ids.is_empty() {
            return self.recent_other(db, limit).await;
        }
        Ok(Entity::find()
            .filter(Column::Status.eq(STATUS_PUBLISHED))
            .filter(Column::Id.is_in(post_ids))
            .order_by_desc(Column::PublishedAt)
            .limit(u64::try_from(limit.max(1)).unwrap_or(3))
            .all(db)
            .await?)
    }

    async fn recent_other(&self, db: &DatabaseConnection, limit: i64) -> ModelResult<Vec<Self>> {
        Ok(Entity::find()
            .filter(Column::Status.eq(STATUS_PUBLISHED))
            .filter(Column::Id.ne(self.id))
            .order_by_desc(Column::PublishedAt)
            .limit(u64::try_from(limit.max(1)).unwrap_or(3))
            .all(db)
            .await?)
    }

    /// # Errors
    /// Database failure.
    pub async fn tag_counts(db: &DatabaseConnection, limit: i64) -> ModelResult<Vec<TagCount>> {
        let sql = "SELECT t.id AS id, t.name AS name, t.slug AS slug, \
                   COUNT(pt.post_id) AS count \
                   FROM tags t \
                   LEFT JOIN post_tags pt ON pt.tag_id = t.id \
                   LEFT JOIN posts p ON p.id = pt.post_id AND p.status = 'published' \
                   GROUP BY t.id, t.name, t.slug \
                   ORDER BY count DESC, t.name ASC LIMIT ?";
        let stmt =
            Statement::from_sql_and_values(db.get_database_backend(), sql, [limit.max(1).into()]);
        let rows = db.query_all_raw(stmt).await?;
        let mut out = Vec::new();
        for row in rows {
            let count: i64 = row.try_get_by("count").unwrap_or(0);
            if count == 0 {
                continue;
            }
            out.push(TagCount {
                id: row.try_get_by("id").unwrap_or_default(),
                name: row.try_get_by("name").unwrap_or_default(),
                slug: row.try_get_by("slug").unwrap_or_default(),
                count,
            });
        }
        Ok(out)
    }

    /// Create a post and its tag links in one transaction.
    ///
    /// # Errors
    /// Validation errors, duplicate slug, or database failure.
    pub async fn create_post(db: &DatabaseConnection, params: &PostParams) -> ModelResult<Self> {
        let title = params.title.trim();
        if title.is_empty() {
            return Err(ModelError::msg("title is required"));
        }
        let status = normalize_status(&params.status)?;
        let slug = if params.slug.trim().is_empty() {
            util::slugify(title)
        } else {
            util::slugify(params.slug.trim())
        };
        let txn = db.begin().await?;
        if Entity::find()
            .filter(Column::Slug.eq(&slug))
            .one(&txn)
            .await?
            .is_some()
        {
            return Err(ModelError::EntityAlreadyExists {});
        }
        let now = now_tz();
        let published_at = (status == STATUS_PUBLISHED).then_some(now);
        let post = ActiveModel {
            slug: ActiveValue::set(slug),
            title: ActiveValue::set(title.to_string()),
            summary: ActiveValue::set(optional(&params.summary)),
            body_md: ActiveValue::set(params.body_md.clone()),
            body_ref: ActiveValue::set(None),
            cover_image: ActiveValue::set(optional(&params.cover_image)),
            status: ActiveValue::set(status.to_string()),
            pinned: ActiveValue::set(Some(params.pinned)),
            reading_minutes: ActiveValue::set(util::reading_minutes(&params.body_md)),
            published_at: ActiveValue::set(published_at),
            ..Default::default()
        }
        .insert(&txn)
        .await?;
        replace_tags(&post.id, &params.tags, &txn).await?;
        txn.commit().await?;
        Ok(post)
    }

    /// Update fields, recompute reading time, and replace tags.
    ///
    /// # Errors
    /// Validation errors, duplicate slug, or database failure.
    pub async fn update_post(
        &self,
        db: &DatabaseConnection,
        params: &PostParams,
    ) -> ModelResult<Self> {
        self.update_post_inner(db, params, None)
            .await?
            .ok_or_else(|| ModelError::msg("post update matched no rows"))
    }

    async fn update_post_inner(
        &self,
        db: &DatabaseConnection,
        params: &PostParams,
        expected_updated_at: Option<DateTime<FixedOffset>>,
    ) -> ModelResult<Option<Self>> {
        let title = params.title.trim();
        if title.is_empty() {
            return Err(ModelError::msg("title is required"));
        }
        let status = normalize_status(&params.status)?;
        let slug = if params.slug.trim().is_empty() {
            util::slugify(title)
        } else {
            util::slugify(params.slug.trim())
        };
        let txn = db.begin().await?;
        let conflict = Entity::find()
            .filter(Column::Slug.eq(&slug))
            .filter(Column::Id.ne(self.id))
            .one(&txn)
            .await?;
        if conflict.is_some() {
            return Err(ModelError::EntityAlreadyExists {});
        }
        let was_published = self.status == STATUS_PUBLISHED;
        let published_at = if status == STATUS_PUBLISHED {
            self.published_at.or_else(|| Some(now_tz()))
        } else if was_published {
            self.published_at
        } else {
            None
        };
        let mut active: ActiveModel = self.clone().into();
        active.title = ActiveValue::set(title.to_string());
        active.slug = ActiveValue::set(slug);
        active.summary = ActiveValue::set(optional(&params.summary));
        active.body_md = ActiveValue::set(params.body_md.clone());
        active.cover_image = ActiveValue::set(optional(&params.cover_image));
        active.status = ActiveValue::set(status.to_string());
        active.pinned = ActiveValue::set(Some(params.pinned));
        active.reading_minutes = ActiveValue::set(util::reading_minutes(&params.body_md));
        active.published_at = ActiveValue::set(published_at);
        active.updated_at = ActiveValue::set(now_tz());
        let mut update = Entity::update_many()
            .set(active)
            .filter(Column::Id.eq(self.id));
        if let Some(expected) = expected_updated_at {
            // SQLite stores date-times as text in more than one accepted
            // format, and may drop sub-second precision. Compare the instant
            // with `julianday` and, in addition, compare every mutable column
            // with the row we loaded. That makes the write a real
            // compare-and-swap: a concurrent edit in the same second still
            // changes at least one of these fields and loses the race.
            if db.get_database_backend() == DatabaseBackend::Sqlite {
                update = update.filter(Expr::cust_with_values(
                    "julianday(updated_at) = julianday(?)",
                    [expected.to_rfc3339()],
                ));
            } else {
                update = update.filter(Column::UpdatedAt.eq(expected));
            }
            update = update
                .filter(Column::Title.eq(self.title.clone()))
                .filter(Column::Slug.eq(self.slug.clone()))
                .filter(Column::Summary.eq(self.summary.clone()))
                .filter(Column::BodyMd.eq(self.body_md.clone()))
                .filter(Column::BodyRef.eq(self.body_ref.clone()))
                .filter(Column::CoverImage.eq(self.cover_image.clone()))
                .filter(Column::Status.eq(self.status.clone()))
                .filter(Column::Pinned.eq(self.pinned))
                .filter(Column::ReadingMinutes.eq(self.reading_minutes))
                .filter(Column::CategoryId.eq(self.category_id));
            // Like `updated_at`, SQLite may store `published_at` in a
            // different text form than a bound RFC3339 value.
            if let Some(published_at) = self.published_at {
                if db.get_database_backend() == DatabaseBackend::Sqlite {
                    update = update.filter(Expr::cust_with_values(
                        "julianday(published_at) = julianday(?)",
                        [published_at.to_rfc3339()],
                    ));
                } else {
                    update = update.filter(Column::PublishedAt.eq(published_at));
                }
            } else {
                update = update.filter(Column::PublishedAt.is_null());
            }
        }
        let result = update.exec(&txn).await?;
        if result.rows_affected == 0 {
            txn.rollback().await?;
            return Ok(None);
        }
        let post = Entity::find_by_id(self.id)
            .one(&txn)
            .await?
            .ok_or(ModelError::EntityNotFound)?;
        replace_tags(&post.id, &params.tags, &txn).await?;
        txn.commit().await?;
        Ok(Some(post))
    }

    /// Apply a partial update. This is the model-level equivalent of editing
    /// one or two fields in the admin form, without forcing callers to send
    /// the whole document back.
    ///
    /// When `expected_updated_at` is present the write is conditional on the
    /// row still having that timestamp; a concurrent edit returns
    /// [`PostWriteError::Conflict`] instead of silently overwriting it.
    ///
    /// # Errors
    /// Validation errors, duplicate slug, stale `expected_updated_at`, or
    /// database failure.
    pub async fn patch_post(
        &self,
        db: &DatabaseConnection,
        patch: &PostPatch,
    ) -> std::result::Result<Self, PostWriteError> {
        let tags = self.tags(db).await?;
        let mut params = PostParams {
            title: self.title.clone(),
            slug: self.slug.clone(),
            summary: self.summary.clone().unwrap_or_default(),
            body_md: self.body_md.clone(),
            status: self.status.clone(),
            pinned: self.pinned.unwrap_or(false),
            cover_image: self.cover_image.clone().unwrap_or_default(),
            tags: tags.iter().map(|tag| tag.name.clone()).collect(),
        };
        if let Some(title) = &patch.title {
            params.title.clone_from(title);
        }
        if let Some(slug) = &patch.slug {
            params.slug.clone_from(slug);
        }
        if let Some(summary) = &patch.summary {
            params.summary.clone_from(summary);
        }
        if let Some(status) = &patch.status {
            params.status.clone_from(status);
        }
        if let Some(pinned) = patch.pinned {
            params.pinned = pinned;
        }
        if let Some(cover_image) = &patch.cover_image {
            params.cover_image.clone_from(cover_image);
        }
        if let Some(tags) = &patch.tags {
            params.tags.clone_from(tags);
        }
        if let Some(body) = &patch.body_md {
            let mode = patch.body_mode.as_deref().unwrap_or("replace").trim();
            params.body_md = match mode {
                "" | "replace" => body.clone(),
                "append" => join_body(&self.body_md, body),
                "prepend" => join_body(body, &self.body_md),
                other => {
                    return Err(ModelError::msg(&format!(
                        "invalid body_mode `{other}`; expected replace, append or prepend"
                    ))
                    .into())
                }
            };
        } else if patch.body_mode.is_some() {
            return Err(ModelError::msg("body_mode requires body_md").into());
        }
        let expected = match patch
            .expected_updated_at
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(value) => Some(DateTime::parse_from_rfc3339(value).map_err(|_| {
                ModelError::msg("expected_updated_at must be an RFC3339 timestamp")
            })?),
            None => None,
        };
        self.update_post_inner(db, &params, expected)
            .await?
            .ok_or(PostWriteError::Conflict)
    }

    /// # Errors
    /// Database failure or invalid status.
    pub async fn set_status(&self, db: &DatabaseConnection, status: &str) -> ModelResult<Self> {
        let status = normalize_status(status)?;
        let mut active: ActiveModel = self.clone().into();
        active.status = ActiveValue::set(status.to_string());
        active.updated_at = ActiveValue::set(now_tz());
        if status == STATUS_PUBLISHED && self.published_at.is_none() {
            active.published_at = ActiveValue::set(Some(now_tz()));
        }
        Ok(active.update(db).await?)
    }

    /// Delete a post and its dependent rows explicitly (SQLite does not enable
    /// foreign-key cascades per connection by default).
    ///
    /// # Errors
    /// Database failure.
    pub async fn delete_post(&self, db: &DatabaseConnection) -> ModelResult<()> {
        let chart_ids = chart_specs::Entity::find()
            .filter(chart_specs::Column::PostId.eq(self.id))
            .all(db)
            .await?
            .into_iter()
            .map(|chart| chart.id)
            .collect::<Vec<_>>();
        let txn = db.begin().await?;
        if !chart_ids.is_empty() {
            chart_data_caches::Entity::delete_many()
                .filter(chart_data_caches::Column::ChartSpecId.is_in(chart_ids.clone()))
                .exec(&txn)
                .await?;
            chart_specs::Entity::delete_many()
                .filter(chart_specs::Column::Id.is_in(chart_ids))
                .exec(&txn)
                .await?;
        }
        embeddings::Entity::delete_many()
            .filter(embeddings::Column::PostId.eq(self.id))
            .exec(&txn)
            .await?;
        post_tags::Entity::delete_many()
            .filter(post_tags::Column::PostId.eq(self.id))
            .exec(&txn)
            .await?;
        ai_sessions::Entity::update_many()
            .set(ai_sessions::ActiveModel {
                post_id: ActiveValue::set(None),
                ..Default::default()
            })
            .filter(ai_sessions::Column::PostId.eq(self.id))
            .exec(&txn)
            .await?;
        Entity::delete_by_id(self.id).exec(&txn).await?;
        txn.commit().await?;
        Ok(())
    }
}

async fn replace_tags(
    post_id: &i64,
    names: &[String],
    txn: &DatabaseTransaction,
) -> ModelResult<()> {
    post_tags::Entity::delete_many()
        .filter(post_tags::Column::PostId.eq(*post_id))
        .exec(txn)
        .await?;
    let mut seen = std::collections::HashSet::new();
    for raw in names {
        let name = raw.trim();
        if name.is_empty() {
            continue;
        }
        let slug = util::slugify(name);
        if !seen.insert(slug.clone()) {
            continue;
        }
        let tag = match tags::Entity::find()
            .filter(tags::Column::Slug.eq(&slug))
            .one(txn)
            .await?
        {
            Some(tag) => tag,
            None => {
                tags::ActiveModel {
                    name: ActiveValue::set(name.to_string()),
                    slug: ActiveValue::set(slug),
                    ..Default::default()
                }
                .insert(txn)
                .await?
            }
        };
        post_tags::ActiveModel {
            post_id: ActiveValue::set(*post_id),
            tag_id: ActiveValue::set(tag.id),
            ..Default::default()
        }
        .insert(txn)
        .await?;
    }
    Ok(())
}

fn normalize_status(status: &str) -> ModelResult<&'static str> {
    match status.trim() {
        STATUS_DRAFT => Ok(STATUS_DRAFT),
        STATUS_REVIEW => Ok(STATUS_REVIEW),
        STATUS_PUBLISHED => Ok(STATUS_PUBLISHED),
        other => Err(ModelError::msg(&format!(
            "invalid status `{other}`; expected one of draft/review/published"
        ))),
    }
}

fn join_body(first: &str, second: &str) -> String {
    let first = first.trim_end();
    let second = second.trim_start();
    match (first.is_empty(), second.is_empty()) {
        (true, true) => String::new(),
        (true, false) => second.to_string(),
        (false, true) => first.to_string(),
        (false, false) => format!("{first}\n\n{second}"),
    }
}

fn optional(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn now_tz() -> DateTime<FixedOffset> {
    Local::now().fixed_offset()
}
