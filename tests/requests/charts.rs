use loco_rs::testing::prelude::*;
use serde_json::{json, Value};
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
            title: "图表验收".to_string(),
            slug: "chart-acceptance".to_string(),
            summary: "图表动态数据".to_string(),
            body_md: "# 图表\n\n正文。".to_string(),
            status: posts::STATUS_PUBLISHED.to_string(),
            ..PostParams::default()
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
#[serial]
async fn static_and_sql_charts_refresh_and_serve_etag() {
    request::<App, _, _>(|request, ctx| async move {
        let post = seed_post(&ctx).await;
        let static_chart = chart_specs::Model::create_chart(
            &ctx.db,
            post.id,
            &ChartParams {
                title: "静态图表".to_string(),
                kind: "bar".to_string(),
                data_source_type: "static".to_string(),
                data_source_config: json!({
                    "data": [{"label": "A", "value": 1}, {"label": "B", "value": 2}]
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
        charts::refresh_chart(&ctx, static_chart.id).await.unwrap();

        let url = format!("/api/charts/{}/data", static_chart.id);
        let response = request.get(&url).await;
        assert_eq!(response.status_code(), 200);
        let payload: Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(payload["data"][0]["label"], "A");
        assert_eq!(payload["stale"], false);
        let etag = response.header("etag").to_str().unwrap().to_string();
        assert_eq!(
            request
                .get(&url)
                .add_header("if-none-match", etag)
                .await
                .status_code(),
            304
        );

        let sql_chart = chart_specs::Model::create_chart(
            &ctx.db,
            post.id,
            &ChartParams {
                title: "SQL 图表".to_string(),
                kind: "bar".to_string(),
                data_source_type: "sql".to_string(),
                data_source_config: json!({
                    "query": "SELECT status AS label, COUNT(*) AS value FROM posts GROUP BY status"
                })
                .to_string(),
                render_config: json!({"dataset": {"source": []}, "series": [{"type": "bar"}]})
                    .to_string(),
                status: chart_specs::STATUS_ACTIVE.to_string(),
                ..ChartParams::default()
            },
        )
        .await
        .unwrap();
        charts::refresh_chart(&ctx, sql_chart.id).await.unwrap();
        let response = request
            .get(&format!("/api/charts/{}/data", sql_chart.id))
            .await;
        let payload: Value = serde_json::from_str(&response.text()).unwrap();
        assert_eq!(payload["data"][0]["label"], "published");

        let bad_chart = chart_specs::Model::create_chart(
            &ctx.db,
            post.id,
            &ChartParams {
                title: "危险 SQL".to_string(),
                kind: "bar".to_string(),
                data_source_type: "sql".to_string(),
                data_source_config: json!({"query": "DELETE FROM posts"}).to_string(),
                render_config: json!({"series": []}).to_string(),
                status: chart_specs::STATUS_ACTIVE.to_string(),
                ..ChartParams::default()
            },
        )
        .await
        .unwrap();
        assert!(charts::refresh_chart(&ctx, bad_chart.id).await.is_err());
        let failed = chart_specs::Model::find_by_id(&ctx.db, bad_chart.id)
            .await
            .unwrap();
        assert_eq!(failed.status, chart_specs::STATUS_ERROR);
        assert!(failed
            .error_message
            .unwrap_or_default()
            .contains("only SELECT"));
    })
    .await;
}

#[test]
fn sql_guard_handles_comma_joins_and_subqueries() {
    let allowed = vec!["posts".to_string()];
    assert!(charts::validate_readonly_sql("SELECT p.id FROM posts p, users u", &allowed).is_err());
    assert!(
        charts::validate_readonly_sql("SELECT * FROM (SELECT * FROM users) x", &allowed).is_err()
    );
    assert!(charts::validate_readonly_sql(
        "SELECT * FROM posts WHERE id IN (SELECT id FROM users)",
        &allowed
    )
    .is_err());
}
