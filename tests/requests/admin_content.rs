use loco_rs::testing::request::{request_with_config, RequestConfigBuilder};
use serial_test::serial;
use weft::{
    app::App,
    models::{
        posts::{self, PostParams},
        users,
    },
};

#[tokio::test]
#[serial]
async fn admin_posts_page_lists_existing_articles() {
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

        posts::Model::create_post(
            &ctx.db,
            &PostParams {
                title: "可管理的文章".to_string(),
                slug: "manageable-post".to_string(),
                summary: "用于后台管理测试".to_string(),
                body_md: "正文".to_string(),
                status: posts::STATUS_PUBLISHED.to_string(),
                ..PostParams::default()
            },
        )
        .await
        .unwrap();

        let posts_page = request.get("/admin/posts").await;
        assert_eq!(posts_page.status_code(), 200);
        assert!(posts_page.text().contains("可管理的文章"));
    })
    .await;
}
