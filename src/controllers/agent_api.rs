use chrono::{DateTime, NaiveDate, Utc};
use loco_rs::prelude::*;
use serde::Deserialize;

use crate::{
    data::settings::Settings,
    models::{chart_specs, posts},
    security::AgentAuth,
    util,
    views::{
        agent::{
            markdown_document, AgentChartResponse, AgentListResponse, AgentPostDetail,
            AgentPostSummary,
        },
        public::SiteView,
    },
};

#[derive(Debug, Deserialize)]
pub struct PostsQuery {
    pub since: Option<String>,
    pub limit: Option<i64>,
}

#[debug_handler]
async fn list_posts(
    State(ctx): State<AppContext>,
    _auth: AgentAuth,
    Query(query): Query<PostsQuery>,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let site = SiteView::from_settings(&settings);
    let since = parse_since(query.since.as_deref())?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let mut all = posts::Model::all_published(&ctx.db).await?;
    if let Some(since) = since {
        all.retain(|post| {
            post.published_at
                .is_some_and(|published| published >= since)
        });
    }
    all.truncate(usize::try_from(limit).unwrap_or(50));
    let mut summaries = Vec::with_capacity(all.len());
    for post in &all {
        let tags = post.tags(&ctx.db).await?;
        summaries.push(AgentPostSummary::from_model(
            post,
            &tags,
            settings.site.base_url.trim_end_matches('/'),
        ));
    }
    let latest = all.iter().map(|post| post.updated_at).max();
    let response = AgentListResponse {
        site,
        generated_at: Utc::now().to_rfc3339(),
        count: summaries.len(),
        posts: summaries,
    };
    let etag = util::etag_for_str(&serde_json::to_string(&response.posts)?);
    let mut builder = format::render()
        .header(
            "cache-control",
            "public, max-age=60, stale-while-revalidate=300",
        )
        .etag(&etag)?;
    if let Some(latest) = latest {
        builder = builder.header("last-modified", util::http_date(&latest));
    }
    builder.json(response)
}

#[debug_handler]
async fn post_detail(
    State(ctx): State<AppContext>,
    _auth: AgentAuth,
    Path(slug): Path<String>,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let post = posts::Model::find_published_by_slug(&ctx.db, &slug).await?;
    let tags = post.tags(&ctx.db).await?;
    let base = settings.site.base_url.trim_end_matches('/');
    let charts = chart_specs::Model::for_post(&ctx.db, post.id).await?;
    let response = AgentPostDetail {
        post: AgentPostSummary::from_model(&post, &tags, base),
        markdown: markdown_document(&post, &tags),
        charts: charts.iter().map(|chart| (chart, base).into()).collect(),
    };
    let etag = util::etag_for_str(&serde_json::to_string(&response)?);
    format::render()
        .header(
            "cache-control",
            "public, max-age=60, stale-while-revalidate=300",
        )
        .header("last-modified", util::http_date(&post.updated_at))
        .etag(&etag)?
        .json(response)
}

#[debug_handler]
async fn chart_detail(
    State(ctx): State<AppContext>,
    _auth: AgentAuth,
    Path(id): Path<i64>,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let chart = chart_specs::Model::find_by_id(&ctx.db, id).await?;
    if chart.status == chart_specs::STATUS_PAUSED {
        return not_found();
    }
    let post = posts::Model::find_by_id(&ctx.db, chart.post_id).await?;
    if post.status != posts::STATUS_PUBLISHED {
        return not_found();
    }
    let cache = crate::charts::cached(&ctx.db, chart.id).await?;
    let (data, etag, updated_at, stale, last_modified) = if let Some(cache) = &cache {
        (
            cache.payload.clone(),
            cache.etag.clone(),
            cache.fetched_at.to_rfc3339(),
            !cache.is_fresh(),
            cache.fetched_at,
        )
    } else {
        let etag = util::etag_for_str(&format!("chart:{}:{}", chart.id, chart.updated_at));
        (
            serde_json::Value::Null,
            etag,
            String::new(),
            true,
            chart.updated_at,
        )
    };
    if stale && !settings.charts.manual_refresh_only {
        let args = crate::workers::refresh_chart::WorkerArgs { chart_id: chart.id };
        if let Err(error) = crate::workers::refresh_chart::Worker::perform_later(&ctx, args).await {
            tracing::warn!(chart_id = chart.id, error = %error, "agent chart refresh enqueue failed");
        }
    }
    let response = AgentChartResponse {
        chart_id: chart.id,
        title: chart.title.clone(),
        kind: chart.kind.clone(),
        spec: chart.render_config.clone(),
        data,
        updated_at,
        etag: etag.clone(),
        stale,
    };
    format::render()
        .header(
            "cache-control",
            "public, max-age=60, stale-while-revalidate=600",
        )
        .header("x-chart-stale", if stale { "1" } else { "0" })
        .header("last-modified", util::http_date(&last_modified))
        .etag(&etag)?
        .json(response)
}

fn parse_since(value: Option<&str>) -> Result<Option<chrono::DateTime<chrono::FixedOffset>>> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if let Ok(dt) = DateTime::parse_from_rfc3339(value) {
        return Ok(Some(dt));
    }
    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        let dt = date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| Error::string("invalid since date"))?;
        return Ok(Some(
            DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc).fixed_offset(),
        ));
    }
    Err(Error::string("since must be RFC3339 or YYYY-MM-DD"))
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/agent")
        .add("/posts", get(list_posts))
        .add("/posts/{slug}", get(post_detail))
        .add("/charts/{id}", get(chart_detail))
}
