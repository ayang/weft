use loco_rs::prelude::*;

use crate::{data::settings::Settings, models::posts, util};

const LLMS_CACHE_KEY: &str = "llms:v1";
const LLMS_FULL_CACHE_KEY: &str = "llms-full:v1";

#[debug_handler]
async fn llms_txt(State(ctx): State<AppContext>) -> Result<Response> {
    let body = cached_body(&ctx, LLMS_CACHE_KEY, false).await?;
    markdown_response(&body)
}

#[debug_handler]
async fn llms_full(State(ctx): State<AppContext>) -> Result<Response> {
    let body = cached_body(&ctx, LLMS_FULL_CACHE_KEY, true).await?;
    markdown_response(&body)
}

async fn cached_body(ctx: &AppContext, key: &str, full: bool) -> Result<String> {
    if let Ok(Some(cached)) = ctx.cache.get::<String>(key).await {
        return Ok(cached);
    }
    let body = render_llms(ctx, full).await?;
    let _ = ctx
        .cache
        .insert_with_expiry(key, &body, std::time::Duration::from_secs(300))
        .await;
    Ok(body)
}

async fn render_llms(ctx: &AppContext, full: bool) -> Result<String> {
    let settings = Settings::from_context(ctx)?;
    let base = settings.site.base_url.trim_end_matches('/');
    let posts = posts::Model::all_published(&ctx.db).await?;
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", settings.site.title));
    out.push_str(&format!("> {}\n\n", settings.site.description));
    out.push_str(&format!("作者：{}  \n", settings.site.author));
    out.push_str(&format!("站点：{base}  \n"));
    out.push_str("语言：zh-CN  \n");
    out.push_str("内容类型：个人文章、可刷新图表\n\n");
    out.push_str("## 说明\n\n");
    out.push_str("这是一个纯动态站点：本文件、文章 Markdown、图表数据和 Agent API 都由服务端按请求生成，不是构建产物。\n");
    out.push_str("AI 生成的摘要与卡片会明确标注；图表只保存 spec + 最新数据，不存静态图片。\n\n");
    out.push_str("## 内容地图\n\n");
    out.push_str(&format!("- 首页：{base}/\n"));
    out.push_str(&format!("- 归档：{base}/archive\n"));
    out.push_str(&format!("- 搜索：{base}/search?q=...\n"));
    out.push_str(&format!("- 本文档：{base}/llms.txt\n"));
    out.push_str(&format!("- 全文 Markdown 合集：{base}/llms-full.txt\n\n"));
    out.push_str("## 文章清单\n\n");
    for post in &posts {
        let url = format!("{base}/posts/{}", post.slug);
        let md = format!("{base}/posts/{}.md", post.slug);
        let summary = post
            .summary
            .clone()
            .map(|summary| summary.replace('\n', " "))
            .unwrap_or_default();
        out.push_str(&format!(
            "- [{}]({url}) · [Markdown]({md}) · 更新 {} · {} 分钟\n",
            post.title,
            post.updated_at.format("%Y-%m-%d"),
            post.reading_minutes.max(1)
        ));
        if !summary.is_empty() {
            out.push_str(&format!("  {}\n", util::truncate_chars(&summary, 200)));
        }
    }
    if full {
        out.push_str("\n## 全文\n\n");
        for post in &posts {
            out.push_str(&format!("### {}\n\n", post.title));
            out.push_str(&format!("来源：{base}/posts/{}.md\n\n", post.slug));
            out.push_str(&post.body_md);
            if !post.body_md.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
        }
    }
    out.push_str("\n## Agent API\n\n");
    out.push_str("只读、结构化、支持 ETag：\n\n");
    out.push_str(&format!(
        "- `GET {base}/api/agent/posts?since=RFC3339&limit=50` — 已发布文章清单\n"
    ));
    out.push_str(&format!(
        "- `GET {base}/api/agent/posts/:slug` — 文章元数据、Markdown、卡片、图表引用\n"
    ));
    out.push_str(&format!(
        "- `GET {base}/api/agent/charts/:id` — 图表 spec + 最新数据 + ETag\n"
    ));
    out.push('\n');
    if settings.agents.token.trim().is_empty() {
        out.push_str(
            "当前 Agent API 无需 token；生产环境建议设置 `AGENT_API_TOKEN` 并启用限流。\n\n",
        );
    } else {
        out.push_str(
            "Agent API 需要 `Authorization: Bearer <AGENT_API_TOKEN>`，并按分钟限流。\n\n",
        );
    }
    out.push_str("## 更新策略\n\n");
    out.push_str("文章发布/更新后立即可见；语义索引由后台任务异步更新。\n");
    out.push_str("带 `refresh_cron` 的图表由 Loco Scheduler 周期刷新，失败按指数退避重试；图表响应包含 `updated_at` 与 ETag，可条件请求。\n");
    out.push_str("页面与 API 使用短时缓存 + ETag；数据缓存过期后仍可返回最近一次成功数据。\n");
    Ok(out)
}
fn markdown_response(body: &str) -> Result<Response> {
    let etag = util::etag_for_str(body);
    let mut response = format::render()
        .header(
            "cache-control",
            "public, max-age=300, stale-while-revalidate=600",
        )
        .etag(&etag)?
        .text(body)?;
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/markdown; charset=utf-8"),
    );
    Ok(response)
}

pub fn routes() -> Routes {
    Routes::new()
        .add("/llms.txt", get(llms_txt))
        .add("/llms-full.txt", get(llms_full))
}

pub async fn invalidate(ctx: &AppContext) {
    let _ = ctx.cache.remove(LLMS_CACHE_KEY).await;
    let _ = ctx.cache.remove(LLMS_FULL_CACHE_KEY).await;
}
