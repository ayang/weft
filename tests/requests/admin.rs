use axum_test::multipart::{MultipartForm, Part};
use loco_rs::testing::request::{request_with_config, RequestConfigBuilder};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::Value;
use serial_test::serial;
use weft::{
    app::App,
    models::{ai_sessions, embeddings, posts, users},
};

#[tokio::test]
#[serial]
async fn admin_login_post_flow() {
    let config = RequestConfigBuilder::new().save_cookies(true).build();
    request_with_config::<App, _, _>(config, |request, ctx| async move {
        let admin = users::Model::create_with_password(
            &ctx.db,
            &users::RegisterParams {
                email: "admin@example.com".to_string(),
                password: "change-me-12345".to_string(),
                name: "Admin".to_string(),
            },
        )
        .await
        .unwrap();
        assert_eq!(admin.email, "admin@example.com");

        let unauthorized = request.get("/admin").await;
        assert_eq!(unauthorized.status_code(), 303);
        assert_eq!(unauthorized.header("location"), "/admin/login");

        let bad_login = request
            .post("/admin/login")
            .form(&serde_json::json!({"email": "admin@example.com", "password": "wrong"}))
            .await;
        assert_eq!(bad_login.status_code(), 401);

        let login = request
            .post("/admin/login")
            .form(&serde_json::json!({
                "email": "admin@example.com",
                "password": "change-me-12345"
            }))
            .await;
        assert_eq!(login.status_code(), 303);
        assert_eq!(login.header("location"), "/admin");

        let dashboard = request.get("/admin").await;
        assert_eq!(dashboard.status_code(), 200);
        assert!(dashboard.text().contains("管理后台"));

        let create = request
            .post("/admin/posts")
            .form(&serde_json::json!({
                "title": "后台创建的文章",
                "slug": "admin-created",
                "summary": "后台流程验收。",
                "body_md": "# 后台\n\n这是从后台发布的正文。",
                "tags": "Rust, AI",
                "status": "draft",
                "cover_image": ""
            }))
            .await;
        assert_eq!(create.status_code(), 303);
        let post = posts::Model::find_by_slug(&ctx.db, "admin-created")
            .await
            .unwrap();
        assert_eq!(post.status, posts::STATUS_DRAFT);

        let publish = request
            .post(&format!("/admin/posts/{}/publish", post.id))
            .await;
        assert_eq!(publish.status_code(), 303);
        let published = posts::Model::find_by_id(&ctx.db, post.id).await.unwrap();
        assert_eq!(published.status, posts::STATUS_PUBLISHED);
        assert!(
            embeddings::Model::count_for_post(&ctx.db, post.id)
                .await
                .unwrap()
                > 0
        );
        assert!(!ai_sessions::Model::for_post(&ctx.db, post.id)
            .await
            .unwrap()
            .is_empty());

        let builder = request.get("/admin/charts/new").await;
        assert_eq!(builder.status_code(), 200);
        assert!(builder.text().contains("chart-builder"));
        let visual = serde_json::json!({
            "kind": "line",
            "x_field": "月份",
            "y_fields": ["发布数"],
            "options": {
                "smooth": true,
                "stack": false,
                "horizontal": false,
                "legend": true,
                "labels": false,
                "palette": "warm",
                "sort": "asc"
            },
            "columns": ["月份", "发布数"],
            "rows": [
                {"月份": "2026-08", "发布数": 5},
                {"月份": "2026-07", "发布数": 3}
            ],
            "source_type": "static",
            "sql_query": "",
            "sql_limit": 500,
            "http_url": "",
            "http_data_path": ""
        });
        let chart_create = request
            .post("/admin/charts")
            .form(&serde_json::json!({
                "post_id": post.id,
                "title": "可视化保存的图",
                "intent": "测试视觉编辑器",
                "status": "active",
                "refresh_cron": "",
                "visual_mode": "1",
                "kind": "line",
                "data_source_type": "static",
                "visual_config": visual.to_string()
            }))
            .await;
        assert_eq!(chart_create.status_code(), 303);
        let chart = weft::models::chart_specs::Entity::find()
            .filter(weft::models::chart_specs::Column::Title.eq("可视化保存的图"))
            .one(&ctx.db)
            .await
            .unwrap()
            .expect("chart should exist");
        assert_eq!(chart.kind, "line");
        assert_eq!(
            chart.data_source_config["blueprint"]["options"]["palette"],
            "warm"
        );
        let api = request.get(&format!("/api/charts/{}/data", chart.id)).await;
        assert_eq!(api.status_code(), 200);
        let api_json: Value = serde_json::from_str(&api.text()).unwrap();
        assert_eq!(api_json["data"][0]["发布数"], 3);
        assert_eq!(api_json["spec"]["series"][0]["type"], "line");

        let upload = MultipartForm::new()
            .add_text("prompt", "把上传的数据做成折线图")
            .add_text("current", "{}")
            .add_part(
                "file",
                Part::bytes("月份,数值\n1月,3\n2月,5\n".as_bytes().to_vec())
                    .file_name("chart.csv")
                    .mime_type("text/csv"),
            );
        let upload_response = request
            .post("/admin/api/charts/assist")
            .multipart(upload)
            .await;
        assert_eq!(upload_response.status_code(), 200);
        let upload_json: Value = serde_json::from_str(&upload_response.text()).unwrap();
        assert_eq!(upload_json["blueprint"]["kind"], "line");
        assert_eq!(upload_json["file"]["rows"], 2);
        assert_eq!(
            upload_json["blueprint"]["rows"].as_array().unwrap().len(),
            2
        );

        let logout = request.post("/admin/logout").await;
        assert_eq!(logout.status_code(), 303);
        assert_eq!(request.get("/admin").await.status_code(), 303);
    })
    .await;
}
