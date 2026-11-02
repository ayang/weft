use loco_rs::{bgworker::BackgroundWorker, testing::prelude::*};
use serde_json::Value;
use serial_test::serial;
use weft::{
    app::App,
    models::posts::{self, PostParams},
    workers::index_post::{Worker as IndexWorker, WorkerArgs},
};

#[tokio::test]
#[serial]
async fn agent_api_is_structured_cacheable_and_cites_sources() {
    request::<App, _, _>(|request, ctx| async move {
        let post = posts::Model::create_post(
            &ctx.db,
            &PostParams {
                title: "Agent 可读的动态站点".to_string(),
                slug: "agent-readable-test".to_string(),
                summary: "动态生成 Markdown 与 JSON。".to_string(),
                body_md: "# Agent\n\n这是一个动态站点。纯 Rust 与 SQLite。".to_string(),
                status: posts::STATUS_PUBLISHED.to_string(),
                ..PostParams::default()
            },
        )
        .await
        .unwrap();
        IndexWorker::perform_later(&ctx, WorkerArgs { post_id: post.id })
            .await
            .unwrap();

        assert_eq!(
            request
                .post("/api/agent/ask")
                .json(&serde_json::json!({"question": "should not exist"}))
                .await
                .status_code(),
            404
        );

        let list = request.get("/api/agent/posts?limit=10").await;
        assert_eq!(list.status_code(), 200);
        let etag = list.header("etag").to_str().unwrap().to_string();
        let payload: Value = serde_json::from_str(&list.text()).unwrap();
        assert_eq!(payload["count"], 1);
        assert_eq!(payload["posts"][0]["slug"], "agent-readable-test");
        assert_eq!(
            request
                .get("/api/agent/posts?limit=10")
                .add_header("if-none-match", etag)
                .await
                .status_code(),
            304
        );

        let detail = request.get("/api/agent/posts/agent-readable-test").await;
        assert_eq!(detail.status_code(), 200);
        let payload: Value = serde_json::from_str(&detail.text()).unwrap();
        assert!(payload["markdown"]
            .as_str()
            .unwrap()
            .contains("slug: agent-readable-test"));
    })
    .await;
}
