use std::io::{Cursor, Read};

use axum_test::multipart::{MultipartForm, Part};
use loco_rs::testing::request::{request, request_with_config, RequestConfigBuilder};
use serde_json::{json, Value};
use serial_test::serial;
use weft::{
    app::App,
    models::{
        agent_api_keys::{self, Scopes},
        posts, users,
    },
};

fn bearer(key: &str) -> (&'static str, String) {
    ("authorization", format!("Bearer {key}"))
}

async fn create_key(ctx: &loco_rs::app::AppContext, scopes: Scopes) -> String {
    let (_, key) = agent_api_keys::Model::create_key(&ctx.db, "test-agent", scopes)
        .await
        .unwrap();
    key
}

#[tokio::test]
#[serial]
async fn agent_can_create_edit_publish_and_delete_posts_and_charts() {
    request::<App, _, _>(|request, ctx| async move {
        let key = create_key(&ctx, Scopes::read_write()).await;
        let (auth_name, auth_value) = bearer(&key);

        let anonymous_me = request.get("/api/agent/me").await;
        assert_eq!(anonymous_me.status_code(), 200);
        let anonymous_me: Value = serde_json::from_str(&anonymous_me.text()).unwrap();
        assert_eq!(anonymous_me["authenticated"], false);

        let me = request
            .get("/api/agent/me")
            .add_header(auth_name, auth_value.clone())
            .await;
        assert_eq!(me.status_code(), 200);
        let me: Value = serde_json::from_str(&me.text()).unwrap();
        assert_eq!(me["authenticated"], true);
        assert_eq!(me["scopes"]["write"], true);

        let create = request
            .post("/api/agent/posts")
            .add_header(auth_name, auth_value.clone())
            .json(&json!({
                "title": "Agent 写入测试",
                "slug": "agent-write-test",
                "summary": "由外部 Agent 创建。",
                "body_md": "# Hello\n\n这是正文。",
                "status": "draft",
                "pinned": false,
                "cover_image": "",
                "tags": ["Rust", "AI"]
            }))
            .await;
        assert_eq!(create.status_code(), 201, "create: {}", create.text());
        let created: Value = serde_json::from_str(&create.text()).unwrap();
        assert_eq!(created["post"]["slug"], "agent-write-test");
        assert_eq!(created["post"]["status"], "draft");
        assert_eq!(created["body_md"], "# Hello\n\n这是正文。");
        assert_eq!(created["post"]["tags"][0]["name"], "Rust");
        let updated_at = created["post"]["updated_at"].as_str().unwrap().to_string();

        // Drafts are invisible without a key.
        assert_eq!(
            request
                .get("/api/agent/posts/agent-write-test")
                .await
                .status_code(),
            404
        );
        let draft_detail = request
            .get("/api/agent/posts/agent-write-test")
            .add_header(auth_name, auth_value.clone())
            .await;
        assert_eq!(draft_detail.status_code(), 200);

        // Draft listing requires authentication.
        assert_eq!(
            request
                .get("/api/agent/posts?status=draft")
                .await
                .status_code(),
            401
        );
        let drafts = request
            .get("/api/agent/posts?status=draft")
            .add_header(auth_name, auth_value.clone())
            .await;
        assert_eq!(drafts.status_code(), 200);
        let drafts_payload: Value = serde_json::from_str(&drafts.text()).unwrap();
        assert_eq!(drafts_payload["count"], 1);

        let chart = request
            .post("/api/agent/posts/agent-write-test/charts")
            .add_header(auth_name, auth_value.clone())
            .json(&json!({
                "title": "发布趋势",
                "kind": "line",
                "data": [
                    {"月份": "1月", "发布数": 3},
                    {"月份": "2月", "发布数": 5}
                ],
                "x_field": "月份",
                "y_fields": ["发布数"]
            }))
            .await;
        assert_eq!(chart.status_code(), 201, "chart: {}", chart.text());
        let chart_payload: Value = serde_json::from_str(&chart.text()).unwrap();
        let chart_id = chart_payload["chart_id"].as_i64().unwrap();
        assert_eq!(chart_payload["data"].as_array().unwrap().len(), 2);
        assert!(!chart_payload["spec"]["series"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(chart_payload["marker"], format!("[[chart:{chart_id}]]"));

        let chart_update = request
            .patch(&format!("/api/agent/charts/{chart_id}"))
            .add_header(auth_name, auth_value.clone())
            .json(&json!({
                "title": "发布趋势 v2",
                "options": {"smooth": true, "labels": true}
            }))
            .await;
        assert_eq!(
            chart_update.status_code(),
            200,
            "chart update: {}",
            chart_update.text()
        );
        let chart_updated: Value = serde_json::from_str(&chart_update.text()).unwrap();
        assert_eq!(chart_updated["title"], "发布趋势 v2");
        assert_eq!(chart_updated["spec"]["series"][0]["smooth"], true);
        assert_eq!(chart_updated["spec"]["series"][0]["label"]["show"], true);

        let advanced = request
            .post("/api/agent/posts/agent-write-test/charts")
            .add_header(auth_name, auth_value.clone())
            .json(&json!({
                "title": "高级图",
                "kind": "bar",
                "data_source_type": "static",
                "data_source_config": {"data": [{"x": 1, "y": 2}]},
                "render_config": {
                    "xAxis": {"type": "category"},
                    "yAxis": {"type": "value"},
                    "series": [{"type": "bar", "data": [2]}]
                }
            }))
            .await;
        assert_eq!(advanced.status_code(), 201, "advanced: {}", advanced.text());
        let advanced_payload: Value = serde_json::from_str(&advanced.text()).unwrap();
        assert_eq!(advanced_payload["spec"]["series"][0]["type"], "bar");
        let advanced_id = advanced_payload["chart_id"].as_i64().unwrap();

        let advanced_update = request
            .patch(&format!("/api/agent/charts/{advanced_id}"))
            .add_header(auth_name, auth_value.clone())
            .json(&json!({"title": "高级图 v2"}))
            .await;
        assert_eq!(
            advanced_update.status_code(),
            200,
            "advanced update: {}",
            advanced_update.text()
        );
        let advanced_updated: Value = serde_json::from_str(&advanced_update.text()).unwrap();
        assert_eq!(advanced_updated["title"], "高级图 v2");
        assert_eq!(advanced_updated["spec"]["series"][0]["type"], "bar");

        let sql_chart = request
            .post("/api/agent/posts/agent-write-test/charts")
            .add_header(auth_name, auth_value.clone())
            .json(&json!({
                "title": "SQL 图",
                "kind": "line",
                "data_source_type": "sql",
                "sql_query": "SELECT slug, reading_minutes FROM posts",
                "sql_limit": 10,
                "columns": ["slug", "reading_minutes"],
                "x_field": "slug",
                "y_fields": ["reading_minutes"]
            }))
            .await;
        assert_eq!(
            sql_chart.status_code(),
            201,
            "sql chart: {}",
            sql_chart.text()
        );
        let sql_payload: Value = serde_json::from_str(&sql_chart.text()).unwrap();
        let sql_id = sql_payload["chart_id"].as_i64().unwrap();

        let sql_update = request
            .patch(&format!("/api/agent/charts/{sql_id}"))
            .add_header(auth_name, auth_value.clone())
            .json(&json!({"options": {"smooth": true, "legend": false}}))
            .await;
        assert_eq!(
            sql_update.status_code(),
            200,
            "sql update: {}",
            sql_update.text()
        );
        let sql_updated: Value = serde_json::from_str(&sql_update.text()).unwrap();
        assert_eq!(
            sql_updated["spec"]["series"][0]["smooth"],
            true,
            "{}",
            sql_update.text()
        );
        assert_eq!(sql_updated["spec"]["legend"]["show"], false);

        let listed = request
            .get("/api/agent/posts/agent-write-test/charts")
            .add_header(auth_name, auth_value.clone())
            .await;
        assert_eq!(listed.status_code(), 200);
        let listed_payload: Value = serde_json::from_str(&listed.text()).unwrap();
        assert_eq!(listed_payload["count"], 3);

        let refreshed = request
            .post(&format!("/api/agent/charts/{sql_id}/refresh"))
            .add_header(auth_name, auth_value.clone())
            .await;
        assert_eq!(
            refreshed.status_code(),
            200,
            "refresh: {}",
            refreshed.text()
        );

        let advanced_render = request
            .patch(&format!("/api/agent/charts/{advanced_id}"))
            .add_header(auth_name, auth_value.clone())
            .json(&json!({
                "render_config": {
                    "xAxis": {"type": "category"},
                    "yAxis": {"type": "value"},
                    "series": [{"type": "line", "data": [3]}]
                }
            }))
            .await;
        assert_eq!(
            advanced_render.status_code(),
            200,
            "advanced render update: {}",
            advanced_render.text()
        );
        let advanced_rendered: Value = serde_json::from_str(&advanced_render.text()).unwrap();
        assert_eq!(advanced_rendered["spec"]["series"][0]["type"], "line");

        let mixed = request
            .patch(&format!("/api/agent/charts/{sql_id}"))
            .add_header(auth_name, auth_value.clone())
            .json(&json!({"render_config": {"series": []}}))
            .await;
        assert_eq!(mixed.status_code(), 400);

        let chart_get = request
            .get(&format!("/api/agent/charts/{chart_id}"))
            .add_header(auth_name, auth_value.clone())
            .await;
        assert_eq!(chart_get.status_code(), 200);
        let chart_etag = chart_get.header("etag").to_str().unwrap().to_string();
        assert_eq!(
            request
                .get(&format!("/api/agent/charts/{chart_id}"))
                .add_header("if-none-match", chart_etag.clone())
                .add_header(auth_name, auth_value.clone())
                .await
                .status_code(),
            304
        );
        let title_only = request
            .patch(&format!("/api/agent/charts/{chart_id}"))
            .add_header(auth_name, auth_value.clone())
            .json(&json!({"title": "发布趋势 v3"}))
            .await;
        assert_eq!(title_only.status_code(), 200);
        let after_title: Value = serde_json::from_str(&title_only.text()).unwrap();
        assert_eq!(after_title["title"], "发布趋势 v3");
        let refreshed_get = request
            .get(&format!("/api/agent/charts/{chart_id}"))
            .add_header("if-none-match", chart_etag)
            .add_header(auth_name, auth_value.clone())
            .await;
        assert_eq!(refreshed_get.status_code(), 200);
        let refreshed_payload: Value = serde_json::from_str(&refreshed_get.text()).unwrap();
        assert_eq!(refreshed_payload["title"], "发布趋势 v3");

        let sql_title = request
            .patch(&format!("/api/agent/charts/{sql_id}"))
            .add_header(auth_name, auth_value.clone())
            .json(&json!({"title": "SQL 图 v2"}))
            .await;
        assert_eq!(
            sql_title.status_code(),
            200,
            "sql title: {}",
            sql_title.text()
        );
        let sql_titled: Value = serde_json::from_str(&sql_title.text()).unwrap();
        assert_eq!(sql_titled["title"], "SQL 图 v2");
        assert!(
            !sql_titled["data"].is_null(),
            "title-only PATCH should keep the SQL chart cache: {}",
            sql_title.text()
        );

        let paused = request
            .post("/api/agent/posts/agent-write-test/charts")
            .add_header(auth_name, auth_value.clone())
            .json(&json!({
                "title": "暂停图",
                "kind": "bar",
                "data": [{"月份": "1月", "数量": 1}],
                "x_field": "月份",
                "y_fields": ["数量"],
                "status": "paused"
            }))
            .await;
        assert_eq!(paused.status_code(), 201, "paused: {}", paused.text());
        let paused_payload: Value = serde_json::from_str(&paused.text()).unwrap();
        assert_eq!(paused_payload["status"], "paused");
        let paused_id = paused_payload["chart_id"].as_i64().unwrap();
        let paused_refresh = request
            .post(&format!("/api/agent/charts/{paused_id}/refresh"))
            .add_header(auth_name, auth_value.clone())
            .await;
        assert_eq!(
            paused_refresh.status_code(),
            200,
            "paused refresh: {}",
            paused_refresh.text()
        );
        let paused_refreshed: Value = serde_json::from_str(&paused_refresh.text()).unwrap();
        assert_eq!(paused_refreshed["status"], "paused");

        let append = request
            .patch("/api/agent/posts/agent-write-test")
            .add_header(auth_name, auth_value.clone())
            .json(&json!({
                "body_md": format!("\n## 图表\n\n[[chart:{chart_id}]]\n"),
                "body_mode": "append",
                "expected_updated_at": updated_at
            }))
            .await;
        assert_eq!(append.status_code(), 200, "append: {}", append.text());
        let appended: Value = serde_json::from_str(&append.text()).unwrap();
        let body = appended["body_md"].as_str().unwrap();
        assert!(body.starts_with("# Hello"));
        assert!(body.contains(&format!("[[chart:{chart_id}]]")));

        // Reusing the old timestamp must not overwrite the newer edit.
        let conflict = request
            .patch("/api/agent/posts/agent-write-test")
            .add_header(auth_name, auth_value.clone())
            .json(&json!({
                "title": "stale title",
                "expected_updated_at": updated_at
            }))
            .await;
        assert_eq!(conflict.status_code(), 409);

        let publish = request
            .patch("/api/agent/posts/agent-write-test")
            .add_header(auth_name, auth_value.clone())
            .json(&json!({"status": "published"}))
            .await;
        assert_eq!(publish.status_code(), 200, "publish: {}", publish.text());
        let published: Value = serde_json::from_str(&publish.text()).unwrap();
        assert_eq!(published["post"]["status"], "published");
        assert!(!published["post"]["published_at"]
            .as_str()
            .unwrap()
            .is_empty());
        assert_eq!(
            request.get("/posts/agent-write-test").await.status_code(),
            200
        );
        assert_eq!(
            request
                .get("/api/agent/posts/agent-write-test")
                .await
                .status_code(),
            200
        );

        let published_updated_at = published["post"]["updated_at"]
            .as_str()
            .unwrap()
            .to_string();
        let edit_published = request
            .patch("/api/agent/posts/agent-write-test")
            .add_header(auth_name, auth_value.clone())
            .json(&json!({
                "title": "Agent 写入测试（已发布修改）",
                "expected_updated_at": published_updated_at
            }))
            .await;
        assert_eq!(
            edit_published.status_code(),
            200,
            "edit published: {}",
            edit_published.text()
        );
        let edited: Value = serde_json::from_str(&edit_published.text()).unwrap();
        assert_eq!(edited["post"]["title"], "Agent 写入测试（已发布修改）");
        assert_eq!(edited["post"]["status"], "published");

        let delete_chart = request
            .delete(&format!("/api/agent/charts/{chart_id}"))
            .add_header(auth_name, auth_value.clone())
            .await;
        assert_eq!(delete_chart.status_code(), 204);
        assert_eq!(
            request
                .get(&format!("/api/agent/charts/{chart_id}"))
                .await
                .status_code(),
            404
        );

        let delete_post = request
            .delete("/api/agent/posts/agent-write-test")
            .add_header(auth_name, auth_value)
            .await;
        assert_eq!(delete_post.status_code(), 204);
        assert_eq!(
            request
                .get("/api/agent/posts/agent-write-test")
                .await
                .status_code(),
            404
        );
        assert!(posts::Model::find_by_slug(&ctx.db, "agent-write-test")
            .await
            .is_err());
    })
    .await;
}

#[tokio::test]
#[serial]
async fn read_only_keys_cannot_write_and_revocation_is_immediate() {
    request::<App, _, _>(|request, ctx| async move {
        let read_key = create_key(&ctx, Scopes::read_only()).await;
        let (auth_name, auth_value) = bearer(&read_key);

        let anonymous = request
            .post("/api/agent/posts")
            .json(&json!({"title": "nope", "body_md": "nope"}))
            .await;
        assert_eq!(anonymous.status_code(), 401);
        assert_eq!(
            request
                .get("/api/agent/posts?since=not-a-date")
                .await
                .status_code(),
            400
        );

        let denied = request
            .post("/api/agent/posts")
            .add_header(auth_name, auth_value.clone())
            .json(&json!({"title": "nope", "body_md": "nope"}))
            .await;
        assert_eq!(denied.status_code(), 403);

        let denied_media = request
            .post("/api/agent/media")
            .add_header(auth_name, auth_value.clone())
            .multipart(
                MultipartForm::new().add_part(
                    "file",
                    Part::bytes(b"\x89PNG\r\n\x1a\nread-only".to_vec())
                        .file_name("cover.png")
                        .mime_type("image/png"),
                ),
            )
            .await;
        assert_eq!(denied_media.status_code(), 403);

        let list = request
            .get("/api/agent/posts?status=all")
            .add_header(auth_name, auth_value.clone())
            .await;
        assert_eq!(list.status_code(), 200);

        assert_eq!(
            request
                .get("/api/agent/posts")
                .add_header("authorization", "Bearer weft_deadbeef_not-a-real-key")
                .await
                .status_code(),
            401
        );

        let model = agent_api_keys::Model::authenticate(&ctx.db, &read_key)
            .await
            .unwrap()
            .unwrap();
        model.revoke(&ctx.db).await.unwrap();
        assert_eq!(
            request
                .get("/api/agent/posts?status=all")
                .add_header(auth_name, auth_value)
                .await
                .status_code(),
            401
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn agent_can_upload_media_and_use_the_returned_markdown() {
    request::<App, _, _>(|request, ctx| async move {
        let key = create_key(&ctx, Scopes::read_write()).await;
        let (auth_name, auth_value) = bearer(&key);

        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(b"agent-media-test");
        let upload = request
            .post("/api/agent/media")
            .add_header(auth_name, auth_value.clone())
            .multipart(
                MultipartForm::new().add_part(
                    "file",
                    Part::bytes(png)
                        .file_name("agent-cover.png")
                        .mime_type("image/png"),
                ),
            )
            .await;
        assert_eq!(upload.status_code(), 201, "upload: {}", upload.text());
        let payload: Value = serde_json::from_str(&upload.text()).unwrap();
        assert_eq!(payload["kind"], "image");
        let url = payload["url"].as_str().unwrap().to_string();
        let filename = payload["filename"].as_str().unwrap().to_string();
        assert!(payload["markdown"]
            .as_str()
            .unwrap()
            .starts_with("![agent-cover.png]("));
        assert_eq!(request.get(&url).await.status_code(), 200);

        let library = request
            .get("/api/agent/media?q=agent-cover")
            .add_header(auth_name, auth_value)
            .await;
        assert_eq!(library.status_code(), 200);
        let library_payload: Value = serde_json::from_str(&library.text()).unwrap();
        assert_eq!(library_payload["count"], 1);

        let _ = std::fs::remove_file(weft::media::upload_dir().join(filename));
    })
    .await;
}

#[tokio::test]
#[serial]
async fn admin_can_create_and_revoke_a_key_from_the_ui() {
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

        let protected = request.get("/admin/settings/api-keys/skill.zip").await;
        assert_eq!(protected.status_code(), 303);

        let login = request
            .post("/admin/login")
            .form(&json!({
                "email": "admin@example.com",
                "password": "change-me-12345"
            }))
            .await;
        assert_eq!(login.status_code(), 303);

        let page = request.get("/admin/settings/api-keys").await;
        assert_eq!(page.status_code(), 200);
        assert!(page.text().contains("Agent API"));
        assert!(page.text().contains("下载 Skill 包"));
        assert!(page.text().contains("/admin/settings/api-keys/skill.zip"));

        let bundle = request.get("/admin/settings/api-keys/skill.zip").await;
        assert_eq!(bundle.status_code(), 200);
        assert_eq!(bundle.header("content-type"), "application/zip");
        assert!(bundle
            .header("content-disposition")
            .to_str()
            .unwrap()
            .contains("weft-content-skill.zip"));
        let bytes = bundle.as_bytes().to_vec();
        assert!(bytes.starts_with(b"PK"));
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let names = (0..archive.len())
            .map(|index| archive.by_index(index).unwrap().name().unwrap().to_string())
            .collect::<Vec<_>>();
        for expected in [
            "weft-content/SKILL.md",
            "weft-content/README.md",
            "weft-content/references/agent-api.md",
            "weft-content/references/agent-openapi.yaml",
            "weft-content/env.example",
            "weft-content/bundle.json",
        ] {
            assert!(
                names.iter().any(|name| name == expected),
                "missing {expected}"
            );
        }
        let mut env = String::new();
        archive
            .by_name("weft-content/env.example")
            .unwrap()
            .read_to_string(&mut env)
            .unwrap();
        assert!(env.contains("WEFT_BASE_URL=http://localhost:5150/api/agent"));
        let mut skill = String::new();
        archive
            .by_name("weft-content/SKILL.md")
            .unwrap()
            .read_to_string(&mut skill)
            .unwrap();
        assert!(skill.contains("references/agent-api.md"));
        let mut metadata = String::new();
        archive
            .by_name("weft-content/bundle.json")
            .unwrap()
            .read_to_string(&mut metadata)
            .unwrap();
        let metadata: Value = serde_json::from_str(&metadata).unwrap();
        assert_eq!(metadata["name"], "weft-content");
        assert_eq!(metadata["api_base_url"], "http://localhost:5150/api/agent");

        let create = request
            .post("/admin/settings/api-keys")
            .form(&json!({"name": "ui-key", "read": "on", "write": "on"}))
            .await;
        assert_eq!(create.status_code(), 200, "create: {}", create.text());
        let html = create.text();
        assert!(html.contains("ui-key"));
        let full_key =
            regex::Regex::new(r"weft_[0-9a-f]{8}_[0-9a-f]{64}").expect("valid key regex");
        let plaintext = full_key
            .find(&html)
            .expect("one-time plaintext key in create response")
            .as_str()
            .to_string();
        assert!(!agent_api_keys::Model::all_for_admin(&ctx.db)
            .await
            .unwrap()
            .is_empty());
        let stored = agent_api_keys::Model::all_for_admin(&ctx.db)
            .await
            .unwrap()
            .remove(0);
        assert_eq!(stored.key_hash, agent_api_keys::hash_key(&plaintext));
        assert_ne!(stored.key_hash, plaintext);

        let reload = request.get("/admin/settings/api-keys").await;
        assert_eq!(reload.status_code(), 200);
        assert!(!full_key.is_match(&reload.text()));

        let key = agent_api_keys::Model::all_for_admin(&ctx.db)
            .await
            .unwrap()
            .remove(0);
        let revoke = request
            .post(&format!("/admin/settings/api-keys/{}/revoke", key.id))
            .await;
        assert_eq!(revoke.status_code(), 303);
        let revoked = agent_api_keys::Model::find_by_id(&ctx.db, key.id)
            .await
            .unwrap();
        assert!(revoked.is_revoked());
    })
    .await;
}
