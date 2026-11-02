use loco_rs::testing::request::{request_with_config, RequestConfigBuilder};
use serial_test::serial;
use weft::{
    ai,
    app::App,
    models::{ai_settings, users},
};

#[tokio::test]
#[serial]
async fn ai_settings_are_admin_only_and_key_is_never_echoed() {
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

        assert_eq!(request.get("/admin/settings/ai").await.status_code(), 303);

        let login = request
            .post("/admin/login")
            .form(&serde_json::json!({
                "email": "admin@example.com",
                "password": "change-me-12345"
            }))
            .await;
        assert_eq!(login.status_code(), 303);

        let page = request.get("/admin/settings/ai").await;
        assert_eq!(page.status_code(), 200);
        let html = page.text();
        assert!(html.contains("AI 设置"));
        assert!(html.contains("DeepSeek"));
        assert!(!html.contains("sk-test-secret"));

        let save = request
            .post("/admin/settings/ai")
            .form(&serde_json::json!({
                "provider": "deepseek",
                "base_url": "https://api.deepseek.com/v1",
                "model": "deepseek-chat",
                "api_key": "sk-test-secret",
                "enabled": "on"
            }))
            .await;
        assert_eq!(save.status_code(), 303);

        let row = ai_settings::Model::singleton(&ctx.db)
            .await
            .unwrap()
            .expect("settings row");
        assert_eq!(row.provider, "deepseek");
        assert_eq!(row.model, "deepseek-chat");
        assert_eq!(row.api_key.as_deref(), Some("sk-test-secret"));

        let page = request.get("/admin/settings/ai").await;
        assert_eq!(page.status_code(), 200);
        let html = page.text();
        assert!(html.contains("已配置"));
        assert!(!html.contains("sk-test-secret"));

        let effective = ai::effective_ai_settings(&ctx).await.unwrap();
        assert_eq!(effective.provider, "deepseek");
        assert_eq!(effective.model, "deepseek-chat");
        assert_eq!(effective.api_key, "sk-test-secret");

        // Clearing the key is explicit and leaves the row without a secret.
        let clear = request
            .post("/admin/settings/ai")
            .form(&serde_json::json!({
                "provider": "deepseek",
                "base_url": "https://api.deepseek.com/v1",
                "model": "deepseek-chat",
                "api_key": "",
                "clear_key": "on",
                "enabled": "on"
            }))
            .await;
        assert_eq!(clear.status_code(), 303);
        let row = ai_settings::Model::singleton(&ctx.db)
            .await
            .unwrap()
            .expect("settings row");
        assert_eq!(row.api_key.as_deref(), Some(""));
    })
    .await;
}

#[tokio::test]
#[serial]
async fn site_settings_are_editable_and_apply_without_restart() {
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

        let page = request.get("/admin/settings/site").await;
        assert_eq!(page.status_code(), 200);
        assert!(page.text().contains("站点设置"));
        assert!(page.text().contains("首页 Banner 图片"));

        let save = request
            .post("/admin/settings/site")
            .form(&serde_json::json!({
                "title": "我的站点",
                "brand": "我的品牌",
                "tagline": "写给人类与 Agent",
                "description": "一个新的个人站点",
                "author": "站长",
                "base_url": "https://example.com",
                "hero_image": "/static/hero-banner.svg",
                "posts_per_page": "7"
            }))
            .await;
        assert_eq!(save.status_code(), 303);

        let home = request.get("/").await;
        assert_eq!(home.status_code(), 200);
        let html = home.text();
        assert!(html.contains("我的站点"));
        assert!(html.contains("我的品牌"));
        assert!(html.contains("写给人类与 Agent"));
        assert!(html.contains("/static/hero-banner.svg"));
    })
    .await;
}
