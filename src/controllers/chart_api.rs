use loco_rs::prelude::*;
use std::time::Duration;

use crate::{
    charts,
    data::settings::Settings,
    models::{chart_specs, posts},
    util,
    views::public::ChartDataView,
};

#[debug_handler]
async fn chart_data(State(ctx): State<AppContext>, Path(id): Path<i64>) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let chart = chart_specs::Model::find_by_id(&ctx.db, id).await?;
    if chart.status == chart_specs::STATUS_PAUSED {
        return not_found();
    }
    let post = posts::Model::find_by_id(&ctx.db, chart.post_id).await?;
    if post.status != posts::STATUS_PUBLISHED {
        return not_found();
    }
    let cache = charts::cached(&ctx.db, chart.id).await?;
    let (data, etag, updated_at, stale, last_modified) = if let Some(cache) = &cache {
        (
            cache.payload.clone(),
            cache.etag.clone(),
            cache.fetched_at.to_rfc3339(),
            !cache.is_fresh(),
            cache.fetched_at,
        )
    } else {
        let etag = util::etag_for_str(&format!("{}:{}", chart.id, chart.render_config));
        (
            serde_json::Value::Null,
            etag,
            String::new(),
            true,
            chart.updated_at,
        )
    };
    if stale {
        enqueue_refresh(&ctx, &settings, chart.id).await;
    }
    let view = ChartDataView {
        chart_id: chart.id,
        title: chart.title.clone(),
        kind: chart.kind.clone(),
        spec: chart.render_config.clone(),
        data,
        updated_at,
        etag: etag.clone(),
        stale,
        status: chart.status.clone(),
        error: chart.error_message.clone().unwrap_or_default(),
    };
    format::render()
        .header(
            "cache-control",
            "public, max-age=60, stale-while-revalidate=600",
        )
        .header("x-chart-stale", if stale { "1" } else { "0" })
        .header("last-modified", util::http_date(&last_modified))
        .etag(&etag)?
        .json(view)
}

async fn enqueue_refresh(ctx: &AppContext, settings: &Settings, chart_id: i64) {
    if settings.charts.manual_refresh_only {
        return;
    }
    let key = format!("chart:lazy-refresh:{chart_id}");
    if matches!(ctx.cache.contains_key(&key).await, Ok(true)) {
        return;
    }
    let _ = ctx
        .cache
        .insert_with_expiry(&key, &true, Duration::from_secs(60))
        .await;
    let args = crate::workers::refresh_chart::WorkerArgs { chart_id };
    if let Err(error) = crate::workers::refresh_chart::Worker::perform_later(ctx, args).await {
        tracing::warn!(chart_id, error = %error, "lazy chart refresh enqueue failed");
    }
}

pub fn routes() -> Routes {
    Routes::new().add("/api/charts/{id}/data", get(chart_data))
}
