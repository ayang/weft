use std::collections::{BTreeMap, HashMap, HashSet};

use axum::{
    body::Body,
    extract::Path as AxumPath,
    http::{header, HeaderValue, StatusCode},
};
use loco_rs::prelude::*;
use serde::Deserialize;

use crate::{
    data::settings::Settings,
    models::{categories, chart_data_caches, chart_specs, post_tags, posts, tags},
    util,
    views::{
        agent::markdown_document,
        public::{
            ArchiveMonth, ArchivePageView, ArchiveYear, CategoryPageView, CategoryView, ChartView,
            HomeView, PostCard, PostDetail, PostPageView, SearchPageView, SiteView, TagPageView,
            TagView,
        },
    },
};

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
}

#[debug_handler]
async fn home(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let site = SiteView::from_settings(&settings);
    let posts = posts::Model::published(&ctx.db, settings.site.posts_per_page).await?;
    let pinned = posts::Model::pinned_published(&ctx.db).await?;
    let tags = posts::Model::tag_counts(&ctx.db, 24).await?;
    let tag_map = tags_for_posts(&ctx.db, &posts).await?;
    let mut all_for_categories = posts.clone();
    all_for_categories.extend(pinned.iter().cloned());
    let cat_map = category_map(&ctx, &all_for_categories).await?;
    let post_cards = posts
        .iter()
        .map(|post| {
            PostCard::from_model(post, tag_map.get(&post.id).cloned().unwrap_or_default())
                .with_category(cat_map.get(&post.id).cloned())
        })
        .collect::<Vec<_>>();
    let pinned_cards = pinned
        .iter()
        .map(|post| {
            PostCard::from_model(post, tag_map.get(&post.id).cloned().unwrap_or_default())
                .with_category(cat_map.get(&post.id).cloned())
        })
        .collect::<Vec<_>>();
    let category_models = categories::Model::all(&ctx.db).await?;
    let mut category_views = Vec::new();
    for category in &category_models {
        let count = posts::Model::count_published_in_category(&ctx.db, category.id).await?;
        category_views.push(CategoryView::from_model(
            category,
            i64::try_from(count).unwrap_or(0),
        ));
    }
    let total_posts = posts::Model::count_published(&ctx.db).await?;
    let view = HomeView {
        site,
        posts: post_cards,
        pinned: pinned_cards,
        tags: tags
            .into_iter()
            .map(|tag| TagView {
                name: tag.name,
                slug: tag.slug,
                count: tag.count,
            })
            .collect(),
        categories: category_views,
        total_posts,
    };
    let etag = util::etag_for_str(&serde_json::to_string(&view)?);
    let latest = posts
        .iter()
        .chain(pinned.iter())
        .map(|post| post.updated_at)
        .max();
    let mut response = format::render()
        .header(
            "cache-control",
            "public, max-age=30, stale-while-revalidate=60",
        )
        .etag(&etag)?;
    if let Some(latest) = latest {
        response = response.header("last-modified", util::http_date(&latest));
    }
    response.view(&v, "home.html", data!({ "view": view }))
}

#[debug_handler]
async fn post_page(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
    Path(slug): Path<String>,
) -> Result<Response> {
    if let Some(markdown_slug) = slug.strip_suffix(".md") {
        return post_markdown(&ctx, markdown_slug).await;
    }
    let settings = Settings::from_context(&ctx)?;
    let site = SiteView::from_settings(&settings);
    let post = posts::Model::find_published_by_slug(&ctx.db, &slug).await?;
    let post_tags_models = post.tags(&ctx.db).await?;
    let charts = chart_specs::Model::for_post(&ctx.db, post.id).await?;
    let mut chart_views = Vec::new();
    for chart in &charts {
        let cache = chart_data_caches::Model::find_for_chart(&ctx.db, chart.id).await?;
        chart_views.push(ChartView::from_model(chart, cache.as_ref()));
    }
    let body_md = util::strip_duplicate_title(&post.body_md, &post.title);
    let (body_html, inline_charts) = render_body_with_charts(&v, &body_md, &chart_views)?;
    let end_charts = chart_views
        .into_iter()
        .filter(|chart| !inline_charts.contains(&chart.id))
        .collect::<Vec<_>>();
    let related = post.related(&ctx.db, 3).await?;
    let related_tags = tags_for_posts(&ctx.db, &related).await?;
    let related_categories = category_map(&ctx, &related).await?;
    let category_models = categories::Model::all(&ctx.db).await?;
    let category = post.category_id.and_then(|id| {
        category_models
            .iter()
            .find(|category| category.id == id)
            .map(|category| CategoryView::from_model(category, 0))
    });
    let detail = PostDetail {
        id: post.id,
        slug: post.slug.clone(),
        title: post.title.clone(),
        summary: post.summary.clone().unwrap_or_default(),
        date: post.published_at.map_or_else(
            || util::format_date(&post.created_at),
            |dt| util::format_date(&dt),
        ),
        updated: util::format_date(&post.updated_at),
        reading_minutes: post.reading_minutes.max(1),
        tags: post_tags_models.iter().map(TagView::from).collect(),
        cover_image: post.cover_image.clone().unwrap_or_default(),
        markdown_url: format!("/posts/{}.md", post.slug),
        category,
    };
    let view = PostPageView {
        site,
        post: detail,
        body_html,
        charts: end_charts,
        related: related
            .iter()
            .map(|related| {
                PostCard::from_model(
                    related,
                    related_tags.get(&related.id).cloned().unwrap_or_default(),
                )
                .with_category(related_categories.get(&related.id).cloned())
            })
            .collect(),
    };
    let etag = util::etag_for_str(&serde_json::to_string(&view)?);
    let mut response = format::render()
        .header(
            "cache-control",
            "public, max-age=30, stale-while-revalidate=60",
        )
        .etag(&etag)?;
    response = response.header("last-modified", util::http_date(&post.updated_at));
    response.view(&v, "post.html", data!({ "view": view }))
}

