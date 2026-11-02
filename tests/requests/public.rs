use loco_rs::testing::prelude::*;
use serde_json::json;
use serial_test::serial;
use weft::{
    app::App,
    charts,
    models::{
        chart_specs::{self, ChartParams},
        posts::{self, PostParams},
    },
};

async fn seed_post(ctx: &loco_rs::app::AppContext) -> posts::Model {
    posts::Model::create_post(
        &ctx.db,
        &PostParams {
            title: "动态站点验收".to_string(),
            slug: "dynamic-acceptance".to_string(),
            summary: "所有输出都应在请求时生成。".to_string(),
            body_md: "# 正文标题\n\n这是 **Markdown** 正文。\n\n```rust\nlet ok = true;\n```"
                .to_string(),
            status: posts::STATUS_PUBLISHED.to_string(),
            pinned: true,
            cover_image: String::new(),
            tags: vec!["Rust".to_string(), "AI".to_string()],
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
#[serial]
async fn public_site_is_dynamic_and_cacheable() {
    request::<App, _, _>(|request, ctx| async move {
        seed_post(&ctx).await;

        assert_eq!(request.get("/ask").await.status_code(), 404);

        let home = request.get("/").await;
        assert_eq!(home.status_code(), 200);
        assert!(home.text().contains("动态站点验收"));
        assert!(home.text().contains("Weft Test"));
        assert!(home.text().contains("integration test site"));
        assert!(home.text().contains("/static/default-cover.svg"));
        assert!(home.text().contains("/static/hero-banner.svg"));

        let etag = home
            .header("etag")
            .to_str()
            .expect("etag header")
            .to_string();
        let not_modified = request.get("/").add_header("if-none-match", etag).await;
        assert_eq!(not_modified.status_code(), 304);

        let article = request.get("/posts/dynamic-acceptance").await;
        assert_eq!(article.status_code(), 200);
        let html = article.text();
        assert!(
            html.contains("<h1>正文标题</h1>"),
            "body html missing: {html}"
        );
        assert!(!html.contains("AI 辅助提炼"));
        assert!(!html.contains("article-deck"));
        assert!(html.contains("data-chart-id") || !html.contains("数据图表"));

        let markdown = request.get("/posts/dynamic-acceptance.md").await;
        assert_eq!(markdown.status_code(), 200);
        assert!(markdown
            .header("content-type")
            .to_str()
            .unwrap()
            .starts_with("text/markdown"));
        let body = markdown.text();
        assert!(body.contains("slug: dynamic-acceptance"));
        assert!(body.contains("# 正文标题"));

        let llms = request.get("/llms.txt").await;
        assert_eq!(llms.status_code(), 200);
        let llms_text = llms.text();
        assert!(llms_text.contains("dynamic-acceptance"));
        assert!(llms_text.contains("Agent API"));
        assert!(llms_text.contains("按请求生成"));
    })
    .await;
}

#[tokio::test]
#[serial]
async fn chart_markers_control_inline_position() {
    request::<App, _, _>(|request, ctx| async move {
        let created = posts::Model::create_post(
            &ctx.db,
            &PostParams {
                title: "图表插入位置".to_string(),
                slug: "chart-position".to_string(),
                summary: "inline chart".to_string(),
                body_md: "draft".to_string(),
                status: posts::STATUS_DRAFT.to_string(),
                ..PostParams::default()
            },
        )
        .await
        .unwrap();
        let chart = chart_specs::Model::create_chart(
            &ctx.db,
            created.id,
            &ChartParams {
                title: "中间图表".to_string(),
                kind: "bar".to_string(),
                data_source_type: "static".to_string(),
                data_source_config: json!({
                    "data": [{"label": "A", "value": 1}],
                    "columns": ["label", "value"],
                    "blueprint": {
                        "kind": "bar",
                        "x_field": "label",
                        "y_fields": ["value"],
                        "options": {"legend": true}
                    }
                })
                .to_string(),
                render_config: json!({
                    "xAxis": {"type": "category"},
                    "yAxis": {"type": "value"},
                    "dataset": {"source": []},
                    "series": [{"type": "bar", "encode": {"x": "label", "y": "value"}}]
                })
                .to_string(),
                status: chart_specs::STATUS_ACTIVE.to_string(),
                ..ChartParams::default()
            },
        )
        .await
        .unwrap();
        charts::refresh_chart(&ctx, chart.id).await.unwrap();
        let body = format!(
            "# 开头\n\n前面的文字。\n\n[[chart:{}]]\n\n## 后面的小节\n\n后面的文字。\n",
            chart.id
        );
        created
            .update_post(
                &ctx.db,
                &PostParams {
                    title: "图表插入位置".to_string(),
                    slug: "chart-position".to_string(),
                    summary: "inline chart".to_string(),
                    body_md: body,
                    status: posts::STATUS_PUBLISHED.to_string(),
                    ..PostParams::default()
                },
            )
            .await
            .unwrap();

        let html = request.get("/posts/chart-position").await.text();
        assert!(html.contains(&format!("id=\"chart-{}\"", chart.id)));
        let start = html.find("前面的文字").expect("intro");
        let marker = html
            .find(&format!("id=\"chart-{}\"", chart.id))
            .expect("chart");
        let after = html.find("后面的小节").expect("after");
        assert!(
            start < marker && marker < after,
            "chart should be between intro and after"
        );
        assert!(!html.contains("[[chart:"));
        assert!(!html.contains("数据图表"));
    })
    .await;
}

#[tokio::test]
#[serial]
async fn draft_and_review_posts_are_not_public() {
    request::<App, _, _>(|request, ctx| async move {
        let draft = posts::Model::create_post(
            &ctx.db,
            &PostParams {
                title: "草稿".to_string(),
                slug: "hidden-draft".to_string(),
                status: posts::STATUS_DRAFT.to_string(),
                ..PostParams::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(request.get("/posts/hidden-draft").await.status_code(), 404);
        assert_eq!(
            request.get("/posts/hidden-draft.md").await.status_code(),
            404
        );
        assert!(!request.get("/").await.text().contains("草稿"));
        assert!(posts::Model::find_by_slug(&ctx.db, &draft.slug)
            .await
            .is_ok());
    })
    .await;
}

#[tokio::test]
#[serial]
async fn article_title_is_rendered_once_even_if_body_repeats_it() {
    request::<App, _, _>(|request, ctx| async move {
        posts::Model::create_post(
            &ctx.db,
            &PostParams {
                title: "只显示一次的标题".to_string(),
                slug: "single-title".to_string(),
                summary: "详情页不再显示摘要".to_string(),
                body_md: "# 只显示一次的标题\n\n正文第一段。".to_string(),
                status: posts::STATUS_PUBLISHED.to_string(),
                ..PostParams::default()
            },
        )
        .await
        .unwrap();
        let html = request.get("/posts/single-title").await.text();
        assert_eq!(html.matches("<h1>只显示一次的标题</h1>").count(), 1);
        assert!(!html.contains("article-deck"));
        assert!(!html.contains("ai-summary"));
    })
    .await;
}
