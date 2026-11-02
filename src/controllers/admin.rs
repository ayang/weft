use axum::{extract::Multipart, http::StatusCode};
use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    ai,
    charts::{
        self,
        blueprint::{self, ChartBlueprint, ChartBootstrap, VisualChartInput},
        parse,
    },
    data::settings::Settings,
    materials, media,
    models::{
        ai_sessions, ai_settings, categories, chart_data_caches, chart_recipes, chart_specs,
        medias, posts, site_settings,
    },
    security::{clear_admin_cookie, issue_admin_cookie, AdminSession, Limits},
    util, web,
    workers::{
        index_post::{Worker as IndexWorker, WorkerArgs as IndexArgs},
        refresh_chart::{Worker as ChartWorker, WorkerArgs as ChartArgs},
    },
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoginForm {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct PostForm {
    pub title: String,
    pub slug: String,
    pub summary: String,
    pub body_md: String,
    #[serde(default)]
    pub tags: String,
    pub status: String,
    #[serde(default)]
    pub pinned: Option<String>,
    #[serde(default)]
    pub cover_image: String,
}

impl PostForm {
    fn tags(&self) -> Vec<String> {
        self.tags
            .split([',', '，'])
            .map(str::trim)
            .filter(|tag| !tag.is_empty())
            .map(str::to_string)
            .collect()
    }

    fn to_params(&self) -> posts::PostParams {
        posts::PostParams {
            title: self.title.clone(),
            slug: self.slug.clone(),
            summary: self.summary.clone(),
            body_md: self.body_md.clone(),
            status: if self.status.is_empty() {
                posts::STATUS_DRAFT.to_string()
            } else {
                self.status.clone()
            },
            pinned: self.pinned.is_some(),
            cover_image: self.cover_image.clone(),
            tags: self.tags(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ChartForm {
    pub post_id: i64,
    pub title: String,
    pub intent: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub data_source_type: String,
    #[serde(default)]
    pub data_source_config: String,
    #[serde(default)]
    pub render_config: String,
    #[serde(default)]
    pub refresh_cron: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub visual_mode: Option<String>,
    #[serde(default)]
    pub advanced_mode: Option<String>,
    #[serde(default)]
    pub visual_config: String,
    #[serde(default)]
    pub sql_query: String,
    #[serde(default)]
    pub sql_limit: String,
    #[serde(default)]
    pub http_url: String,
    #[serde(default)]
    pub http_data_path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChartSourcePreview {
    pub data_source_type: String,
    pub data_source_config: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct AiSettingsForm {
    pub provider: String,
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub clear_key: Option<String>,
    #[serde(default)]
    pub enabled: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SavedQuery {
    pub saved: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MediaQuery {
    pub q: Option<String>,
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct SiteSettingsForm {
    pub title: String,
    #[serde(default)]
    pub brand: String,
    #[serde(default)]
    pub tagline: String,
    #[serde(default)]
    pub description: String,
    pub author: String,
    pub base_url: String,
    #[serde(default)]
    pub hero_image: String,
    pub posts_per_page: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChartQuery {
    pub post_id: Option<i64>,
}

#[debug_handler]
async fn login_page(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    format::render().header("cache-control", "no-store").view(
        &v,
        "admin/login.html",
        data!({ "site": site_json(&settings), "error": "" }),
    )
}

#[debug_handler]
async fn login(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
    Form(form): Form<LoginForm>,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let email = form.email.trim().to_ascii_lowercase();
    let allowed = ctx
        .shared_store
        .get::<Limits>()
        .is_none_or(|limits| limits.allow(&format!("login:{email}"), 10));
    if !allowed {
        return format::render().status(StatusCode::TOO_MANY_REQUESTS).view(
            &v,
            "admin/login.html",
            data!({ "site": site_json(&settings), "error": "尝试过于频繁，请一分钟后再试。" }),
        );
    }
    let Ok(user) = crate::models::users::Model::find_by_email(&ctx.db, &email).await else {
        return format::render().status(StatusCode::UNAUTHORIZED).view(
            &v,
            "admin/login.html",
            data!({ "site": site_json(&settings), "error": "邮箱或密码不正确。" }),
        );
    };
    if !user.verify_password(&form.password) {
        return format::render().status(StatusCode::UNAUTHORIZED).view(
            &v,
            "admin/login.html",
            data!({ "site": site_json(&settings), "error": "邮箱或密码不正确。" }),
        );
    }
    if let Some(admin_email) = settings.admin_email() {
        if !user.email.eq_ignore_ascii_case(admin_email) {
            return format::render().status(StatusCode::FORBIDDEN).view(
                &v,
                "admin/login.html",
                data!({ "site": site_json(&settings), "error": "该账号不是站点管理员。" }),
            );
        }
    }
    let cookie = issue_admin_cookie(&ctx, &user)?;
    format::render()
        .status(StatusCode::SEE_OTHER)
        .header("location", "/admin")
        .header("set-cookie", cookie)
        .empty()
}

#[debug_handler]
async fn logout(State(_ctx): State<AppContext>, _admin: AdminSession) -> Result<Response> {
    format::render()
        .status(StatusCode::SEE_OTHER)
        .header("location", "/admin/login")
        .header("set-cookie", clear_admin_cookie())
        .empty()
}

#[debug_handler]
async fn dashboard(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
    _admin: AdminSession,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let ai_settings = ai::effective_ai_settings(&ctx).await?;
    let posts = posts::Model::all_for_admin(&ctx.db).await?;
    let charts = chart_specs::Model::all_for_admin(&ctx.db).await?;
    let sessions = ai_sessions::Model::recent(&ctx.db, 10).await?;
    let session_count = ai_sessions::Entity::find().count(&ctx.db).await?;
    let published = posts::Model::count_by_status(&ctx.db, posts::STATUS_PUBLISHED).await?;
    let drafts = posts::Model::count_by_status(&ctx.db, posts::STATUS_DRAFT).await?;
    let review = posts::Model::count_by_status(&ctx.db, posts::STATUS_REVIEW).await?;
    let post_rows = posts
        .iter()
        .map(|post| {
            json!({
                "id": post.id,
                "title": post.title,
                "slug": post.slug,
                "status": post.status,
                "pinned": post.pinned.unwrap_or(false),
                "updated": util::format_date(&post.updated_at),
                "reading_minutes": post.reading_minutes.max(1),
            })
        })
        .collect::<Vec<_>>();
    let chart_rows = charts
        .iter()
        .map(|chart| {
            json!({
                "id": chart.id,
                "post_id": chart.post_id,
                "title": chart.title,
                "kind": chart.kind,
                "source": chart.data_source_type,
                "status": chart.status,
                "last_success": chart.last_success_at.map(|dt| util::format_date(&dt)).unwrap_or_default(),
                "failure_count": chart.failure_count,
                "refresh_cron": chart.refresh_cron.clone().unwrap_or_default(),
                "error": chart.error_message.clone().unwrap_or_default(),
            })
        })
        .collect::<Vec<_>>();
    let session_rows = sessions
        .iter()
        .map(|session| {
            json!({
                "id": session.id,
                "purpose": session.purpose,
                "status": session.status,
                "provider": session.provider,
                "model": session.model,
                "created": session.created_at.format("%m-%d %H:%M").to_string(),
                "cost": session.cost_estimate.unwrap_or(0.0),
                "error": session.error.clone().unwrap_or_default(),
            })
        })
        .collect::<Vec<_>>();
    format::render().header("cache-control", "no-store").view(
        &v,
        "admin/dashboard.html",
        data!({
            "site": site_json(&settings),
            "posts": post_rows,
            "charts": chart_rows,
            "sessions": session_rows,
            "counts": {
                "posts": posts.len(),
                "published": published,
                "drafts": drafts,
                "review": review,
                "charts": charts.len(),
                "sessions": session_count,
            },
            "ai_offline": ai_settings.is_offline(),
        }),
    )
}

#[debug_handler]
async fn new_post(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
    _admin: AdminSession,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let form = PostForm {
        status: posts::STATUS_DRAFT.to_string(),
        ..PostForm::default()
    };
    render_post_form(&ctx, &v, &settings, None, &form, None, false).await
}

#[debug_handler]
async fn create_post(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
    _admin: AdminSession,
    Form(form): Form<PostForm>,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    match posts::Model::create_post(&ctx.db, &form.to_params()).await {
        Ok(post) => {
            if post.status == posts::STATUS_PUBLISHED {
                enqueue_publish(&ctx, post.id).await;
            }
            crate::controllers::llms::invalidate(&ctx).await;
            redirect_to(&format!("/admin/posts/{}/edit", post.id))
        }
        Err(error) => {
            render_post_form(
                &ctx,
                &v,
                &settings,
                None,
                &form,
                Some(&error.to_string()),
                true,
            )
            .await
        }
    }
}

#[debug_handler]
async fn edit_post(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
    _admin: AdminSession,
    Path(id): Path<i64>,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let post = posts::Model::find_by_id(&ctx.db, id).await?;
    let tags = post.tags(&ctx.db).await?;
    let form = PostForm {
        title: post.title.clone(),
        slug: post.slug.clone(),
        summary: post.summary.clone().unwrap_or_default(),
        body_md: post.body_md.clone(),
        tags: tags
            .iter()
            .map(|tag| tag.name.clone())
            .collect::<Vec<_>>()
            .join(", "),
        status: post.status.clone(),
        pinned: post.pinned.unwrap_or(false).then(|| "on".to_string()),
        cover_image: post.cover_image.clone().unwrap_or_default(),
    };
    render_post_form(&ctx, &v, &settings, Some(&post), &form, None, false).await
}

#[debug_handler]
async fn update_post(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
    _admin: AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<PostForm>,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let post = posts::Model::find_by_id(&ctx.db, id).await?;
    match post.update_post(&ctx.db, &form.to_params()).await {
        Ok(post) => {
            if post.status == posts::STATUS_PUBLISHED {
                enqueue_publish(&ctx, post.id).await;
            }
            crate::controllers::llms::invalidate(&ctx).await;
            redirect_to(&format!("/admin/posts/{}/edit", post.id))
        }
        Err(error) => {
            render_post_form(
                &ctx,
                &v,
                &settings,
                Some(&post),
                &form,
                Some(&error.to_string()),
                false,
            )
            .await
        }
    }
}

#[debug_handler]
async fn publish_post(
    State(ctx): State<AppContext>,
    _admin: AdminSession,
    Path(id): Path<i64>,
) -> Result<Response> {
    let post = posts::Model::find_by_id(&ctx.db, id).await?;
    post.set_status(&ctx.db, posts::STATUS_PUBLISHED).await?;
    enqueue_publish(&ctx, post.id).await;
    crate::controllers::llms::invalidate(&ctx).await;
    redirect_to(&format!("/admin/posts/{}/edit", post.id))
}

#[debug_handler]
async fn unpublish_post(
    State(ctx): State<AppContext>,
    _admin: AdminSession,
    Path(id): Path<i64>,
) -> Result<Response> {
    let post = posts::Model::find_by_id(&ctx.db, id).await?;
    post.set_status(&ctx.db, posts::STATUS_DRAFT).await?;
    crate::controllers::llms::invalidate(&ctx).await;
    redirect_to("/admin/posts")
}

#[debug_handler]
async fn posts_index(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
    _admin: AdminSession,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let posts = posts::Model::all_for_admin(&ctx.db).await?;
    let category_rows = categories::Model::all(&ctx.db).await?;
    let by_id = category_rows
        .iter()
        .map(|category| (category.id, category.name.clone()))
        .collect::<std::collections::HashMap<_, _>>();
    let rows = posts
        .iter()
        .map(|post| {
            json!({
                "id": post.id,
                "title": post.title,
                "slug": post.slug,
                "status": post.status,
                "pinned": post.pinned.unwrap_or(false),
                "category": post.category_id.and_then(|id| by_id.get(&id).cloned()).unwrap_or_default(),
                "updated": util::format_date(&post.updated_at),
                "published": post.published_at.map(|dt| util::format_date(&dt)).unwrap_or_default(),
                "reading_minutes": post.reading_minutes.max(1),
            })
        })
        .collect::<Vec<_>>();
    format::render().header("cache-control", "no-store").view(
        &v,
        "admin/posts.html",
        data!({
            "site": site_json(&settings),
            "posts": rows,
            "total": posts.len(),
            "published_count": posts.iter().filter(|post| post.status == posts::STATUS_PUBLISHED).count(),
            "draft_count": posts.iter().filter(|post| post.status == posts::STATUS_DRAFT).count(),
        }),
    )
}

#[debug_handler]
async fn delete_post(
    State(ctx): State<AppContext>,
    _admin: AdminSession,
    Path(id): Path<i64>,
) -> Result<Response> {
    let post = posts::Model::find_by_id(&ctx.db, id).await?;
    post.delete_post(&ctx.db).await?;
    crate::controllers::llms::invalidate(&ctx).await;
    redirect_to("/admin/posts")
}

async fn render_post_form(
    ctx: &AppContext,
    v: &TeraView,
    settings: &Settings,
    post: Option<&posts::Model>,
    form: &PostForm,
    error: Option<&str>,
    is_new: bool,
) -> Result<Response> {
    let ai_settings = ai::effective_ai_settings(ctx).await?;
    let media_models = medias::Model::recent(&ctx.db, 50).await?;
    let media = media_models
        .iter()
        .map(|item| {
            json!({
                "original_name": item.original_name,
                "url": media::public_url(&item.filename),
                "kind": item.kind,
                "markdown": if item.kind == "image" {
                    format!("![{}]({})", media::markdown_alt(&item.original_name), media::public_url(&item.filename))
                } else {
                    media::video_marker(&item.filename)
                },
                "video_marker": media::video_marker(&item.filename),
            })
        })
        .collect::<Vec<_>>();
    let mut charts = Vec::new();
    let mut sessions = Vec::new();
    if let Some(post) = post {
        let chart_models = chart_specs::Model::for_post(&ctx.db, post.id).await?;
        for chart in chart_models {
            let cache = chart_data_caches::Model::find_for_chart(&ctx.db, chart.id).await?;
            charts.push(json!({
                "id": chart.id,
                "title": chart.title,
                "kind": chart.kind,
                "status": chart.status,
                "stale": cache.as_ref().is_none_or(|cache| !cache.is_fresh()),
                "last_success": chart.last_success_at.map(|dt| util::format_date(&dt)).unwrap_or_default(),
                "refresh_cron": chart.refresh_cron.clone().unwrap_or_default(),
                "error": chart.error_message.clone().unwrap_or_default(),
            }));
        }
        for session in ai_sessions::Model::for_post(&ctx.db, post.id).await? {
            sessions.push(json!({
                "id": session.id,
                "purpose": session.purpose,
                "status": session.status,
                "provider": session.provider,
                "model": session.model,
                "created": session.created_at.format("%m-%d %H:%M").to_string(),
                "error": session.error.clone().unwrap_or_default(),
            }));
        }
    }
    let data = json!({
        "site": site_json(settings),
        "post": {
            "id": post.map(|post| post.id),
            "title": &form.title,
            "slug": &form.slug,
            "summary": &form.summary,
            "body_md": &form.body_md,
            "tags": &form.tags,
            "status": if form.status.is_empty() { posts::STATUS_DRAFT } else { &form.status },
            "pinned": form.pinned.is_some(),
            "cover_image": &form.cover_image,
        },
        "charts": charts,
        "media": media,
        "sessions": sessions,
        "error": error.unwrap_or_default(),
        "is_new": is_new,
        "ai_offline": ai_settings.is_offline(),
    });
    let mut response = format::render();
    if error.is_some() {
        response = response.status(StatusCode::BAD_REQUEST);
    }
    response
        .header("cache-control", "no-store")
        .view(v, "admin/post_form.html", data)
}

#[debug_handler]
async fn charts_index(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
    _admin: AdminSession,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let charts = chart_specs::Model::all_for_admin(&ctx.db).await?;
    let rows = charts
        .iter()
        .map(|chart| {
            json!({
                "id": chart.id,
                "post_id": chart.post_id,
                "title": chart.title,
                "kind": chart.kind,
                "source": chart.data_source_type,
                "status": chart.status,
                "last_success": chart.last_success_at.map(|dt| util::format_date(&dt)).unwrap_or_default(),
                "failure_count": chart.failure_count,
                "refresh_cron": chart.refresh_cron.clone().unwrap_or_default(),
                "error": chart.error_message.clone().unwrap_or_default(),
            })
        })
        .collect::<Vec<_>>();
    format::render().header("cache-control", "no-store").view(
        &v,
        "admin/charts.html",
        data!({ "site": site_json(&settings), "charts": rows }),
    )
}

#[debug_handler]
async fn new_chart(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
    _admin: AdminSession,
    Query(query): Query<ChartQuery>,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let posts = posts::Model::all_for_admin(&ctx.db).await?;
    let Some(first_post) = posts.first() else {
        return bad_request("请先创建至少一篇文章，再新建图表");
    };
    let post_id = query
        .post_id
        .filter(|id| posts.iter().any(|post| post.id == *id))
        .unwrap_or(first_post.id);
    let form = ChartForm {
        post_id,
        title: String::new(),
        intent: String::new(),
        kind: "bar".to_string(),
        data_source_type: "static".to_string(),
        data_source_config: "{}".to_string(),
        render_config: "{}".to_string(),
        refresh_cron: "0 0 4 * * * *".to_string(),
        status: chart_specs::STATUS_ACTIVE.to_string(),
        visual_mode: Some("1".to_string()),
        advanced_mode: None,
        visual_config: String::new(),
        sql_query: String::new(),
        sql_limit: "500".to_string(),
        http_url: String::new(),
        http_data_path: String::new(),
    };
    render_chart_builder(&ctx, &v, &settings, None, &form, None).await
}

#[debug_handler]
async fn create_chart(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
    _admin: AdminSession,
    Form(form): Form<ChartForm>,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    if posts::Model::find_by_id(&ctx.db, form.post_id)
        .await
        .is_err()
    {
        return render_chart_builder(
            &ctx,
            &v,
            &settings,
            None,
            &form,
            Some("请选择一篇存在的文章"),
        )
        .await;
    }
    let params = match chart_params(&form, &settings) {
        Ok(params) => params,
        Err(error) => {
            return render_chart_builder(
                &ctx,
                &v,
                &settings,
                None,
                &form,
                Some(&error.to_string()),
            )
            .await;
        }
    };
    match chart_specs::Model::create_chart(&ctx.db, form.post_id, &params).await {
        Ok(chart) => {
            let _ = ChartWorker::perform_later(&ctx, ChartArgs { chart_id: chart.id }).await;
            redirect_to(&format!("/admin/charts/{}/edit", chart.id))
        }
        Err(error) => {
            render_chart_builder(&ctx, &v, &settings, None, &form, Some(&error.to_string())).await
        }
    }
}

#[debug_handler]
async fn edit_chart(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
    _admin: AdminSession,
    Path(id): Path<i64>,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let chart = chart_specs::Model::find_by_id(&ctx.db, id).await?;
    let form = chart_form_from_model(&chart);
    render_chart_builder(&ctx, &v, &settings, Some(&chart), &form, None).await
}

#[debug_handler]
async fn update_chart(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
    _admin: AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<ChartForm>,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let chart = chart_specs::Model::find_by_id(&ctx.db, id).await?;
    let params = match chart_params(&form, &settings) {
        Ok(params) => params,
        Err(error) => {
            return render_chart_builder(
                &ctx,
                &v,
                &settings,
                Some(&chart),
                &form,
                Some(&error.to_string()),
            )
            .await;
        }
    };
    match chart.update_chart(&ctx.db, &params).await {
        Ok(chart) => {
            let _ = ChartWorker::perform_later(&ctx, ChartArgs { chart_id: chart.id }).await;
            redirect_to(&format!("/admin/charts/{}/edit", chart.id))
        }
        Err(error) => {
            render_chart_builder(
                &ctx,
                &v,
                &settings,
                Some(&chart),
                &form,
                Some(&error.to_string()),
            )
            .await
        }
    }
}

#[debug_handler]
async fn delete_chart(
    State(ctx): State<AppContext>,
    _admin: AdminSession,
    Path(id): Path<i64>,
) -> Result<Response> {
    let chart = chart_specs::Model::find_by_id(&ctx.db, id).await?;
    chart.delete_chart(&ctx.db).await?;
    redirect_to("/admin/charts")
}

#[debug_handler]
async fn refresh_chart_now(
    State(ctx): State<AppContext>,
    _admin: AdminSession,
    Path(id): Path<i64>,
) -> Result<Response> {
    let chart = chart_specs::Model::find_by_id(&ctx.db, id).await?;
    let _ = ChartWorker::perform_later(&ctx, ChartArgs { chart_id: chart.id }).await;
    redirect_to(&format!("/admin/charts/{}/edit", chart.id))
}

fn chart_form_from_model(chart: &chart_specs::Model) -> ChartForm {
    ChartForm {
        post_id: chart.post_id,
        title: chart.title.clone(),
        intent: chart.intent.clone().unwrap_or_default(),
        kind: chart.kind.clone(),
        data_source_type: chart.data_source_type.clone(),
        data_source_config: serde_json::to_string_pretty(&chart.data_source_config)
            .unwrap_or_else(|_| "{}".to_string()),
        render_config: serde_json::to_string_pretty(&chart.render_config)
            .unwrap_or_else(|_| "{}".to_string()),
        refresh_cron: chart.refresh_cron.clone().unwrap_or_default(),
        status: chart.status.clone(),
        visual_mode: Some("1".to_string()),
        advanced_mode: None,
        visual_config: String::new(),
        sql_query: String::new(),
        sql_limit: "500".to_string(),
        http_url: String::new(),
        http_data_path: String::new(),
    }
}

async fn render_chart_builder(
    ctx: &AppContext,
    v: &TeraView,
    settings: &Settings,
    chart: Option<&chart_specs::Model>,
    form: &ChartForm,
    error: Option<&str>,
) -> Result<Response> {
    let posts = posts::Model::all_for_admin(&ctx.db).await?;
    let recipes = chart_recipes::Model::all(&ctx.db).await?;
    let ai_settings = ai::effective_ai_settings(ctx).await?;
    let mut bootstrap = build_chart_bootstrap(ctx, chart, form).await?;
    if bootstrap.post_id == 0 {
        if let Some(post) = posts.first() {
            bootstrap.post_id = post.id;
        }
    }
    let post_rows = posts
        .iter()
        .map(|post| json!({"id": post.id, "title": post.title, "status": post.status}))
        .collect::<Vec<_>>();
    let recipe_rows = recipes
        .iter()
        .map(|recipe| {
            json!({
                "id": recipe.id,
                "name": recipe.name,
                "description": recipe.description.clone().unwrap_or_default(),
                "prompt_pattern": recipe.prompt_pattern,
            })
        })
        .collect::<Vec<_>>();
    let data = json!({
        "site": site_json(settings),
        "chart_id": chart.map(|chart| chart.id),
        "post_id": bootstrap.post_id,
        "posts": post_rows,
        "recipes": recipe_rows,
        "bootstrap": serde_json::to_string(&bootstrap)?,
        "error": error.unwrap_or_default(),
        "ai_offline": ai_settings.is_offline(),
    });
    let mut response = format::render();
    if error.is_some() {
        response = response.status(StatusCode::BAD_REQUEST);
    }
    response
        .header("cache-control", "no-store")
        .view(v, "admin/chart_builder.html", data)
}

async fn build_chart_bootstrap(
    ctx: &AppContext,
    chart: Option<&chart_specs::Model>,
    form: &ChartForm,
) -> Result<ChartBootstrap> {
    let cache = if let Some(chart) = chart {
        chart_data_caches::Model::find_for_chart(&ctx.db, chart.id).await?
    } else {
        None
    };
    let mut rows = Vec::new();
    let mut columns = Vec::new();
    let mut kind = if form.kind.is_empty() {
        "bar".to_string()
    } else {
        form.kind.clone()
    };
    let mut x_field = String::new();
    let mut y_fields = Vec::new();
    let mut options = blueprint::ChartOptions::default();
    let mut title = form.title.clone();
    let mut intent = form.intent.clone();
    let mut source_type = if form.data_source_type.is_empty() {
        "static".to_string()
    } else {
        form.data_source_type.clone()
    };
    let mut sql_query = form.sql_query.clone();
    let mut sql_limit = form.sql_limit.parse::<usize>().unwrap_or(500);
    let mut http_url = form.http_url.clone();
    let mut http_data_path = form.http_data_path.clone();
    let mut advanced_config = form.data_source_config.clone();
    let mut advanced_render = form.render_config.clone();

    if let Some(chart) = chart {
        if title.trim().is_empty() {
            title = chart.title.clone();
        }
        if intent.trim().is_empty() {
            intent = chart.intent.clone().unwrap_or_default();
        }
        if form.data_source_type.is_empty() {
            source_type = chart.data_source_type.clone();
        }
        advanced_config = serde_json::to_string_pretty(&chart.data_source_config)
            .unwrap_or_else(|_| form.data_source_config.clone());
        advanced_render = serde_json::to_string_pretty(&chart.render_config)
            .unwrap_or_else(|_| form.render_config.clone());
        let config = &chart.data_source_config;
        if chart.data_source_type == "static" {
            rows = config
                .get("data")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
        } else if let Some(cache) = &cache {
            rows = blueprint::rows_from_payload(&cache.payload);
        }
        columns = config
            .get("columns")
            .and_then(serde_json::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if columns.is_empty() {
            columns = blueprint::columns_from_rows(&rows);
        }
        let stored = config
            .get("blueprint")
            .and_then(|value| serde_json::from_value::<ChartBlueprint>(value.clone()).ok());
        if let Some(stored) = stored {
            kind = if stored.kind.is_empty() {
                chart.kind.clone()
            } else {
                stored.kind
            };
            x_field = stored.x_field;
            y_fields = stored.y_fields;
            options = stored.options;
        } else {
            let derived = blueprint::derive_blueprint(
                &title,
                &chart.render_config,
                rows.clone(),
                columns.clone(),
            );
            kind = derived.kind;
            x_field = derived.x_field;
            y_fields = derived.y_fields;
            options = derived.options;
        }
        sql_query = config
            .get("query")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&sql_query)
            .to_string();
        sql_limit = config
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(sql_limit);
        http_url = config
            .get("url")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&http_url)
            .to_string();
        http_data_path = config
            .get("data_path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&http_data_path)
            .to_string();
    }

    if !form.visual_config.trim().is_empty() {
        if let Ok(input) = serde_json::from_str::<VisualChartInput>(&form.visual_config) {
            kind = if input.kind.is_empty() {
                kind
            } else {
                input.kind
            };
            x_field = input.x_field;
            y_fields = input.y_fields;
            options = input.options;
            if !input.columns.is_empty() {
                columns = input.columns;
            }
            if !input.rows.is_empty() {
                rows = input.rows;
            }
            if !input.source_type.trim().is_empty() {
                source_type = input.source_type;
            }
            if !input.sql_query.trim().is_empty() {
                sql_query = input.sql_query;
            }
            if let Some(limit) = input.sql_limit {
                sql_limit = limit;
            }
            if !input.http_url.trim().is_empty() {
                http_url = input.http_url;
            }
            if !input.http_data_path.trim().is_empty() {
                http_data_path = input.http_data_path;
            }
        }
    }

    if rows.is_empty() && chart.is_none() {
        let sample = sample_chart_data();
        columns = sample.0;
        rows = sample.1;
        if x_field.is_empty() {
            x_field = columns.first().cloned().unwrap_or_default();
        }
        if y_fields.is_empty() {
            y_fields = columns.iter().skip(1).cloned().collect();
        }
    }
    if columns.is_empty() {
        columns = blueprint::columns_from_rows(&rows);
    }
    if (x_field.is_empty() || y_fields.is_empty()) && !rows.is_empty() {
        let derived = blueprint::derive_blueprint(
            &title,
            &serde_json::Value::Object(serde_json::Map::new()),
            rows.clone(),
            columns.clone(),
        );
        if x_field.is_empty() {
            x_field = derived.x_field;
        }
        if y_fields.is_empty() {
            y_fields = derived.y_fields;
        }
    }
    if kind.is_empty() {
        kind = "bar".to_string();
    }
    Ok(ChartBootstrap {
        id: chart.map(|chart| chart.id),
        post_id: if form.post_id > 0 {
            form.post_id
        } else {
            chart.map_or(0, |chart| chart.post_id)
        },
        title,
        intent,
        status: if form.status.is_empty() {
            chart_specs::STATUS_ACTIVE.to_string()
        } else {
            form.status.clone()
        },
        refresh_cron: if form.refresh_cron.is_empty() {
            chart
                .and_then(|chart| chart.refresh_cron.clone())
                .unwrap_or_default()
        } else {
            form.refresh_cron.clone()
        },
        source_type,
        sql_query,
        sql_limit,
        http_url,
        http_data_path,
        kind,
        x_field,
        y_fields,
        options,
        columns,
        rows,
        advanced_config,
        advanced_render,
        last_success: chart
            .and_then(|chart| chart.last_success_at)
            .map(|dt| util::format_date(&dt))
            .unwrap_or_default(),
        error: chart
            .and_then(|chart| chart.error_message.clone())
            .unwrap_or_default(),
        stale: cache.as_ref().is_none_or(|cache| !cache.is_fresh()),
        is_new: chart.is_none(),
    })
}

fn sample_chart_data() -> (Vec<String>, Vec<serde_json::Value>) {
    (
        vec!["月份".to_string(), "发布数".to_string()],
        vec![
            json!({"月份": "2026-07", "发布数": 3}),
            json!({"月份": "2026-08", "发布数": 5}),
            json!({"月份": "2026-09", "发布数": 4}),
            json!({"月份": "2026-10", "发布数": 7}),
        ],
    )
}

fn chart_params(form: &ChartForm, settings: &Settings) -> Result<chart_specs::ChartParams> {
    if form.title.trim().is_empty() {
        return Err(Error::string("图表标题不能为空"));
    }
    let visual = form.advanced_mode.is_none()
        && (form.visual_mode.as_deref() == Some("1") || !form.visual_config.trim().is_empty());
    if visual {
        if form.visual_config.trim().is_empty() {
            return Err(Error::string("可视化配置为空，请刷新页面重试"));
        }
        if form.visual_config.len() > 2 * 1024 * 1024 {
            return Err(Error::string("可视化数据超过 2 MiB"));
        }
        let input: VisualChartInput = serde_json::from_str(&form.visual_config)
            .map_err(|err| Error::string(&format!("可视化配置解析失败：{err}")))?;
        return blueprint::params_from_visual(
            &form.title,
            &form.intent,
            &form.refresh_cron,
            &form.status,
            input,
            &settings.charts.sql_allowed_tables,
        );
    }
    if form.render_config.trim().is_empty() {
        return Err(Error::string("高级模式下 render_config 不能为空"));
    }
    Ok(chart_specs::ChartParams {
        title: form.title.clone(),
        intent: form.intent.clone(),
        kind: if form.kind.is_empty() {
            "bar".to_string()
        } else {
            form.kind.clone()
        },
        data_source_type: if form.data_source_type.is_empty() {
            "static".to_string()
        } else {
            form.data_source_type.clone()
        },
        data_source_config: form.data_source_config.clone(),
        render_config: form.render_config.clone(),
        refresh_cron: form.refresh_cron.clone(),
        status: form.status.clone(),
    })
}

#[debug_handler]
async fn chart_assist(
    State(ctx): State<AppContext>,
    _admin: AdminSession,
    mut multipart: Multipart,
) -> Result<Response> {
    let ai_settings = ai::effective_ai_settings(&ctx).await?;
    let mut prompt = String::new();
    let mut current = None;
    let mut parsed = None;
    let mut file_name = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|err| Error::string(&format!("无法读取上传内容：{err}")))?
    {
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "prompt" => {
                prompt = field
                    .text()
                    .await
                    .map_err(|err| Error::string(&format!("无法读取提示词：{err}")))?;
            }
            "current" => {
                let text = field
                    .text()
                    .await
                    .map_err(|err| Error::string(&format!("无法读取当前图表：{err}")))?;
                current = serde_json::from_str::<ChartBlueprint>(&text).ok();
            }
            "file" => {
                let name = field.file_name().map(str::to_string);
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|err| Error::string(&format!("无法读取文件：{err}")))?;
                if !bytes.is_empty() {
                    let filename = name.unwrap_or_else(|| "upload.csv".to_string());
                    match parse::parse_file(&filename, &bytes) {
                        Ok(data) => parsed = Some(data),
                        Err(error) => return bad_request(error.to_string()),
                    }
                    file_name = Some(filename);
                }
            }
            _ => {}
        }
    }
    if prompt.trim().is_empty() && parsed.is_none() && current.is_none() {
        return bad_request("请输入提示词或上传数据文件");
    }
    if prompt.trim().is_empty() {
        prompt = "根据这份数据生成一个清晰的图表".to_string();
    }
    let provider = if ai_settings.is_offline() {
        "offline".to_string()
    } else {
        ai_settings.provider.clone()
    };
    let model = if ai_settings.is_offline() {
        "heuristic-v1".to_string()
    } else {
        ai_settings.model.clone()
    };
    let (system, user) = ai::chart_blueprint_prompts(&prompt, parsed.as_ref(), current.as_ref());
    let session = ai_sessions::Model::start(
        &ctx.db,
        None,
        "chart_blueprint",
        &provider,
        &model,
        json!([
            {"role": "system", "content": system},
            {"role": "user", "content": user},
            {"role": "file", "content": file_name.clone().unwrap_or_default()}
        ]),
    )
    .await?;

    let result: Result<(ChartBlueprint, i64, i64)> = if ai_settings.is_offline() {
        Ok((
            blueprint::offline_blueprint(&prompt, parsed.as_ref(), current.as_ref()),
            0,
            0,
        ))
    } else {
        let permit = match ctx.shared_store.get::<Limits>() {
            Some(limits) => Some(limits.ai_permit().await?),
            None => None,
        };
        let chat = ai::chat(&ctx, "chart_blueprint", &system, &user, &user).await;
        drop(permit);
        match chat {
            Ok(response) => {
                let value = ai::parse_json_response(&response.content)?;
                let blueprint = serde_json::from_value::<ChartBlueprint>(value)
                    .map_err(|err| Error::string(&format!("AI 返回的图表配置无效：{err}")))?;
                Ok((blueprint, response.input_tokens, response.output_tokens))
            }
            Err(error) => Err(error),
        }
    };

    let result = match result {
        Ok((blueprint, input_tokens, output_tokens)) => {
            let fallback_rows = parsed
                .as_ref()
                .map(|parsed| parsed.rows.clone())
                .or_else(|| current.as_ref().map(|current| current.rows.clone()))
                .unwrap_or_default();
            let fallback_columns = parsed
                .as_ref()
                .map(|parsed| parsed.columns.clone())
                .or_else(|| current.as_ref().map(|current| current.columns.clone()))
                .unwrap_or_default();
            let mut blueprint = blueprint;
            if blueprint.title.trim().is_empty() {
                blueprint.title = util::truncate_chars(prompt.trim(), 60);
            }
            let normalized =
                blueprint::normalize_blueprint(blueprint, fallback_rows, fallback_columns)?;
            Ok((normalized, input_tokens, output_tokens))
        }
        Err(error) => Err(error),
    };

    match result {
        Ok((blueprint, input_tokens, output_tokens)) => {
            let cost = ai::cost_estimate(&ai_settings, input_tokens, output_tokens);
            let audit = json!({
                "title": blueprint.title,
                "kind": blueprint.kind,
                "x_field": blueprint.x_field,
                "y_fields": blueprint.y_fields,
                "columns": blueprint.columns,
                "row_count": blueprint.rows.len(),
            });
            session
                .complete(&ctx.db, audit, input_tokens, output_tokens, cost, None)
                .await?;
            Ok(format::json(json!({
                "blueprint": blueprint,
                "session_id": session.id,
                "offline": ai_settings.is_offline(),
                "model": model,
                "file": {
                    "name": file_name,
                    "columns": parsed.as_ref().map(|parsed| parsed.columns.clone()).unwrap_or_default(),
                    "rows": parsed.as_ref().map(|parsed| parsed.rows.len()).unwrap_or(0)
                }
            }))?)
        }
        Err(error) => {
            let _ = session.fail(&ctx.db, &error.to_string()).await;
            Err(error)
        }
    }
}

#[debug_handler]
async fn chart_source_preview(
    State(ctx): State<AppContext>,
    _admin: AdminSession,
    Json(params): Json<ChartSourcePreview>,
) -> Result<Response> {
    if params.data_source_type == "static" {
        return bad_request("静态图表不需要服务端预览");
    }
    let settings = Settings::from_context(&ctx)?;
    let data = match charts::fetch_config(
        &ctx,
        &params.data_source_type,
        &params.data_source_config,
        &settings,
    )
    .await
    {
        Ok(data) => data,
        Err(error) => return bad_request(error.to_string()),
    };
    let rows = blueprint::rows_from_payload(&data);
    let columns = blueprint::columns_from_rows(&rows);
    format::json(json!({
        "data": data,
        "rows": rows,
        "columns": columns,
    }))
}

#[debug_handler]
async fn site_settings_page(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
    _admin: AdminSession,
    Query(query): Query<SavedQuery>,
) -> Result<Response> {
    render_site_settings(&ctx, &v, query.saved.as_deref() == Some("1"), None).await
}

#[debug_handler]
async fn save_site_settings(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
    _admin: AdminSession,
    Form(form): Form<SiteSettingsForm>,
) -> Result<Response> {
    let title = form.title.trim();
    let author = form.author.trim();
    let base_url = form.base_url.trim().trim_end_matches('/');
    if title.is_empty() {
        return render_site_settings(&ctx, &v, false, Some("站点名称不能为空")).await;
    }
    if author.is_empty() {
        return render_site_settings(&ctx, &v, false, Some("作者不能为空")).await;
    }
    let Ok(url) = reqwest::Url::parse(base_url) else {
        return render_site_settings(&ctx, &v, false, Some("站点 URL 不是合法 URL")).await;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return render_site_settings(&ctx, &v, false, Some("站点 URL 必须是 http/https")).await;
    }
    let posts_per_page = form.posts_per_page.trim().parse::<i64>().unwrap_or(20);
    if !(1..=100).contains(&posts_per_page) {
        return render_site_settings(&ctx, &v, false, Some("每页文章数必须在 1–100 之间")).await;
    }
    let row = site_settings::Model::save(
        &ctx.db,
        title,
        form.brand.trim(),
        form.tagline.trim(),
        form.description.trim(),
        author,
        base_url,
        form.hero_image.trim(),
        posts_per_page,
    )
    .await?;
    ctx.shared_store.insert(row);
    crate::controllers::llms::invalidate(&ctx).await;
    redirect_to("/admin/settings/site?saved=1")
}

async fn render_site_settings(
    ctx: &AppContext,
    v: &TeraView,
    saved: bool,
    error: Option<&str>,
) -> Result<Response> {
    let settings = Settings::from_context(ctx)?;
    let data = data!({
        "site": site_json(&settings),
        "values": {
            "title": settings.site.title,
            "brand": settings.site.brand,
            "tagline": settings.site.tagline,
            "description": settings.site.description,
            "author": settings.site.author,
            "base_url": settings.site.base_url,
            "hero_image": settings.site.hero_image,
            "posts_per_page": settings.site.posts_per_page,
        },
        "saved": saved,
        "error": error.unwrap_or_default(),
    });
    let mut response = format::render();
    if error.is_some() {
        response = response.status(StatusCode::BAD_REQUEST);
    }
    response
        .header("cache-control", "no-store")
        .view(v, "admin/site_settings.html", data)
}

#[debug_handler]
async fn ai_settings_page(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
    _admin: AdminSession,
    Query(query): Query<SavedQuery>,
) -> Result<Response> {
    render_ai_settings(&ctx, &v, query.saved.as_deref() == Some("1"), None).await
}

#[debug_handler]
async fn save_ai_settings(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
    _admin: AdminSession,
    Form(form): Form<AiSettingsForm>,
) -> Result<Response> {
    let provider = form.provider.trim().to_ascii_lowercase();
    if provider.is_empty() {
        return render_ai_settings(&ctx, &v, false, Some("提供商不能为空")).await;
    }
    let base_url = form.base_url.trim().to_string();
    if provider != "offline" {
        if base_url.is_empty() {
            return render_ai_settings(&ctx, &v, false, Some("Base URL 不能为空")).await;
        }
        let Ok(url) = reqwest::Url::parse(&base_url) else {
            return render_ai_settings(&ctx, &v, false, Some("Base URL 不是合法 URL")).await;
        };
        if !matches!(url.scheme(), "http" | "https") {
            return render_ai_settings(&ctx, &v, false, Some("Base URL 必须是 http/https")).await;
        }
        if form.model.trim().is_empty() {
            return render_ai_settings(&ctx, &v, false, Some("模型名不能为空")).await;
        }
    }
    let existing = ai_settings::Model::singleton(&ctx.db).await?;
    let key_change = if form.clear_key.is_some() {
        Some(String::new())
    } else if form.api_key.trim().is_empty() {
        None
    } else {
        Some(form.api_key.trim().to_string())
    };
    ai_settings::Model::save(
        &ctx.db,
        &provider,
        &base_url,
        form.model.trim(),
        key_change,
        form.enabled.is_some(),
    )
    .await?;
    // Existing row values are intentionally not logged; only the provider/model
    // and the fact that a key was present are auditable.
    tracing::info!(
        provider,
        model = form.model.trim(),
        has_key = existing
            .as_ref()
            .and_then(|row| row.api_key.as_ref())
            .is_some_and(|key| !key.is_empty())
            || !form.api_key.trim().is_empty(),
        "AI settings updated"
    );
    redirect_to("/admin/settings/ai?saved=1")
}

async fn render_ai_settings(
    ctx: &AppContext,
    v: &TeraView,
    saved: bool,
    error: Option<&str>,
) -> Result<Response> {
    let settings = Settings::from_context(ctx)?;
    let row = ai_settings::Model::singleton(&ctx.db).await?;
    let effective = ai::effective_ai_settings(ctx).await?;
    let env_has_key = !settings.ai.api_key.trim().is_empty();
    let row_has_key = row
        .as_ref()
        .and_then(|row| row.api_key.as_ref())
        .is_some_and(|key| !key.is_empty());
    let has_key =
        row_has_key || (row.as_ref().is_none_or(|row| row.api_key.is_none()) && env_has_key);
    let key_source = if row_has_key {
        "database"
    } else if row.as_ref().is_none_or(|row| row.api_key.is_none()) && env_has_key {
        "environment"
    } else {
        "none"
    };
    let provider = row
        .as_ref()
        .map_or_else(|| settings.ai.provider.clone(), |row| row.provider.clone());
    let base_url = row
        .as_ref()
        .map_or_else(|| settings.ai.base_url.clone(), |row| row.base_url.clone());
    let model = row
        .as_ref()
        .map_or_else(|| settings.ai.model.clone(), |row| row.model.clone());
    let enabled = row.as_ref().is_none_or(|row| row.enabled);
    let data = data!({
        "site": site_json(&settings),
        "provider": provider,
        "base_url": base_url,
        "model": model,
        "has_key": has_key,
        "key_source": key_source,
        "enabled": enabled,
        "effective_offline": effective.is_offline(),
        "effective_provider": effective.provider,
        "effective_model": effective.model,
        "presets": ai::presets(),
        "saved": saved,
        "error": error.unwrap_or_default(),
    });
    let mut response = format::render();
    if error.is_some() {
        response = response.status(StatusCode::BAD_REQUEST);
    }
    response
        .header("cache-control", "no-store")
        .view(v, "admin/ai_settings.html", data)
}

const MAX_ARTICLE_EDIT_BYTES: usize = 5_000_000;
const MAX_ARTICLE_BODY_CHARS: usize = 200_000;
const MAX_REFERENCE_URLS: usize = 5;

#[debug_handler]
async fn article_edit(
    State(ctx): State<AppContext>,
    _admin: AdminSession,
    mut multipart: Multipart,
) -> Result<Response> {
    let mut post_id = None;
    let mut mode = "edit".to_string();
    let mut title = String::new();
    let mut prompt = String::new();
    let mut body_md = String::new();
    let mut urls_raw = String::new();
    let mut materials = Vec::new();
    let mut uploaded_bytes = 0usize;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|err| Error::string(&format!("读取上传材料失败：{err}")))?
    {
        let name = field.name().unwrap_or_default().to_string();
        if name == "file" {
            let filename = field
                .file_name()
                .map(ToString::to_string)
                .unwrap_or_else(|| "material.txt".to_string());
            let bytes = field
                .bytes()
                .await
                .map_err(|err| Error::string(&format!("读取材料失败：{err}")))?;
            uploaded_bytes += bytes.len();
            if uploaded_bytes > MAX_ARTICLE_EDIT_BYTES {
                return bad_request("上传材料总大小超过 5 MB");
            }
            match materials::parse_material(&filename, &bytes) {
                Ok(material) => materials.push(material),
                Err(error) => return bad_request(error.to_string()),
            }
            continue;
        }
        let value = field
            .text()
            .await
            .map_err(|err| Error::string(&format!("读取表单失败：{err}")))?;
        match name.as_str() {
            "post_id" => {
                post_id = value
                    .trim()
                    .parse::<i64>()
                    .ok()
                    .filter(|post_id| *post_id > 0);
            }
            "mode" => mode = value,
            "title" => title = value,
            "prompt" => prompt = value,
            "body_md" => body_md = value,
            "urls" => urls_raw = value,
            _ => {}
        }
    }

    let creating = mode.trim() == "create";
    mode = if creating { "create" } else { "edit" }.to_string();
    if body_md.trim().is_empty() && !creating {
        return bad_request("正文为空；如果要直接创作，请选择“从零创作”");
    }
    if body_md.chars().count() > MAX_ARTICLE_BODY_CHARS {
        return bad_request("正文超过 200,000 字符，暂不支持一次修改");
    }
    let post = if let Some(post_id) = post_id {
        Some(posts::Model::find_by_id(&ctx.db, post_id).await?)
    } else {
        None
    };
    if title.trim().is_empty() && !creating {
        title = post
            .as_ref()
            .map_or_else(|| "未命名文章".to_string(), |post| post.title.clone());
    }
    let prompt = if prompt.trim().is_empty() {
        if creating {
            "写一篇结构完整、观点清晰的中文博客文章。".to_string()
        } else {
            "请根据材料改进这篇文章，保持原有观点和结构，必要时补充引用。".to_string()
        }
    } else {
        prompt.trim().to_string()
    };

    let mut urls = Vec::new();
    for line in urls_raw.lines() {
        let line = line.trim();
        if line.is_empty() || urls.iter().any(|url: &String| url == line) {
            continue;
        }
        if urls.len() >= MAX_REFERENCE_URLS {
            return bad_request("一次最多引用 5 个网页");
        }
        urls.push(line.to_string());
    }

    let mut pages = Vec::new();
    let mut warnings = Vec::new();
    for url in &urls {
        match web::fetch_page(url).await {
            Ok(page) => pages.push(page),
            Err(error) => warnings.push(format!("{url}：{error}")),
        }
    }

    let ai_settings = ai::effective_ai_settings(&ctx).await?;
    let provider = if ai_settings.is_offline() {
        "offline".to_string()
    } else {
        ai_settings.provider.clone()
    };
    let sources = pages
        .iter()
        .map(|page| json!({"title": page.title, "url": page.url}))
        .collect::<Vec<_>>();
    let purpose = if creating {
        "article_create"
    } else {
        "article_edit"
    };
    let session = ai_sessions::Model::start(
        &ctx.db,
        post.as_ref().map(|post| post.id),
        purpose,
        &provider,
        &ai_settings.model,
        json!([
            {"role": "system", "content": if creating { "create article with preview" } else { "edit article with diff proposal" }},
            {"role": "user", "content": prompt},
            {"role": "materials", "content": materials.iter().map(|item| item.name.clone()).collect::<Vec<_>>()},
            {"role": "sources", "content": urls.clone()},
            {"role": "body_chars", "content": body_md.chars().count()},
            {"role": "mode", "content": mode}
        ]),
    )
    .await?;
    let (system, user) = if creating {
        ai::article_create_prompts(&title, &prompt, &materials, &pages)
    } else {
        ai::article_edit_prompts(&title, &body_md, &prompt, &materials, &pages)
    };
    let result: Result<(String, i64, i64, Option<ai::ArticleDraft>)> = if ai_settings.is_offline() {
        if creating {
            let draft = ai::offline_article_create(&title, &prompt, &materials, &pages);
            Ok((draft.body_md.clone(), 0, 0, Some(draft)))
        } else {
            Ok((
                ai::offline_article_edit(&body_md, &materials, &pages),
                0,
                0,
                None,
            ))
        }
    } else {
        let permit = match ctx.shared_store.get::<Limits>() {
            Some(limits) => Some(limits.ai_permit().await?),
            None => None,
        };
        let chat = ai::chat_with_max_tokens(
            &ctx,
            purpose,
            &system,
            &user,
            &body_md,
            Some(ai_settings.article_max_output_tokens.max(1_800)),
        )
        .await;
        drop(permit);
        match chat {
            Ok(response) => {
                if creating {
                    let parsed = ai::parse_json_response(&response.content)?;
                    let draft = ai::ArticleDraft {
                        title: parsed
                            .get("title")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .trim()
                            .to_string(),
                        summary: parsed
                            .get("summary")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .trim()
                            .to_string(),
                        tags: parsed
                            .get("tags")
                            .and_then(serde_json::Value::as_array)
                            .map(|tags| {
                                tags.iter()
                                    .filter_map(serde_json::Value::as_str)
                                    .map(str::trim)
                                    .filter(|tag| !tag.is_empty())
                                    .take(5)
                                    .map(str::to_string)
                                    .collect()
                            })
                            .unwrap_or_default(),
                        body_md: parsed
                            .get("body_md")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    };
                    if draft.body_md.trim().is_empty() {
                        Err(Error::string("模型没有返回文章正文"))
                    } else {
                        Ok((
                            draft.body_md.clone(),
                            response.input_tokens,
                            response.output_tokens,
                            Some(draft),
                        ))
                    }
                } else {
                    let proposed = ai::clean_article_response(&response.content);
                    if proposed.trim().is_empty() {
                        Err(Error::string("模型返回了空文章"))
                    } else {
                        Ok((
                            proposed,
                            response.input_tokens,
                            response.output_tokens,
                            None,
                        ))
                    }
                }
            }
            Err(error) => Err(error),
        }
    };
    let (proposed, input_tokens, output_tokens, draft) = match result {
        Ok(result) => result,
        Err(error) => {
            let _ = session.fail(&ctx.db, &error.to_string()).await;
            return Err(error);
        }
    };
    let cost = ai::cost_estimate(&ai_settings, input_tokens, output_tokens);
    let changed = proposed.trim() != body_md.trim();
    let output_title = draft
        .as_ref()
        .map(|draft| draft.title.clone())
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| {
            if title.trim().is_empty() {
                util::truncate_chars(&prompt, 48)
            } else {
                title.clone()
            }
        });
    let output_summary = draft
        .as_ref()
        .map(|draft| draft.summary.clone())
        .unwrap_or_default();
    let output_tags = draft
        .as_ref()
        .map(|draft| draft.tags.clone())
        .unwrap_or_default();
    let audit = json!({
        "mode": mode,
        "changed": changed,
        "chars_before": body_md.chars().count(),
        "chars_after": proposed.chars().count(),
        "materials": materials.iter().map(|item| item.name.clone()).collect::<Vec<_>>(),
        "sources": sources,
        "warnings": warnings,
        "title": output_title,
        "tags": output_tags,
    });
    session
        .complete(&ctx.db, audit, input_tokens, output_tokens, cost, None)
        .await?;

    format::json(json!({
        "session_id": session.id,
        "mode": mode,
        "title": output_title,
        "summary": output_summary,
        "tags": output_tags,
        "original": body_md,
        "proposed": proposed,
        "changed": changed,
        "offline": ai_settings.is_offline(),
        "provider": provider,
        "model": ai_settings.model,
        "sources": sources,
        "warnings": warnings,
    }))
}

#[debug_handler]
async fn categories_page(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
    _admin: AdminSession,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let rows = categories::Model::all(&ctx.db).await?;
    let mut categories_json = Vec::new();
    for category in &rows {
        let count = posts::Model::count_published_in_category(&ctx.db, category.id).await?;
        categories_json.push(json!({
            "name": category.name,
            "slug": category.slug,
            "count": count,
        }));
    }
    format::render().header("cache-control", "no-store").view(
        &v,
        "admin/categories.html",
        data!({
            "site": site_json(&settings),
            "categories": categories_json,
            "post_count": posts::Model::all_for_admin(&ctx.db).await?.len(),
        }),
    )
}

#[debug_handler]
async fn regenerate_categories(
    State(ctx): State<AppContext>,
    _admin: AdminSession,
) -> Result<Response> {
    let posts = posts::Model::all_for_admin(&ctx.db).await?;
    if posts.is_empty() {
        return bad_request("没有文章可以分类");
    }
    let ai_settings = ai::effective_ai_settings(&ctx).await?;
    let provider = if ai_settings.is_offline() {
        "offline".to_string()
    } else {
        ai_settings.provider.clone()
    };
    let session = ai_sessions::Model::start(
        &ctx.db,
        None,
        "categorize",
        &provider,
        &ai_settings.model,
        json!([{"role": "system", "content": "classify all posts into categories"}]),
    )
    .await?;
    let result: Result<(ai::CategoryPlan, i64, i64)> = if ai_settings.is_offline() {
        Ok((ai::offline_category_plan(&posts), 0, 0))
    } else {
        let (system, user) = ai::category_prompts(&posts);
        let permit = match ctx.shared_store.get::<Limits>() {
            Some(limits) => Some(limits.ai_permit().await?),
            None => None,
        };
        let chat =
            ai::chat_with_max_tokens(&ctx, "categorize", &system, &user, &user, Some(2_500)).await;
        drop(permit);
        match chat {
            Ok(response) => {
                let parsed = ai::parse_json_response(&response.content)?;
                let plan = serde_json::from_value::<ai::CategoryPlan>(parsed)
                    .map_err(|err| Error::string(&format!("分类 JSON 无效：{err}")))?;
                Ok((plan, response.input_tokens, response.output_tokens))
            }
            Err(error) => Err(error),
        }
    };
    let (plan, input_tokens, output_tokens) = match result {
        Ok(result) => result,
        Err(error) => {
            let _ = session.fail(&ctx.db, &error.to_string()).await;
            return Err(error);
        }
    };
    if plan.categories.is_empty() {
        let _ = session.fail(&ctx.db, "分类结果为空").await;
        return bad_request("AI 没有生成任何分类");
    }

    let mut names = Vec::new();
    let mut slugs = Vec::new();
    for name in plan.categories.iter().take(12) {
        let name = util::truncate_chars(name.trim(), 40);
        if name.is_empty() {
            continue;
        }
        let slug = util::slugify(&name);
        if slug.is_empty() || slugs.contains(&slug) {
            continue;
        }
        names.push(name);
        slugs.push(slug);
    }
    if names.is_empty() {
        let _ = session.fail(&ctx.db, "分类 slug 为空").await;
        return bad_request("AI 没有生成有效分类");
    }
    let has_unmatched = posts.iter().any(|post| {
        plan.assignments
            .iter()
            .find(|assignment| assignment.post_id == post.id)
            .is_none_or(|assignment| {
                let name = assignment.category.trim();
                !names.iter().any(|item| item == name) && !slugs.contains(&util::slugify(name))
            })
    });
    if has_unmatched && !names.iter().any(|name| name == "其他") {
        names.push("其他".to_string());
        slugs.push(util::slugify("其他"));
    }

    posts::Model::clear_categories(&ctx.db).await?;
    categories::Model::clear_all(&ctx.db).await?;
    let mut by_name = std::collections::HashMap::new();
    let mut by_slug = std::collections::HashMap::new();
    for (name, slug) in names.iter().zip(slugs.iter()) {
        let category = categories::Model::create_category(&ctx.db, name, slug).await?;
        by_name.insert(name.clone(), category.id);
        by_slug.insert(slug.clone(), category.id);
    }
    let mut assigned = 0usize;
    for post in &posts {
        let category_name = plan
            .assignments
            .iter()
            .find(|assignment| assignment.post_id == post.id)
            .map(|assignment| assignment.category.trim())
            .filter(|name| !name.is_empty());
        let category_id = category_name
            .and_then(|name| {
                by_name.get(name).copied().or_else(|| {
                    let slug = util::slugify(name);
                    by_slug.get(&slug).copied()
                })
            })
            .or_else(|| by_name.get("其他").copied());
        if let Some(category_id) = category_id {
            posts::Model::set_category(&ctx.db, post.id, Some(category_id)).await?;
            assigned += 1;
        }
    }
    let cost = ai::cost_estimate(&ai_settings, input_tokens, output_tokens);
    session
        .complete(
            &ctx.db,
            json!({
                "categories": names,
                "assigned": assigned,
                "posts": posts.len(),
            }),
            input_tokens,
            output_tokens,
            cost,
            None,
        )
        .await?;
    format::json(json!({
        "categories": names,
        "assigned": assigned,
        "total": posts.len(),
        "offline": ai_settings.is_offline(),
        "session_id": session.id,
    }))
}

#[debug_handler]
async fn media_page(
    State(ctx): State<AppContext>,
    ViewEngine(v): ViewEngine<TeraView>,
    _admin: AdminSession,
) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let items = medias::Model::recent(&ctx.db, 100).await?;
    let items = items
        .iter()
        .map(|item| {
            json!({
                "id": item.id,
                "url": media::public_url(&item.filename),
                "filename": item.filename,
                "original_name": item.original_name,
                "kind": item.kind,
                "mime": item.mime,
                "size": item.size,
                "created": item.created_at.format("%Y-%m-%d %H:%M").to_string(),
                "markdown": if item.kind == "image" {
                    format!(
                        "![{}]({})",
                        media::markdown_alt(&item.original_name),
                        media::public_url(&item.filename)
                    )
                } else {
                    media::video_marker(&item.filename)
                },
            })
        })
        .collect::<Vec<_>>();
    format::render().header("cache-control", "no-store").view(
        &v,
        "admin/media.html",
        data!({ "site": site_json(&settings), "items": items }),
    )
}

#[debug_handler]
async fn media_api(
    State(ctx): State<AppContext>,
    _admin: AdminSession,
    Query(query): Query<MediaQuery>,
) -> Result<Response> {
    let items = medias::Model::search(
        &ctx.db,
        query.q.as_deref().unwrap_or_default(),
        query.kind.as_deref(),
        100,
    )
    .await?;
    let items = items
        .iter()
        .map(|item| {
            json!({
                "id": item.id,
                "url": media::public_url(&item.filename),
                "filename": item.filename,
                "original_name": item.original_name,
                "kind": item.kind,
                "size": item.size,
                "created": item.created_at.format("%Y-%m-%d %H:%M").to_string(),
                "markdown": if item.kind == "image" {
                    format!("![{}]({})", media::markdown_alt(&item.original_name), media::public_url(&item.filename))
                } else {
                    media::video_marker(&item.filename)
                },
                "video_marker": media::video_marker(&item.filename),
            })
        })
        .collect::<Vec<_>>();
    format::json(json!({ "items": items }))
}

#[debug_handler]
async fn media_upload(
    State(ctx): State<AppContext>,
    _admin: AdminSession,
    mut multipart: Multipart,
) -> Result<Response> {
    let mut upload = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|err| Error::string(&format!("读取上传文件失败：{err}")))?
    {
        if field.name() != Some("file") {
            continue;
        }
        let filename = field
            .file_name()
            .map(ToString::to_string)
            .unwrap_or_else(|| "upload".to_string());
        let bytes = field
            .bytes()
            .await
            .map_err(|err| Error::string(&format!("读取上传文件失败：{err}")))?;
        if bytes.len() > media::MAX_UPLOAD_BYTES {
            return bad_request("文件太大：图片最大 8 MB，视频最大 12 MB");
        }
        upload = Some((filename, bytes));
        break;
    }
    let Some((filename, bytes)) = upload else {
        return bad_request("没有收到文件");
    };
    if let Err(error) = media::inspect(&filename, &bytes) {
        return bad_request(error.to_string());
    }
    let item = media::store(&ctx.db, &filename, &bytes).await?;
    let url = media::public_url(&item.filename);
    let markdown = if item.kind == "image" {
        format!("![{}]({url})", media::markdown_alt(&item.original_name))
    } else {
        media::video_marker(&item.filename)
    };
    format::json(json!({
        "id": item.id,
        "url": url,
        "filename": item.filename,
        "kind": item.kind,
        "mime": item.mime,
        "size": item.size,
        "markdown": markdown,
        "video_marker": media::video_marker(&item.filename),
    }))
}

#[debug_handler]
async fn chart_preview_data(
    State(ctx): State<AppContext>,
    _admin: AdminSession,
    Path(id): Path<i64>,
) -> Result<Response> {
    let chart = chart_specs::Model::find_by_id(&ctx.db, id).await?;
    let cache = chart_data_caches::Model::find_for_chart(&ctx.db, id).await?;
    let (data, etag, updated_at, stale) = if let Some(cache) = &cache {
        (
            cache.payload.clone(),
            cache.etag.clone(),
            cache.fetched_at.to_rfc3339(),
            !cache.is_fresh(),
        )
    } else {
        (
            serde_json::Value::Null,
            util::etag_for_str(&format!("admin-chart:{id}")),
            String::new(),
            true,
        )
    };
    format::render()
        .header("cache-control", "no-store")
        .header("x-chart-stale", if stale { "1" } else { "0" })
        .etag(&etag)?
        .json(json!({
            "chart_id": chart.id,
            "title": chart.title,
            "kind": chart.kind,
            "spec": chart.render_config,
            "data": data,
            "updated_at": updated_at,
            "etag": etag,
            "stale": stale,
            "status": chart.status,
            "error": chart.error_message.clone().unwrap_or_default(),
        }))
}

async fn enqueue_publish(ctx: &AppContext, post_id: i64) {
    if let Err(error) = IndexWorker::perform_later(ctx, IndexArgs { post_id }).await {
        tracing::warn!(post_id, error = %error, "could not enqueue index");
    }
    if let Ok(charts) = chart_specs::Model::for_post(&ctx.db, post_id).await {
        for chart in charts {
            let _ = ChartWorker::perform_later(ctx, ChartArgs { chart_id: chart.id }).await;
        }
    }
}

fn site_json(settings: &Settings) -> serde_json::Value {
    let title = &settings.site.title;
    let brand = if settings.site.brand.trim().is_empty() {
        title
    } else {
        &settings.site.brand
    };
    let tagline = if settings.site.tagline.trim().is_empty() {
        "人与 AI 的编织现场"
    } else {
        &settings.site.tagline
    };
    json!({
        "title": title,
        "brand": brand,
        "tagline": tagline,
        "description": &settings.site.description,
        "author": &settings.site.author,
        "base_url": settings.site.base_url.trim_end_matches('/'),
        "hero_image": settings.site.hero_image,
    })
}

fn redirect_to(location: &str) -> Result<Response> {
    format::render()
        .status(StatusCode::SEE_OTHER)
        .header("location", location)
        .empty()
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/admin")
        .add("/login", get(login_page).post(login))
        .add("/logout", post(logout))
        .add("/", get(dashboard))
        .add("/posts/new", get(new_post))
        .add("/posts", get(posts_index).post(create_post))
        .add("/posts/{id}/edit", get(edit_post))
        .add("/posts/{id}", post(update_post))
        .add("/posts/{id}/publish", post(publish_post))
        .add("/posts/{id}/unpublish", post(unpublish_post))
        .add("/posts/{id}/delete", post(delete_post))
        .add("/charts", get(charts_index))
        .add("/charts/new", get(new_chart))
        .add("/charts", post(create_chart))
        .add("/charts/{id}/edit", get(edit_chart))
        .add("/charts/{id}", post(update_chart))
        .add("/charts/{id}/delete", post(delete_chart))
        .add("/charts/{id}/refresh", post(refresh_chart_now))
        .add("/categories", get(categories_page))
        .add("/api/categories/regenerate", post(regenerate_categories))
        .add("/media", get(media_page))
        .add("/media/upload", post(media_upload))
        .add("/api/media", get(media_api))
        .add("/api/ai/edit-article", post(article_edit))
        .add("/api/charts/{id}/data", get(chart_preview_data))
        .add("/api/charts/assist", post(chart_assist))
        .add("/api/charts/preview", post(chart_source_preview))
        .add(
            "/settings/site",
            get(site_settings_page).post(save_site_settings),
        )
        .add("/settings/ai", get(ai_settings_page).post(save_ai_settings))
}