async fn post_markdown(ctx: &AppContext, slug: &str) -> Result<Response> {
    let post = posts::Model::find_published_by_slug(&ctx.db, slug).await?;
    let tags = post.tags(&ctx.db).await?;
    let body = markdown_document(&post, &tags);
    let etag = util::etag_for_str(&body);
    let mut response = format::render()
        .header(
            "cache-control",
            "public, max-age=60, stale-while-revalidate=300",
        )
        .header("last-modified", util::http_date(&post.updated_at))
        .etag(&etag)?
        .text(&body)?;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/markdown; charset=utf-8"),
    );
    Ok(response)
}

#[debug_handler]
async fn category_page(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
    Path(slug): Path<String>,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let site = SiteView::from_settings(&settings);
    let Some(category) = categories::Model::find_by_slug(&ctx.db, &slug).await? else {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("category not found"))
            .map_err(|err| Error::string(&err.to_string()));
    };
    let posts =
        posts::Model::published_in_category(&ctx.db, category.id, settings.site.posts_per_page)
            .await?;
    let tag_map = tags_for_posts(&ctx.db, &posts).await?;
    let cat_map = category_map(&ctx, &posts).await?;
    let count = posts::Model::count_published_in_category(&ctx.db, category.id).await?;
    let view = CategoryPageView {
        site,
        category: CategoryView::from_model(&category, i64::try_from(count).unwrap_or(0)),
        posts: posts
            .iter()
            .map(|post| {
                PostCard::from_model(post, tag_map.get(&post.id).cloned().unwrap_or_default())
                    .with_category(cat_map.get(&post.id).cloned())
            })
            .collect(),
    };
    let etag = util::etag_for_str(&serde_json::to_string(&view)?);
    format::render()
        .header("cache-control", "public, max-age=30")
        .etag(&etag)?
        .view(&v, "category.html", data!({ "view": view }))
}

#[debug_handler]
async fn tag_page(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
    Path(slug): Path<String>,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let site = SiteView::from_settings(&settings);
    let tag = tags::Model::find_by_slug(&ctx.db, &slug).await?;
    let posts = tag.published_posts(&ctx.db).await?;
    let tag_map = tags_for_posts(&ctx.db, &posts).await?;
    let cat_map = category_map(&ctx, &posts).await?;
    let view = TagPageView {
        site,
        tag: TagView::from(&tag),
        posts: posts
            .iter()
            .map(|post| {
                PostCard::from_model(post, tag_map.get(&post.id).cloned().unwrap_or_default())
                    .with_category(cat_map.get(&post.id).cloned())
            })
            .collect(),
    };
    let etag = util::etag_for_str(&serde_json::to_string(&view)?);
    format::render()
        .header("cache-control", "public, max-age=30")
        .etag(&etag)?
        .view(&v, "tag.html", data!({ "view": view }))
}

