use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    models::{chart_specs, posts, tags},
    views::public::SiteView,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTag {
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPostSummary {
    pub slug: String,
    pub title: String,
    pub summary: String,
    pub url: String,
    pub markdown_url: String,
    pub published_at: String,
    pub updated_at: String,
    pub reading_minutes: i64,
    pub tags: Vec<AgentTag>,
}

impl AgentPostSummary {
    #[must_use]
    pub fn from_model(post: &posts::Model, tags: &[tags::Model], base_url: &str) -> Self {
        let slug = post.slug.clone();
        Self {
            slug: slug.clone(),
            title: post.title.clone(),
            summary: post.summary.clone().unwrap_or_default(),
            url: format!("{base_url}/posts/{slug}"),
            markdown_url: format!("{base_url}/posts/{slug}.md"),
            published_at: post
                .published_at
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default(),
            updated_at: post.updated_at.to_rfc3339(),
            reading_minutes: post.reading_minutes.max(1),
            tags: tags
                .iter()
                .map(|tag| AgentTag {
                    name: tag.name.clone(),
                    slug: tag.slug.clone(),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentChartRef {
    pub chart_id: i64,
    pub title: String,
    pub kind: String,
    pub data_url: String,
}

impl From<(&chart_specs::Model, &str)> for AgentChartRef {
    fn from((chart, base_url): (&chart_specs::Model, &str)) -> Self {
        Self {
            chart_id: chart.id,
            title: chart.title.clone(),
            kind: chart.kind.clone(),
            data_url: format!("{base_url}/api/agent/charts/{}", chart.id),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPostDetail {
    pub post: AgentPostSummary,
    pub markdown: String,
    pub charts: Vec<AgentChartRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentListResponse {
    pub site: SiteView,
    pub generated_at: String,
    pub count: usize,
    pub posts: Vec<AgentPostSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentChartResponse {
    pub chart_id: i64,
    pub title: String,
    pub kind: String,
    pub spec: Value,
    pub data: Value,
    pub updated_at: String,
    pub etag: String,
    pub stale: bool,
}

#[must_use]
pub fn markdown_document(post: &posts::Model, tags: &[tags::Model]) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("slug: {}\n", post.slug));
    out.push_str(&format!("title: {}\n", post.title.replace('\n', " ")));
    if let Some(summary) = &post.summary {
        out.push_str(&format!("summary: {}\n", summary.replace('\n', " ")));
    }
    out.push_str(&format!("status: {}\n", post.status));
    out.push_str(&format!(
        "published_at: {}\n",
        post.published_at
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default()
    ));
    out.push_str(&format!("updated_at: {}\n", post.updated_at.to_rfc3339()));
    out.push_str(&format!(
        "tags: {}\n",
        tags.iter()
            .map(|tag| tag.name.clone())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    out.push_str("---\n\n");
    out.push_str(&post.body_md);
    if !post.body_md.ends_with('\n') {
        out.push('\n');
    }
    out
}
