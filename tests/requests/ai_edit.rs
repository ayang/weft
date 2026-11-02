use axum_test::multipart::{MultipartForm, Part};
use loco_rs::testing::request::{request_with_config, RequestConfigBuilder};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serial_test::serial;
use weft::{
    app::App,
    models::{ai_sessions, users},
};

#[tokio::test]
#[serial]
async fn article_edit_returns_diffable_offline_proposal_and_warns_on_private_urls() {
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

        let upload = MultipartForm::new()
            .add_text("title", "访谈之后")
            .add_text("prompt", "把访谈材料整理成文章，保持原有结构")
            .add_text("body_md", "# 访谈之后\n\n原来的正文。\n")
            .add_text("urls", "http://127.0.0.1:1/secret")
            .add_text("post_id", "0")
            .add_part(
                "file",
                Part::bytes("# 访谈提纲\n\n受访者提到三个关键变化。".as_bytes().to_vec())
                    .file_name("notes.md")
                    .mime_type("text/markdown"),
            );
        let response = request
            .post("/admin/api/ai/edit-article")
            .multipart(upload)
            .await;
        assert_eq!(response.status_code(), 200);
        let payload: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(payload["offline"], true);
        assert_eq!(payload["changed"], true);
        assert!(payload["proposed"].as_str().unwrap().contains("参考材料"));
        assert!(payload["proposed"].as_str().unwrap().contains("访谈提纲"));
        assert!(payload["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| { warning.as_str().unwrap_or_default().contains("内网") }));
        assert_eq!(payload["original"], "# 访谈之后\n\n原来的正文。\n");

        let session = ai_sessions::Entity::find()
            .filter(ai_sessions::Column::Purpose.eq("article_edit"))
            .one(&ctx.db)
            .await
            .unwrap()
            .expect("article edit session");
        assert_eq!(session.status, "succeeded");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn article_create_mode_can_start_from_an_empty_body() {
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

        let upload = MultipartForm::new()
            .add_text("mode", "create")
            .add_text("title", "")
            .add_text("prompt", "写一篇关于个人站点如何同时服务人和 Agent 的文章")
            .add_text("body_md", "")
            .add_text("urls", "")
            .add_text("post_id", "")
            .add_part(
                "file",
                Part::bytes(
                    "访谈材料：读者越来越常通过 AI 搜索找到文章。"
                        .as_bytes()
                        .to_vec(),
                )
                .file_name("interview.md")
                .mime_type("text/markdown"),
            );
        let response = request
            .post("/admin/api/ai/edit-article")
            .multipart(upload)
            .await;
        assert_eq!(response.status_code(), 200);
        let payload: serde_json::Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(payload["mode"], "create");
        assert_eq!(payload["offline"], true);
        assert!(payload["changed"].as_bool().unwrap());
        assert!(!payload["title"].as_str().unwrap().trim().is_empty());
        assert!(!payload["summary"].as_str().unwrap().trim().is_empty());
        assert!(!payload["tags"].as_array().unwrap().is_empty());
        assert!(payload["proposed"].as_str().unwrap().starts_with('#'));
        assert!(payload["proposed"].as_str().unwrap().contains("访谈材料"));

        let session = ai_sessions::Entity::find()
            .filter(ai_sessions::Column::Purpose.eq("article_create"))
            .one(&ctx.db)
            .await
            .unwrap()
            .expect("article create session");
        assert_eq!(session.status, "succeeded");
    })
    .await;
}