#[debug_handler]
async fn archive(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let site = SiteView::from_settings(&settings);
    let posts = posts::Model::all_published(&ctx.db).await?;
    let tag_map = tags_for_posts(&ctx.db, &posts).await?;
    let cat_map = category_map(&ctx, &posts).await?;
    let mut grouped: BTreeMap<String, BTreeMap<String, Vec<PostCard>>> = BTreeMap::new();
    for post in &posts {
        let date = post.published_at.unwrap_or(post.created_at);
        let year = date.format("%Y").to_string();
        let month = date.format("%m").to_string();
        grouped
            .entry(year)
            .or_default()
            .entry(month)
            .or_default()
            .push(
                PostCard::from_model(post, tag_map.get(&post.id).cloned().unwrap_or_default())
                    .with_category(cat_map.get(&post.id).cloned()),
            );
    }
    let years = grouped
        .into_iter()
        .rev()
        .map(|(year, months)| ArchiveYear {
            year,
            months: months
                .into_iter()
                .rev()
                .map(|(month, posts)| ArchiveMonth { month, posts })
                .collect(),
        })
        .collect();
    let view = ArchivePageView {
        site,
        years,
        total_posts: u64::try_from(posts.len()).unwrap_or(0),
    };
    let etag = util::etag_for_str(&serde_json::to_string(&view)?);
    format::render()
        .header("cache-control", "public, max-age=60")
        .etag(&etag)?
        .view(&v, "archive.html", data!({ "view": view }))
}

#[debug_handler]
async fn search_page(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
    Query(query): Query<SearchQuery>,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let site = SiteView::from_settings(&settings);
    let q = query.q.unwrap_or_default().trim().to_string();
    let posts = if q.is_empty() {
        Vec::new()
    } else {
        posts::Model::search_published(&ctx.db, &q, 30).await?
    };
    let tag_map = tags_for_posts(&ctx.db, &posts).await?;
    let cat_map = category_map(&ctx, &posts).await?;
    let view = SearchPageView {
        site,
        query: q.clone(),
        posts: posts
            .iter()
            .map(|post| {
                PostCard::from_model(post, tag_map.get(&post.id).cloned().unwrap_or_default())
                    .with_category(cat_map.get(&post.id).cloned())
            })
            .collect(),
        searched: !q.is_empty(),
    };
    format::render().header("cache-control", "no-store").view(
        &v,
        "search.html",
        data!({ "view": view }),
    )
}

#[debug_handler]
async fn health(State(ctx): State<AppContext>) -> Result<Response> {
    let db_ok = ctx
        .db
        .query_one_raw(sea_orm::Statement::from_string(
            ctx.db.get_database_backend(),
            "SELECT 1",
        ))
        .await
        .is_ok();
    let cache_ok = ctx.cache.ping().await.is_ok();
    let published = posts::Model::count_published(&ctx.db).await.unwrap_or(0);
    let charts = chart_specs::Entity::find()
        .count(&ctx.db)
        .await
        .unwrap_or(0);
    format::json(serde_json::json!({
        "ok": db_ok,
        "version": env!("CARGO_PKG_VERSION"),
        "time": chrono::Utc::now().to_rfc3339(),
        "database": if db_ok { "ok" } else { "unavailable" },
        "cache": if cache_ok { "ok" } else { "unavailable" },
        "published_posts": published,
        "charts": charts
    }))
}

#[debug_handler]
async fn robots(State(ctx): State<AppContext>) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let body = format!(
        "User-agent: *\nAllow: /\nSitemap: {}/sitemap.xml\n",
        settings.site.base_url.trim_end_matches('/')
    );
    text_response("text/plain; charset=utf-8", &body, "public, max-age=3600")
}

#[debug_handler]
async fn sitemap(State(ctx): State<AppContext>) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let base = settings.site.base_url.trim_end_matches('/');
    let posts = posts::Model::all_published(&ctx.db).await?;
    let tags = posts::Model::tag_counts(&ctx.db, 200).await?;
    let mut body = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    body.push_str("<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n");
    body.push_str(&format!("  <url><loc>{base}/</loc></url>\n"));
    body.push_str(&format!("  <url><loc>{base}/archive</loc></url>\n"));
    body.push_str(&format!("  <url><loc>{base}/llms.txt</loc></url>\n"));
    for post in &posts {
        let lastmod = post.updated_at.format("%Y-%m-%d").to_string();
        body.push_str(&format!(
            "  <url><loc>{base}/posts/{}</loc><lastmod>{lastmod}</lastmod></url>\n",
            post.slug
        ));
        body.push_str(&format!(
            "  <url><loc>{base}/posts/{}.md</loc><lastmod>{lastmod}</lastmod></url>\n",
            post.slug
        ));
    }
    for tag in &tags {
        body.push_str(&format!(
            "  <url><loc>{base}/tags/{}</loc></url>\n",
            tag.slug
        ));
    }
    body.push_str("</urlset>\n");
    text_response(
        "application/xml; charset=utf-8",
        &body,
        "public, max-age=300",
    )
}

fn parse_video_marker(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let inner = trimmed.strip_prefix("[[video:")?.strip_suffix("]]")?;
    let filename = inner.trim();
    crate::media::safe_filename(filename).then_some(filename)
}

