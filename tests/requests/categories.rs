use loco_rs::testing::request::{request_with_config, RequestConfigBuilder};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serial_test::serial;
use weft::{
    app::App,
    models::{
        ai_sessions, categories,
        posts::{self, PostParams},
        users,
    },
};

#[tokio::test]
#[serial]
async fn offline_category_regeneration_assigns_posts_and_serves_category_pages() {
    let config = RequestConfigBuilder::new().save_cookies(true).build();
    request_with_config::<App, _, _>(config, |request, ctx| async move {
        users::Model::create_with_password(
            &ctx.db,
            &users::RegisterParams {
                email: "admin@example.com".to_string(),
                password: "change-me-12345".to_string(),
                name: "Admin".to_string(),
            },
        )
        .await
        .unwrap();
        let login = request
            .post("/admin/login")
            .form(&serde_json::json!({
                "email": "admin@example.com",
                "password": "change-me-12345"
            }))
            .await;
        assert_eq!(login.status_code(), 303);

        for (title, slug, body) in [
            (
                "Rust 与 SQLite",
                "rust-sqlite",
                "用 Rust 写 SQLite 数据库和代码。",
            ),
            (
                "AI Agent 笔记",
                "ai-agent",
                "Agent 与模型、提示词和 AI 工作流。",
            ),
        ] {
            posts::Model::create_post(
                &ctx.db,
                &PostParams {
                    title: title.to_string(),
                    slug: slug.to_string(),
                    summary: body.to_string(),
                    body_md: body.to_string(),
                    status: posts::STATUS_PUBLISHED.to_string(),
                    ..PostParams::default()
                },
            )
            .await
            .unwrap();
        }

        let page = request.get("/admin/categories").await;
        assert_eq!(page.status_code(), 200);
        assert!(page.text().contains("AI 重新分类"));

        let response = request.post("/admin/api/categories/regenerate").await;
        assert_eq!(response.status_code(), 200);
        let payload: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert!(payload["assigned"].as_u64().unwrap() >= 2);
        assert!(!payload["categories"].as_array().unwrap().is_empty());

        let category = categories::Entity::find()
            .filter(categories::Column::Slug.eq("技术"))
            .one(&ctx.db)
            .await
            .unwrap()
            .expect("technology category");
        let rust_post = posts::Model::find_by_slug(&ctx.db, "rust-sqlite")
            .await
            .unwrap();
        assert_eq!(rust_post.category_id, Some(category.id));

        let category_page = request.get("/categories/ji-shu").await;
        // The slugifier may choose a different romanization; use the actual slug.
        let category = categories::Entity::find_by_id(category.id)
            .one(&ctx.db)
            .await
            .unwrap()
            .unwrap();
        let category_page = if category_page.status_code() == 200 {
            category_page
        } else {
            request.get(&format!("/categories/{}", category.slug)).await
        };
        assert_eq!(category_page.status_code(), 200);
        assert!(category_page.text().contains("Rust 与 SQLite"));

        let session = ai_sessions::Entity::find()
            .filter(ai_sessions::Column::Purpose.eq("categorize"))
            .one(&ctx.db)
            .await
            .unwrap()
            .expect("categorize session");
        assert_eq!(session.status, "succeeded");
    })
    .await;
}
