use axum::{
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, NaiveDate, Utc};
use loco_rs::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    charts::{
        self,
        blueprint::{self, ChartOptions, VisualChartInput},
    },
    data::settings::Settings,
    media,
    models::{chart_specs, medias, posts},
    security::{agent_error, AgentAuth},
    util,
    views::{
        agent::{
            markdown_document, AgentChartDetail, AgentChartRef, AgentChartResponse,
            AgentListResponse, AgentPostDetail, AgentPostSummary,
        },
        public::SiteView,
    },
    workers::{
        index_post::{Worker as IndexWorker, WorkerArgs as IndexArgs},
        refresh_chart::{Worker as ChartWorker, WorkerArgs as ChartArgs},
    },
};

#[derive(Debug, Deserialize)]
pub struct PostsQuery {
    pub since: Option<String>,
    pub limit: Option<i64>,
    /// `published` (default), `draft`, `review`, or `all`. Anything other than
    /// `published` requires an authenticated key.
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MediaQuery {
    pub q: Option<String>,
    pub kind: Option<String>,
    pub limit: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
pub struct PostCreate {
    pub title: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub body_md: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub cover_image: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ChartWrite {
    pub title: String,
    #[serde(default)]
    pub intent: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub data: Vec<Value>,
    #[serde(default)]
    pub columns: Vec<String>,
    #[serde(default)]
    pub x_field: String,
    #[serde(default)]
    pub y_fields: Vec<String>,
    #[serde(default)]
    pub options: Option<ChartOptions>,
    #[serde(default)]
    pub data_source_type: Option<String>,
    #[serde(default)]
    pub sql_query: Option<String>,
    #[serde(default)]
    pub sql_limit: Option<usize>,
    #[serde(default)]
    pub http_url: Option<String>,
    #[serde(default)]
    pub http_data_path: Option<String>,
    /// Advanced mode: raw ECharts option. Must be sent together with
    /// `data_source_config`.
    #[serde(default)]
    pub render_config: Option<Value>,
    #[serde(default)]
    pub data_source_config: Option<Value>,
    #[serde(default)]
    pub refresh_cron: String,
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct ChartUpdate {
    pub title: Option<String>,
    pub intent: Option<String>,
    pub kind: Option<String>,
    pub data: Option<Vec<Value>>,
    pub columns: Option<Vec<String>>,
    pub x_field: Option<String>,
    pub y_fields: Option<Vec<String>>,
    /// Partial options object merged into the existing chart options.
    pub options: Option<Value>,
    pub data_source_type: Option<String>,
    pub sql_query: Option<String>,
    pub sql_limit: Option<usize>,
    pub http_url: Option<String>,
    pub http_data_path: Option<String>,
    pub render_config: Option<Value>,
    pub data_source_config: Option<Value>,
    pub refresh_cron: Option<String>,
    pub status: Option<String>,
}

impl ChartUpdate {
    fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.intent.is_none()
            && self.kind.is_none()
            && self.data.is_none()
            && self.columns.is_none()
            && self.x_field.is_none()
            && self.y_fields.is_none()
            && self.options.is_none()
            && self.data_source_type.is_none()
            && self.sql_query.is_none()
            && self.sql_limit.is_none()
            && self.http_url.is_none()
            && self.http_data_path.is_none()
            && self.render_config.is_none()
            && self.data_source_config.is_none()
            && self.refresh_cron.is_none()
            && self.status.is_none()
    }
}

#[debug_handler]
async fn whoami(State(ctx): State<AppContext>, auth: AgentAuth) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    format::render()
        .header("cache-control", "no-store")
        .json(json!({
            "authenticated": auth.is_authenticated(),
            "actor": auth.label,
            "scopes": {
                "read": auth.scopes.can_read(),
                "write": auth.scopes.can_write(),
                "private_read": auth.can_read_private(),
            },
            "site": SiteView::from_settings(&settings),
            "agent_api": format!(
                "{}/api/agent",
                settings.site.base_url.trim_end_matches('/')
            ),
        }))
}

#[debug_handler]
async fn list_posts(
    State(ctx): State<AppContext>,
    auth: AgentAuth,
    Query(query): Query<PostsQuery>,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let site = SiteView::from_settings(&settings);
    let status = query.status.as_deref().unwrap_or(posts::STATUS_PUBLISHED);
    let private_listing = !matches!(status.trim(), "" | posts::STATUS_PUBLISHED);
    if private_listing {
        if let Some(response) = auth.private_read_error() {
            return Ok(response);
        }
    }
    let since = match parse_since(query.since.as_deref()) {
        Ok(since) => since,
        Err(message) => return Ok(bad_request(&message)),
    };
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let mut all = match posts::Model::list_for_agent(&ctx.db, status).await {
        Ok(all) => all,
        Err(error) => return Ok(model_error_response(error)),
    };
    if let Some(since) = since {
        if private_listing {
            all.retain(|post| post.updated_at >= since);
        } else {
            all.retain(|post| {
                post.published_at
                    .is_some_and(|published| published >= since)
            });
        }
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
    let cache_control = if private_listing {
        "private, no-store"
    } else {
        "public, max-age=60, stale-while-revalidate=300"
    };
    let mut builder = format::render()
        .header("cache-control", cache_control)
        .etag(&etag)?;
    if let Some(latest) = latest {
        builder = builder.header("last-modified", util::http_date(&latest));
    }
    builder.json(response)
}

#[debug_handler]
async fn post_detail(
    State(ctx): State<AppContext>,
    auth: AgentAuth,
    Path(slug): Path<String>,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let post = if auth.can_read_private() {
        posts::Model::find_by_slug(&ctx.db, &slug).await?
    } else {
        posts::Model::find_published_by_slug(&ctx.db, &slug).await?
    };
    let detail = post_detail_response(&ctx, &settings, &post, auth.is_authenticated()).await?;
    let etag = util::etag_for_str(&serde_json::to_string(&detail)?);
    let cache_control = if auth.can_read_private() {
        "private, no-store"
    } else {
        "public, max-age=60, stale-while-revalidate=300"
    };
    format::render()
        .header("cache-control", cache_control)
        .header("last-modified", util::http_date(&post.updated_at))
        .etag(&etag)?
        .json(detail)
}

#[debug_handler]
async fn create_post(
    State(ctx): State<AppContext>,
    auth: AgentAuth,
    Json(input): Json<PostCreate>,
) -> Result<Response> {
    if let Some(response) = auth.write_error() {
        return Ok(response);
    }
    let status = if input.status.trim().is_empty() {
        posts::STATUS_DRAFT.to_string()
    } else {
        input.status.clone()
    };
    let params = posts::PostParams {
        title: input.title,
        slug: input.slug,
        summary: input.summary,
        body_md: input.body_md,
        status,
        pinned: input.pinned,
        cover_image: input.cover_image,
        tags: input.tags,
    };
    match posts::Model::create_post(&ctx.db, &params).await {
        Ok(post) => {
            tracing::info!(actor = %auth.label, post_id = post.id, "agent created post");
            after_post_write(&ctx, &post).await;
            let settings = Settings::from_context(&ctx)?;
            let detail = post_detail_response(&ctx, &settings, &post, true).await?;
            format::render()
                .status(StatusCode::CREATED)
                .header("cache-control", "no-store")
                .json(detail)
        }
        Err(error) => Ok(model_error_response(error)),
    }
}

#[debug_handler]
async fn update_post(
    State(ctx): State<AppContext>,
    auth: AgentAuth,
    Path(slug): Path<String>,
    Json(input): Json<posts::PostPatch>,
) -> Result<Response> {
    if let Some(response) = auth.write_error() {
        return Ok(response);
    }
    if input.is_empty() {
        return Ok(bad_request("no fields to update"));
    }
    let post = posts::Model::find_by_slug(&ctx.db, &slug).await?;
    match post.patch_post(&ctx.db, &input).await {
        Ok(post) => {
            tracing::info!(actor = %auth.label, post_id = post.id, "agent updated post");
            after_post_write(&ctx, &post).await;
            let settings = Settings::from_context(&ctx)?;
            let detail = post_detail_response(&ctx, &settings, &post, true).await?;
            format::render()
                .header("cache-control", "no-store")
                .json(detail)
        }
        Err(posts::PostWriteError::Conflict) => {
            let current = posts::Model::find_by_id(&ctx.db, post.id).await?;
            Ok(conflict_response(&current))
        }
        Err(posts::PostWriteError::Model(error)) => Ok(model_error_response(error)),
    }
}

#[debug_handler]
async fn delete_post(
    State(ctx): State<AppContext>,
    auth: AgentAuth,
    Path(slug): Path<String>,
) -> Result<Response> {
    if let Some(response) = auth.write_error() {
        return Ok(response);
    }
    let post = posts::Model::find_by_slug(&ctx.db, &slug).await?;
    match post.delete_post(&ctx.db).await {
        Ok(()) => {
            tracing::warn!(actor = %auth.label, post_id = post.id, "agent deleted post");
            crate::controllers::llms::invalidate(&ctx).await;
            format::render().status(StatusCode::NO_CONTENT).empty()
        }
        Err(error) => Ok(model_error_response(error)),
    }
}

#[debug_handler]
async fn list_post_charts(
    State(ctx): State<AppContext>,
    auth: AgentAuth,
    Path(slug): Path<String>,
) -> Result<Response> {
    if let Some(response) = auth.private_read_error() {
        return Ok(response);
    }
    let settings = Settings::from_context(&ctx)?;
    let post = posts::Model::find_by_slug(&ctx.db, &slug).await?;
    let base = settings.site.base_url.trim_end_matches('/');
    let charts = chart_specs::Model::all_for_post(&ctx.db, post.id).await?;
    let items = charts
        .iter()
        .map(|chart| AgentChartRef::from((chart, base)))
        .collect::<Vec<_>>();
    format::render()
        .header("cache-control", "no-store")
        .json(json!({
            "post_id": post.id,
            "slug": post.slug,
            "count": items.len(),
            "charts": items,
        }))
}

#[debug_handler]
async fn create_chart(
    State(ctx): State<AppContext>,
    auth: AgentAuth,
    Path(slug): Path<String>,
    Json(input): Json<ChartWrite>,
) -> Result<Response> {
    if let Some(response) = auth.write_error() {
        return Ok(response);
    }
    let settings = Settings::from_context(&ctx)?;
    let post = posts::Model::find_by_slug(&ctx.db, &slug).await?;
    let params = match chart_params_from_write(&input, &settings) {
        Ok(params) => params,
        Err(message) => return Ok(bad_request(&message)),
    };
    match chart_specs::Model::create_chart(&ctx.db, post.id, &params).await {
        Ok(chart) => {
            tracing::info!(
                actor = %auth.label,
                chart_id = chart.id,
                post_id = post.id,
                "agent created chart"
            );
            let chart = refresh_after_write(&ctx, chart).await;
            let detail = chart_detail_response(&ctx, &settings, &chart).await?;
            format::render()
                .status(StatusCode::CREATED)
                .header("cache-control", "no-store")
                .json(detail)
        }
        Err(error) => Ok(model_error_response(error)),
    }
}

#[debug_handler]
async fn update_chart(
    State(ctx): State<AppContext>,
    auth: AgentAuth,
    Path(id): Path<i64>,
    Json(input): Json<ChartUpdate>,
) -> Result<Response> {
    if let Some(response) = auth.write_error() {
        return Ok(response);
    }
    if input.is_empty() {
        return Ok(bad_request("no fields to update"));
    }
    let settings = Settings::from_context(&ctx)?;
    let chart = chart_specs::Model::find_by_id(&ctx.db, id).await?;
    let params = match chart_params_from_update(&chart, &input, &settings) {
        Ok(params) => params,
        Err(message) => return Ok(bad_request(&message)),
    };
    let new_source =
        serde_json::from_str::<Value>(&params.data_source_config).unwrap_or(Value::Null);
    let new_render = serde_json::from_str::<Value>(&params.render_config).unwrap_or(Value::Null);
    let data_changed = params.data_source_type != chart.data_source_type
        || new_source != chart.data_source_config
        || new_render != chart.render_config;
    match chart.update_chart(&ctx.db, &params).await {
        Ok(chart) => {
            tracing::info!(actor = %auth.label, chart_id = chart.id, "agent updated chart");
            let chart = if data_changed {
                refresh_after_write(&ctx, chart).await
            } else {
                chart
            };
            let detail = chart_detail_response(&ctx, &settings, &chart).await?;
            format::render()
                .header("cache-control", "no-store")
                .json(detail)
        }
        Err(error) => Ok(model_error_response(error)),
    }
}

#[debug_handler]
async fn delete_chart(
    State(ctx): State<AppContext>,
    auth: AgentAuth,
    Path(id): Path<i64>,
) -> Result<Response> {
    if let Some(response) = auth.write_error() {
        return Ok(response);
    }
    let chart = chart_specs::Model::find_by_id(&ctx.db, id).await?;
    match chart.delete_chart(&ctx.db).await {
        Ok(()) => {
            tracing::warn!(actor = %auth.label, chart_id = chart.id, "agent deleted chart");
            format::render().status(StatusCode::NO_CONTENT).empty()
        }
        Err(error) => Ok(model_error_response(error)),
    }
}

#[debug_handler]
async fn refresh_chart(
    State(ctx): State<AppContext>,
    auth: AgentAuth,
    Path(id): Path<i64>,
) -> Result<Response> {
    if let Some(response) = auth.write_error() {
        return Ok(response);
    }
    let settings = Settings::from_context(&ctx)?;
    let chart = chart_specs::Model::find_by_id(&ctx.db, id).await?;
    match charts::refresh_chart(&ctx, chart.id).await {
        Ok(chart) => {
            tracing::info!(actor = %auth.label, chart_id = chart.id, "agent refreshed chart");
            let detail = chart_detail_response(&ctx, &settings, &chart).await?;
            format::render()
                .header("cache-control", "no-store")
                .json(detail)
        }
        // Keep the last successful payload: a transient source failure should
        // not blank a published chart. GET will keep serving it as stale.
        Err(error) => Ok(agent_error(
            StatusCode::BAD_GATEWAY,
            "chart_refresh_failed",
            &error.to_string(),
        )),
    }
}

#[debug_handler]
async fn chart_detail(
    State(ctx): State<AppContext>,
    auth: AgentAuth,
    Path(id): Path<i64>,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let chart = chart_specs::Model::find_by_id(&ctx.db, id).await?;
    if chart.status == chart_specs::STATUS_PAUSED && !auth.can_read_private() {
        return not_found();
    }
    let post = posts::Model::find_by_id(&ctx.db, chart.post_id).await?;
    if post.status != posts::STATUS_PUBLISHED && !auth.can_read_private() {
        return not_found();
    }
    let cache = charts::cached(&ctx.db, chart.id).await?;
    let (data, cache_etag, updated_at, stale, last_modified) = if let Some(cache) = &cache {
        (
            cache.payload.clone(),
            cache.etag.clone(),
            cache.fetched_at.to_rfc3339(),
            !cache.is_fresh(),
            cache.fetched_at,
        )
    } else {
        (
            Value::Null,
            String::new(),
            String::new(),
            true,
            chart.updated_at,
        )
    };
    // The cache ETag alone does not change when only the spec/title/status is
    // updated, so include the chart row timestamp and the auth mode (the
    // anonymous body omits `data_source`).
    let etag = util::etag_for_str(&format!(
        "chart:{}:{}:{}:{}",
        chart.id,
        chart.updated_at.to_rfc3339(),
        cache_etag,
        auth.can_read_private()
    ));
    let last_modified = last_modified.max(chart.updated_at);
    if stale && !settings.charts.manual_refresh_only {
        if let Err(error) = ChartWorker::perform_later(&ctx, ChartArgs { chart_id: chart.id }).await
        {
            tracing::warn!(chart_id = chart.id, error = %error, "agent chart refresh enqueue failed");
        }
    }
    let response = AgentChartResponse {
        chart_id: chart.id,
        post_id: chart.post_id,
        title: chart.title.clone(),
        kind: chart.kind.clone(),
        source_type: chart.data_source_type.clone(),
        status: chart.status.clone(),
        marker: format!("[[chart:{}]]", chart.id),
        spec: chart.render_config.clone(),
        data_source: if auth.can_read_private() {
            chart.data_source_config.clone()
        } else {
            Value::Null
        },
        data,
        updated_at,
        etag: etag.clone(),
        stale,
    };
    let cache_control = if auth.can_read_private() {
        "private, no-store"
    } else {
        "public, max-age=60, stale-while-revalidate=600"
    };
    format::render()
        .header("cache-control", cache_control)
        .header("vary", "authorization")
        .header("x-chart-stale", if stale { "1" } else { "0" })
        .header("last-modified", util::http_date(&last_modified))
        .etag(&etag)?
        .json(response)
}

#[debug_handler]
async fn list_media(
    State(ctx): State<AppContext>,
    auth: AgentAuth,
    Query(query): Query<MediaQuery>,
) -> Result<Response> {
    if let Some(response) = auth.private_read_error() {
        return Ok(response);
    }
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let items = medias::Model::search(
        &ctx.db,
        query.q.as_deref().unwrap_or_default(),
        query.kind.as_deref(),
        limit,
    )
    .await?;
    let items = items.iter().map(media_json).collect::<Vec<_>>();
    format::render()
        .header("cache-control", "no-store")
        .json(json!({"count": items.len(), "items": items}))
}

#[debug_handler]
async fn upload_media(
    State(ctx): State<AppContext>,
    auth: AgentAuth,
    mut multipart: Multipart,
) -> Result<Response> {
    if let Some(response) = auth.write_error() {
        return Ok(response);
    }
    let mut upload = None;
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(err) => {
                return Ok(bad_request(&format!("读取上传文件失败：{err}")));
            }
        };
        if field.name() != Some("file") {
            continue;
        }
        let filename = field
            .file_name()
            .map(ToString::to_string)
            .unwrap_or_else(|| "upload".to_string());
        let bytes = match field.bytes().await {
            Ok(bytes) => bytes,
            Err(err) => {
                return Ok(bad_request(&format!("读取上传文件失败：{err}")));
            }
        };
        if bytes.len() > media::MAX_UPLOAD_BYTES {
            return Ok(bad_request("文件太大：图片最大 8 MB，视频最大 12 MB"));
        }
        upload = Some((filename, bytes));
        break;
    }
    let Some((filename, bytes)) = upload else {
        return Ok(bad_request(
            "没有收到文件；请用 multipart/form-data 的 file 字段上传",
        ));
    };
    if let Err(error) = media::inspect(&filename, &bytes) {
        return Ok(bad_request(&error.to_string()));
    }
    let item = media::store(&ctx.db, &filename, &bytes).await?;
    tracing::info!(actor = %auth.label, media_id = item.id, "agent uploaded media");
    format::render()
        .status(StatusCode::CREATED)
        .header("cache-control", "no-store")
        .json(media_json(&item))
}

fn media_json(item: &medias::Model) -> Value {
    let url = media::public_url(&item.filename);
    let markdown = if item.kind == "image" {
        format!("![{}]({url})", media::markdown_alt(&item.original_name))
    } else {
        media::video_marker(&item.filename)
    };
    json!({
        "id": item.id,
        "url": url,
        "filename": item.filename,
        "original_name": item.original_name,
        "kind": item.kind,
        "mime": item.mime,
        "size": item.size,
        "markdown": markdown,
        "video_marker": media::video_marker(&item.filename),
        "created_at": item.created_at.to_rfc3339(),
    })
}

async fn post_detail_response(
    ctx: &AppContext,
    settings: &Settings,
    post: &posts::Model,
    include_paused_charts: bool,
) -> Result<AgentPostDetail> {
    let tags = post.tags(&ctx.db).await?;
    let base = settings.site.base_url.trim_end_matches('/');
    let charts = if include_paused_charts {
        chart_specs::Model::all_for_post(&ctx.db, post.id).await?
    } else {
        chart_specs::Model::for_post(&ctx.db, post.id).await?
    };
    Ok(AgentPostDetail {
        post: AgentPostSummary::from_model(post, &tags, base),
        body_md: post.body_md.clone(),
        markdown: markdown_document(post, &tags),
        charts: charts.iter().map(|chart| (chart, base).into()).collect(),
    })
}

async fn chart_detail_response(
    ctx: &AppContext,
    settings: &Settings,
    chart: &chart_specs::Model,
) -> Result<AgentChartDetail> {
    let cache = charts::cached(&ctx.db, chart.id).await?;
    let (data, stale) = cache.as_ref().map_or((Value::Null, true), |cache| {
        (cache.payload.clone(), !cache.is_fresh())
    });
    let base = settings.site.base_url.trim_end_matches('/');
    Ok(AgentChartDetail {
        chart_id: chart.id,
        post_id: chart.post_id,
        title: chart.title.clone(),
        kind: chart.kind.clone(),
        source_type: chart.data_source_type.clone(),
        status: chart.status.clone(),
        refresh_cron: chart.refresh_cron.clone().unwrap_or_default(),
        spec: chart.render_config.clone(),
        data_source: chart.data_source_config.clone(),
        data,
        stale,
        marker: format!("[[chart:{}]]", chart.id),
        data_url: format!("{base}/api/agent/charts/{}", chart.id),
        updated_at: chart.updated_at.to_rfc3339(),
    })
}

async fn after_post_write(ctx: &AppContext, post: &posts::Model) {
    if post.status == posts::STATUS_PUBLISHED {
        if let Err(error) = IndexWorker::perform_later(ctx, IndexArgs { post_id: post.id }).await {
            tracing::warn!(post_id = post.id, error = %error, "agent post index enqueue failed");
        }
    }
    crate::controllers::llms::invalidate(ctx).await;
}

async fn refresh_after_write(ctx: &AppContext, chart: chart_specs::Model) -> chart_specs::Model {
    if chart.status == chart_specs::STATUS_PAUSED {
        return chart;
    }
    if chart.data_source_type == "static" {
        if let Err(error) =
            crate::models::chart_data_caches::Model::delete_for_chart(&ctx.db, chart.id).await
        {
            tracing::warn!(chart_id = chart.id, error = %error, "agent chart cache invalidation failed");
        }
        match charts::refresh_chart(ctx, chart.id).await {
            Ok(chart) => chart,
            Err(error) => {
                tracing::warn!(chart_id = chart.id, error = %error, "agent static chart refresh failed");
                chart
            }
        }
    } else {
        if let Err(error) =
            crate::models::chart_data_caches::Model::delete_for_chart(&ctx.db, chart.id).await
        {
            tracing::warn!(chart_id = chart.id, error = %error, "agent chart cache invalidation failed");
        }
        if let Err(error) = ChartWorker::perform_later(ctx, ChartArgs { chart_id: chart.id }).await
        {
            tracing::warn!(chart_id = chart.id, error = %error, "agent chart refresh enqueue failed");
        }
        chart
    }
}

fn chart_params_from_write(
    input: &ChartWrite,
    settings: &Settings,
) -> std::result::Result<chart_specs::ChartParams, String> {
    if input.title.trim().is_empty() {
        return Err("title is required".to_string());
    }
    match (&input.render_config, &input.data_source_config) {
        (Some(render), Some(source)) => Ok(chart_specs::ChartParams {
            title: input.title.clone(),
            intent: input.intent.clone(),
            kind: if input.kind.trim().is_empty() {
                "bar".to_string()
            } else {
                input.kind.clone()
            },
            data_source_type: input
                .data_source_type
                .clone()
                .unwrap_or_else(|| "static".to_string()),
            data_source_config: source.to_string(),
            render_config: render.to_string(),
            refresh_cron: input.refresh_cron.clone(),
            status: input.status.clone(),
        }),
        (None, None) => {
            let source_type = input
                .data_source_type
                .clone()
                .unwrap_or_else(|| "static".to_string());
            let visual = VisualChartInput {
                kind: if input.kind.trim().is_empty() {
                    "bar".to_string()
                } else {
                    input.kind.clone()
                },
                x_field: input.x_field.clone(),
                y_fields: input.y_fields.clone(),
                options: input.options.clone().unwrap_or_default(),
                columns: input.columns.clone(),
                rows: input.data.clone(),
                source_type,
                sql_query: input.sql_query.clone().unwrap_or_default(),
                sql_limit: input.sql_limit,
                http_url: input.http_url.clone().unwrap_or_default(),
                http_data_path: input.http_data_path.clone().unwrap_or_default(),
            };
            blueprint::params_from_visual(
                &input.title,
                &input.intent,
                &input.refresh_cron,
                &input.status,
                visual,
                &settings.charts.sql_allowed_tables,
            )
            .map_err(|error| error.to_string())
        }
        _ => Err(
            "send render_config and data_source_config together, or neither for blueprint mode"
                .to_string(),
        ),
    }
}

fn chart_params_from_update(
    chart: &chart_specs::Model,
    input: &ChartUpdate,
    settings: &Settings,
) -> std::result::Result<chart_specs::ChartParams, String> {
    let title = input.title.clone().unwrap_or_else(|| chart.title.clone());
    if title.trim().is_empty() {
        return Err("title is required".to_string());
    }
    let intent = input
        .intent
        .clone()
        .unwrap_or_else(|| chart.intent.clone().unwrap_or_default());
    let refresh_cron = input
        .refresh_cron
        .clone()
        .unwrap_or_else(|| chart.refresh_cron.clone().unwrap_or_default());
    let status = input.status.clone().unwrap_or_else(|| chart.status.clone());
    let advanced = input.render_config.is_some() || input.data_source_config.is_some();
    if advanced {
        if chart.data_source_config.get("blueprint").is_some()
            && (input.render_config.is_none() || input.data_source_config.is_none())
        {
            return Err(
                "converting a blueprint chart to advanced mode requires both render_config and data_source_config"
                    .to_string(),
            );
        }
        if input.data.is_some()
            || input.columns.is_some()
            || input.x_field.is_some()
            || input.y_fields.is_some()
            || input.options.is_some()
            || input.kind.is_some()
            || input.sql_query.is_some()
            || input.sql_limit.is_some()
            || input.http_url.is_some()
            || input.http_data_path.is_some()
        {
            return Err(
                "advanced render_config/data_source_config cannot be mixed with blueprint fields"
                    .to_string(),
            );
        }
        return Ok(chart_specs::ChartParams {
            title,
            intent,
            kind: chart.kind.clone(),
            data_source_type: input
                .data_source_type
                .clone()
                .unwrap_or_else(|| chart.data_source_type.clone()),
            data_source_config: input
                .data_source_config
                .clone()
                .unwrap_or_else(|| chart.data_source_config.clone())
                .to_string(),
            render_config: input
                .render_config
                .clone()
                .unwrap_or_else(|| chart.render_config.clone())
                .to_string(),
            refresh_cron,
            status,
        });
    }

    let visual_update = input.data.is_some()
        || input.columns.is_some()
        || input.x_field.is_some()
        || input.y_fields.is_some()
        || input.options.is_some()
        || input.kind.is_some()
        || input.data_source_type.is_some()
        || input.sql_query.is_some()
        || input.sql_limit.is_some()
        || input.http_url.is_some()
        || input.http_data_path.is_some();
    if visual_update {
        let mut visual = visual_from_chart(chart)?;
        if let Some(kind) = &input.kind {
            visual.kind.clone_from(kind);
        }
        if let Some(x_field) = &input.x_field {
            visual.x_field.clone_from(x_field);
        }
        if let Some(y_fields) = &input.y_fields {
            visual.y_fields.clone_from(y_fields);
        }
        if let Some(options) = &input.options {
            visual.options = merge_chart_options(&visual.options, options)?;
        }
        if let Some(columns) = &input.columns {
            visual.columns.clone_from(columns);
        }
        if let Some(data) = &input.data {
            visual.rows.clone_from(data);
        }
        if let Some(source_type) = &input.data_source_type {
            visual.source_type.clone_from(source_type);
        }
        if let Some(sql_query) = &input.sql_query {
            visual.sql_query.clone_from(sql_query);
        }
        if input.sql_limit.is_some() {
            visual.sql_limit = input.sql_limit;
        }
        if let Some(http_url) = &input.http_url {
            visual.http_url.clone_from(http_url);
        }
        if let Some(http_data_path) = &input.http_data_path {
            visual.http_data_path.clone_from(http_data_path);
        }
        return blueprint::params_from_visual(
            &title,
            &intent,
            &refresh_cron,
            &status,
            visual,
            &settings.charts.sql_allowed_tables,
        )
        .map_err(|error| error.to_string());
    }

    Ok(chart_specs::ChartParams {
        title,
        intent,
        kind: chart.kind.clone(),
        data_source_type: chart.data_source_type.clone(),
        data_source_config: chart.data_source_config.to_string(),
        render_config: chart.render_config.to_string(),
        refresh_cron,
        status,
    })
}

fn merge_chart_options(
    existing: &ChartOptions,
    patch: &Value,
) -> std::result::Result<ChartOptions, String> {
    let patch = patch
        .as_object()
        .ok_or_else(|| "options must be a JSON object".to_string())?;
    let mut merged = serde_json::to_value(existing)
        .map_err(|error| format!("could not read existing options: {error}"))?;
    let Some(merged) = merged.as_object_mut() else {
        return Err("could not read existing options".to_string());
    };
    for (key, value) in patch {
        merged.insert(key.clone(), value.clone());
    }
    serde_json::from_value(Value::Object(merged.clone()))
        .map_err(|error| format!("invalid chart options: {error}"))
}

fn visual_from_chart(chart: &chart_specs::Model) -> std::result::Result<VisualChartInput, String> {
    let config = &chart.data_source_config;
    let blueprint = config.get("blueprint").ok_or_else(|| {
        "this chart was created with raw render_config; send render_config/data_source_config to update it"
            .to_string()
    })?;
    let options = blueprint
        .get("options")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default();
    let mut columns = config
        .get("columns")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if columns.is_empty() {
        // Charts saved before `columns` was persisted (and SQL/HTTP charts in
        // older rows) can still be edited from their x/y blueprint metadata.
        if let Some(x_field) = blueprint.get("x_field").and_then(Value::as_str) {
            if !x_field.is_empty() {
                columns.push(x_field.to_string());
            }
        }
        if let Some(y_fields) = blueprint.get("y_fields").and_then(Value::as_array) {
            for field in y_fields.iter().filter_map(Value::as_str) {
                if !columns.iter().any(|column| column == field) {
                    columns.push(field.to_string());
                }
            }
        }
    }
    let rows = config
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(VisualChartInput {
        kind: blueprint
            .get("kind")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| chart.kind.clone()),
        x_field: blueprint
            .get("x_field")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        y_fields: blueprint
            .get("y_fields")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        options,
        columns,
        rows,
        source_type: chart.data_source_type.clone(),
        sql_query: config
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        sql_limit: config
            .get("limit")
            .and_then(Value::as_u64)
            .map(|value| usize::try_from(value).unwrap_or(500)),
        http_url: config
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        http_data_path: config
            .get("data_path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    })
}

fn bad_request(message: &str) -> Response {
    agent_error(StatusCode::BAD_REQUEST, "bad_request", message)
}

fn conflict_response(post: &posts::Model) -> Response {
    (
        StatusCode::CONFLICT,
        Json(json!({
            "error": "conflict",
            "message": "post changed since it was read; re-fetch it and retry",
            "current_updated_at": post.updated_at.to_rfc3339(),
        })),
    )
        .into_response()
}

fn model_error_response(error: ModelError) -> Response {
    match error {
        ModelError::EntityNotFound => {
            agent_error(StatusCode::NOT_FOUND, "not_found", "resource was not found")
        }
        ModelError::EntityAlreadyExists => agent_error(
            StatusCode::CONFLICT,
            "conflict",
            "a resource with that slug already exists",
        ),
        ModelError::Validation(error) => agent_error(
            StatusCode::BAD_REQUEST,
            "validation_error",
            &error.to_string(),
        ),
        ModelError::Message(message) => {
            agent_error(StatusCode::BAD_REQUEST, "bad_request", &message)
        }
        other => {
            tracing::error!(error = %other, "agent write failed");
            agent_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "internal server error",
            )
        }
    }
}

fn parse_since(
    value: Option<&str>,
) -> std::result::Result<Option<DateTime<chrono::FixedOffset>>, String> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if let Ok(dt) = DateTime::parse_from_rfc3339(value) {
        return Ok(Some(dt));
    }
    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        let dt = date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| "invalid since date".to_string())?;
        return Ok(Some(
            DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc).fixed_offset(),
        ));
    }
    Err("since must be RFC3339 or YYYY-MM-DD".to_string())
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/agent")
        .add("/me", get(whoami))
        .add("/posts", get(list_posts).post(create_post))
        .add(
            "/posts/{slug}",
            get(post_detail).patch(update_post).delete(delete_post),
        )
        .add(
            "/posts/{slug}/charts",
            get(list_post_charts).post(create_chart),
        )
        .add(
            "/charts/{id}",
            get(chart_detail).patch(update_chart).delete(delete_chart),
        )
        .add("/charts/{id}/refresh", post(refresh_chart))
        .add("/media", get(list_media).post(upload_media))
}