fn render_video(filename: &str) -> String {
    format!(
        "<figure class=\"media-video\"><video controls preload=\"metadata\" playsinline src=\"/media/{}\"></video></figure>",
        filename
    )
}

fn parse_chart_marker(line: &str) -> Option<i64> {
    let trimmed = line.trim();
    let inner = trimmed.strip_prefix("[[chart:")?.strip_suffix("]]")?;
    inner.trim().parse().ok()
}

fn render_body_with_charts(
    v: &TeraView,
    body: &str,
    charts: &[ChartView],
) -> Result<(String, HashSet<i64>)> {
    let by_id = charts
        .iter()
        .map(|chart| (chart.id, chart))
        .collect::<HashMap<_, _>>();
    let mut used = HashSet::new();
    let mut output = String::new();
    let mut segment = String::new();
    let mut in_fence = false;
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            segment.push_str(line);
            segment.push('\n');
            continue;
        }
        if !in_fence {
            if let Some(filename) = parse_video_marker(line) {
                if !segment.trim().is_empty() {
                    output.push_str(&util::render_markdown(&segment));
                    segment.clear();
                }
                output.push_str(&render_video(filename));
                continue;
            }
            if let Some(chart_id) = parse_chart_marker(line) {
                if !segment.trim().is_empty() {
                    output.push_str(&util::render_markdown(&segment));
                    segment.clear();
                }
                if let Some(chart) = by_id.get(&chart_id) {
                    output.push_str(&v.render("partials/chart.html", data!({"chart": chart}))?);
                    used.insert(chart_id);
                }
                continue;
            }
        }
        segment.push_str(line);
        segment.push('\n');
    }
    if !segment.trim().is_empty() {
        output.push_str(&util::render_markdown(&segment));
    }
    Ok((output, used))
}

async fn media_file(AxumPath(filename): AxumPath<String>) -> Result<Response> {
    if !crate::media::safe_filename(&filename) {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .map_err(|err| Error::string(&err.to_string()));
    }
    let path = crate::media::upload_dir().join(&filename);
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::empty())
                .map_err(|err| Error::string(&err.to_string()));
        }
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            crate::media::mime_for_filename(&filename),
        )
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .body(Body::from(bytes))
        .map_err(|err| Error::string(&err.to_string()))
}

fn text_response(
    content_type: &'static str,
    body: &str,
    cache_control: &'static str,
) -> Result<Response> {
    let etag = util::etag_for_str(body);
    let mut response = format::render()
        .header("cache-control", cache_control)
        .etag(&etag)?
        .text(body)?;
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    Ok(response)
}

async fn category_map(
    ctx: &AppContext,
    posts: &[posts::Model],
) -> ModelResult<HashMap<i64, CategoryView>> {
    let categories = categories::Model::all(&ctx.db).await?;
    let by_id = categories
        .iter()
        .map(|category| (category.id, category))
        .collect::<HashMap<_, _>>();
    Ok(posts
        .iter()
        .filter_map(|post| {
            post.category_id.and_then(|id| {
                by_id
                    .get(&id)
                    .map(|category| (post.id, CategoryView::from_model(category, 0)))
            })
        })
        .collect())
}

async fn tags_for_posts(
    db: &DatabaseConnection,
    posts: &[posts::Model],
) -> Result<HashMap<i64, Vec<tags::Model>>> {
    if posts.is_empty() {
        return Ok(HashMap::new());
    }
    let post_ids = posts.iter().map(|post| post.id).collect::<Vec<_>>();
    let links = post_tags::Entity::find()
        .filter(post_tags::Column::PostId.is_in(post_ids))
        .all(db)
        .await?;
    let tag_ids = links.iter().map(|link| link.tag_id).collect::<Vec<_>>();
    if tag_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let tag_rows = tags::Entity::find()
        .filter(tags::Column::Id.is_in(tag_ids))
        .all(db)
        .await?;
    let by_id = tag_rows
        .into_iter()
        .map(|tag| (tag.id, tag))
        .collect::<HashMap<_, _>>();
    let mut out: HashMap<i64, Vec<tags::Model>> = HashMap::new();
    for link in links {
        if let Some(tag) = by_id.get(&link.tag_id) {
            out.entry(link.post_id).or_default().push(tag.clone());
        }
    }
    Ok(out)
}

pub fn routes() -> Routes {
    Routes::new()
        .add("/", get(home))
        .add("/posts/{slug}", get(post_page))
        .add("/tags/{slug}", get(tag_page))
        .add("/categories/{slug}", get(category_page))
        .add("/archive", get(archive))
        .add("/search", get(search_page))
        .add("/media/{filename}", get(media_file))
        .add("/health", get(health))
        .add("/robots.txt", get(robots))
        .add("/sitemap.xml", get(sitemap))
}
