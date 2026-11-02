use serde::{Deserialize, Serialize};

use crate::{
    data::settings::Settings,
    models::{categories, chart_data_caches, chart_specs, posts, tags},
    util,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteView {
    pub title: String,
    pub brand: String,
    pub tagline: String,
    pub description: String,
    pub author: String,
    pub base_url: String,
    pub hero_image: String,
    pub agent_api_enabled: bool,
}

impl SiteView {
    #[must_use]
    pub fn from_settings(settings: &Settings) -> Self {
        let title = settings.site.title.clone();
        let brand = if settings.site.brand.trim().is_empty() {
            title.clone()
        } else {
            settings.site.brand.clone()
        };
        let tagline = if settings.site.tagline.trim().is_empty() {
            "人与 AI 的编织现场".to_string()
        } else {
            settings.site.tagline.clone()
        };
        Self {
            title,
            brand,
            tagline,
            description: settings.site.description.clone(),
            author: settings.site.author.clone(),
            base_url: settings.site.base_url.trim_end_matches('/').to_string(),
            hero_image: if settings.site.hero_image.trim().is_empty() {
                "/static/hero-banner.svg".to_string()
            } else {
                settings.site.hero_image.trim().to_string()
            },
            agent_api_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagView {
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub count: i64,
}

impl From<&tags::Model> for TagView {
    fn from(tag: &tags::Model) -> Self {
        Self {
            name: tag.name.clone(),
            slug: tag.slug.clone(),
            count: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryView {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub count: i64,
}

impl CategoryView {
    #[must_use]
    pub fn from_model(category: &categories::Model, count: i64) -> Self {
        Self {
            id: category.id,
            name: category.name.clone(),
            slug: category.slug.clone(),
            count,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostCard {
    pub id: i64,
    pub slug: String,
    pub title: String,
    pub summary: String,
    pub date: String,
    pub reading_minutes: i64,
    pub pinned: bool,
    pub tags: Vec<TagView>,
    pub category: Option<CategoryView>,
    pub cover_image: String,
}

impl PostCard {
    #[must_use]
    pub fn from_model(post: &posts::Model, tags: Vec<tags::Model>) -> Self {
        Self {
            id: post.id,
            slug: post.slug.clone(),
            title: post.title.clone(),
            summary: post
                .summary
                .clone()
                .or_else(|| {
                    Some(util::truncate_chars(
                        &crate::ai::strip_markdown(&post.body_md),
                        180,
                    ))
                })
                .unwrap_or_default(),
            date: post.published_at.map_or_else(
                || util::format_date(&post.created_at),
                |dt| util::format_date(&dt),
            ),
            reading_minutes: post.reading_minutes.max(1),
            pinned: post.pinned.unwrap_or(false),
            tags: tags.iter().map(TagView::from).collect(),
            category: None,
            cover_image: post
                .cover_image
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "/static/default-cover.svg".to_string()),
        }
    }

    #[must_use]
    pub fn with_category(mut self, category: Option<CategoryView>) -> Self {
        self.category = category;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChartView {
    pub id: i64,
    pub title: String,
    pub kind: String,
    pub intent: String,
    pub status: String,
    pub error_message: String,
    pub fetched_at: String,
    pub stale: bool,
    pub last_success_at: String,
    pub height: u16,
}

impl ChartView {
    #[must_use]
    pub fn from_model(
        chart: &chart_specs::Model,
        cache: Option<&chart_data_caches::Model>,
    ) -> Self {
        Self {
            id: chart.id,
            title: chart.title.clone(),
            kind: chart.kind.clone(),
            intent: chart.intent.clone().unwrap_or_default(),
            status: chart.status.clone(),
            error_message: chart.error_message.clone().unwrap_or_default(),
            fetched_at: cache
                .map(|cache| util::format_date(&cache.fetched_at))
                .unwrap_or_default(),
            stale: cache.is_none_or(|cache| !cache.is_fresh()),
            last_success_at: chart
                .last_success_at
                .map(|dt| util::format_date(&dt))
                .unwrap_or_default(),
            height: chart
                .render_config
                .get("_weft")
                .and_then(|meta| meta.get("height"))
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(380.0)
                .clamp(260.0, 900.0) as u16,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostDetail {
    pub id: i64,
    pub slug: String,
    pub title: String,
    pub summary: String,
    pub date: String,
    pub updated: String,
    pub reading_minutes: i64,
    pub tags: Vec<TagView>,
    pub cover_image: String,
    pub markdown_url: String,
    pub category: Option<CategoryView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomeView {
    pub site: SiteView,
    pub posts: Vec<PostCard>,
    pub pinned: Vec<PostCard>,
    pub tags: Vec<TagView>,
    pub categories: Vec<CategoryView>,
    pub total_posts: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostPageView {
    pub site: SiteView,
    pub post: PostDetail,
    pub body_html: String,
    pub charts: Vec<ChartView>,
    pub related: Vec<PostCard>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryPageView {
    pub site: SiteView,
    pub category: CategoryView,
    pub posts: Vec<PostCard>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagPageView {
    pub site: SiteView,
    pub tag: TagView,
    pub posts: Vec<PostCard>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchPageView {
    pub site: SiteView,
    pub query: String,
    pub posts: Vec<PostCard>,
    pub searched: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveMonth {
    pub month: String,
    pub posts: Vec<PostCard>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveYear {
    pub year: String,
    pub months: Vec<ArchiveMonth>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivePageView {
    pub site: SiteView,
    pub years: Vec<ArchiveYear>,
    pub total_posts: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChartDataView {
    pub chart_id: i64,
    pub title: String,
    pub kind: String,
    pub spec: serde_json::Value,
    pub data: serde_json::Value,
    pub updated_at: String,
    pub etag: String,
    pub stale: bool,
    pub status: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostMarkdown {
    pub slug: String,
    pub title: String,
    pub summary: String,
    pub published_at: String,
    pub updated_at: String,
    pub tags: Vec<String>,
    pub body: String,
}
