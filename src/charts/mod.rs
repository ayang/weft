pub mod blueprint;
pub mod parse;

use std::str::FromStr;
use std::time::Duration;

use chrono::{DateTime, Utc};
use loco_rs::prelude::*;
use sea_orm::{QueryResult, Statement};
use serde_json::{json, Value};

use crate::{
    data::settings::Settings,
    models::{chart_data_caches, chart_specs},
    security::Limits,
    util,
};

/// Fetch, validate and persist a chart's data. Called from the manual refresh
/// endpoint and from `RefreshChartWorker`; never from a render path.
///
/// # Errors
/// Adapter failure, invalid JSON, timeout, or database failure.
pub async fn refresh_chart(ctx: &AppContext, chart_id: i64) -> Result<chart_specs::Model> {
    let settings = Settings::from_context(ctx)?;
    let chart = chart_specs::Model::find_by_id(&ctx.db, chart_id).await?;
    let permit = match ctx.shared_store.get::<Limits>() {
        Some(limits) => Some(limits.chart_permit().await?),
        None => None,
    };
    let started = std::time::Instant::now();
    let result = fetch(ctx, &chart, &settings).await;
    drop(permit);
    match result {
        Ok(data) => {
            let etag = util::etag_for_str(&data.to_string());
            chart_data_caches::Model::upsert(
                &ctx.db,
                chart.id,
                &data,
                &etag,
                settings.charts.cache_ttl_secs,
            )
            .await?;
            let saved = chart.record_success(&ctx.db).await?;
            tracing::info!(
                chart_id = chart.id,
                elapsed_ms = started.elapsed().as_millis(),
                source = %chart.data_source_type,
                "chart refreshed"
            );
            Ok(saved)
        }
        Err(error) => {
            let _ = chart
                .record_failure(&ctx.db, &error.to_string(), settings.jobs.max_attempts)
                .await;
            tracing::warn!(
                chart_id = chart.id,
                elapsed_ms = started.elapsed().as_millis(),
                error = %error,
                "chart refresh failed"
            );
            Err(error)
        }
    }
}

/// Fetch a chart's data without mutating status. Exposed for tests and for
/// adapter-level diagnostics.
///
/// # Errors
/// Adapter failure, invalid JSON, timeout, or oversized payload.
pub async fn fetch(
    ctx: &AppContext,
    chart: &chart_specs::Model,
    settings: &Settings,
) -> Result<Value> {
    fetch_config(
        ctx,
        &chart.data_source_type,
        &chart.data_source_config,
        settings,
    )
    .await
}

/// Fetch a data source that is not persisted yet (used by the admin builder's
/// SQL/HTTP preview).
///
/// # Errors
/// Adapter failure, invalid JSON, timeout, or oversized payload.
pub async fn fetch_config(
    ctx: &AppContext,
    source_type: &str,
    config: &Value,
    settings: &Settings,
) -> Result<Value> {
    let value = match source_type {
        "static" => fetch_static(config)?,
        "sql" => fetch_sql(ctx, config, settings).await?,
        "http" => fetch_http(ctx, config, settings).await?,
        other => {
            return Err(Error::string(&format!(
                "unsupported chart data source `{other}`"
            )));
        }
    };
    if value.to_string().len() > 2 * 1024 * 1024 {
        return Err(Error::string("chart payload exceeds 2 MiB"));
    }
    Ok(value)
}

fn fetch_static(config: &Value) -> Result<Value> {
    if let Some(data) = config.get("data") {
        return Ok(data.clone());
    }
    if let Some(rows) = config.get("rows") {
        return Ok(rows.clone());
    }
    if let Some(payload) = config.get("payload") {
        return Ok(payload.clone());
    }
    Ok(config.clone())
}

async fn fetch_sql(ctx: &AppContext, config: &Value, settings: &Settings) -> Result<Value> {
    let query = config
        .get("query")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::string("sql data source requires `query`"))?;
    validate_readonly_sql(query, &settings.charts.sql_allowed_tables)?;
    let requested = config
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(settings.charts.sql_max_rows as u64);
    let limit = usize::try_from(requested)
        .unwrap_or(settings.charts.sql_max_rows)
        .min(settings.charts.sql_max_rows)
        .max(1);
    let wrapped = format!(
        "SELECT * FROM ({}) AS chart_source LIMIT {limit}",
        query.trim().trim_end_matches(';')
    );
    let stmt = Statement::from_string(ctx.db.get_database_backend(), wrapped);
    let timeout = Duration::from_secs(settings.charts.sql_timeout_secs.max(1));
    let rows = tokio::time::timeout(timeout, ctx.db.query_all_raw(stmt))
        .await
        .map_err(|_| Error::string("sql data source timed out"))??;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let mut object = serde_json::Map::new();
        for name in row.column_names() {
            object.insert(name.clone(), cell_to_json(&row, &name));
        }
        out.push(Value::Object(object));
    }
    Ok(Value::Array(out))
}

fn cell_to_json(row: &QueryResult, name: &str) -> Value {
    if let Ok(value) = row.try_get_by::<Option<i64>, _>(name) {
        return value.map_or(Value::Null, |value| json!(value));
    }
    if let Ok(value) = row.try_get_by::<Option<f64>, _>(name) {
        return value.map_or(Value::Null, |value| json!(value));
    }
    if let Ok(value) = row.try_get_by::<Option<bool>, _>(name) {
        return value.map_or(Value::Null, |value| json!(value));
    }
    if let Ok(value) = row.try_get_by::<Option<String>, _>(name) {
        return value.map_or(Value::Null, Value::String);
    }
    Value::Null
}

/// Read-only SQL guard: one statement, SELECT only, no comment tricks,
/// no PRAGMA/ATTACH, and every referenced table must be allowlisted.
///
/// # Errors
/// Returns an error describing the first rejected construct.
pub fn validate_readonly_sql(query: &str, allowed_tables: &[String]) -> Result<()> {
    let trimmed = query.trim().trim_end_matches(';').trim();
    if trimmed.is_empty() {
        return Err(Error::string("sql query is empty"));
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains(';') {
        return Err(Error::string("multiple SQL statements are not allowed"));
    }
    if lower.contains("--") || lower.contains("/*") || lower.contains("*/") {
        return Err(Error::string("SQL comments are not allowed"));
    }
    let first = lower.split_whitespace().next().unwrap_or_default();
    if first != "select" {
        return Err(Error::string("only SELECT queries are allowed"));
    }
    for forbidden in [
        "insert",
        "update",
        "delete",
        "drop",
        "alter",
        "create",
        "replace",
        "attach",
        "detach",
        "pragma",
        "vacuum",
        "reindex",
        "load_extension",
        "sqlite_master",
        "sqlite_schema",
        "sqlite_temp_master",
        "sqlite_temp_schema",
    ] {
        if contains_keyword(&lower, forbidden) {
            return Err(Error::string(&format!(
                "forbidden keyword `{forbidden}` in SQL query"
            )));
        }
    }
    let mut found_any = false;
    for table in referenced_tables(trimmed) {
        found_any = true;
        let base = table
            .rsplit('.')
            .next()
            .unwrap_or(&table)
            .to_ascii_lowercase();
        if !allowed_tables
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(&base))
        {
            return Err(Error::string(&format!(
                "table `{base}` is not in the chart SQL allowlist"
            )));
        }
    }
    if !found_any {
        return Err(Error::string("could not identify a table in the SQL query"));
    }
    Ok(())
}

fn contains_keyword(haystack: &str, keyword: &str) -> bool {
    haystack
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .any(|word| word == keyword)
}

fn referenced_tables(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in query.chars() {
        if ch.is_alphanumeric() || matches!(ch, '_' | '.' | '$') {
            current.push(ch);
        } else {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            if matches!(ch, '(' | ')' | ',') {
                tokens.push(ch.to_string());
            }
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    let mut out = Vec::new();
    let mut in_from = false;
    let mut expect_table = false;
    let mut depth = 0_i32;
    for token in &tokens {
        let lower = token.to_ascii_lowercase();
        match lower.as_str() {
            "from" => {
                in_from = true;
                expect_table = true;
            }
            "join" => {
                expect_table = true;
            }
            "(" => {
                depth += 1;
                expect_table = false;
            }
            ")" => {
                depth = (depth - 1).max(0);
            }
            "," if in_from => {
                expect_table = true;
            }
            "where" | "group" | "order" | "limit" | "having" | "window" | "union" | "except"
            | "intersect" => {
                in_from = false;
            }
            _ => {
                if expect_table && depth >= 0 {
                    if token == "(" {
                        expect_table = false;
                    } else if is_sql_identifier(token) {
                        out.push(token.clone());
                        expect_table = false;
                    }
                }
            }
        }
    }
    out
}

fn is_sql_identifier(token: &str) -> bool {
    token
        .chars()
        .next()
        .is_some_and(|ch| ch.is_alphabetic() || ch == '_')
}

async fn fetch_http(ctx: &AppContext, config: &Value, settings: &Settings) -> Result<Value> {
    let raw_url = config
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::string("http data source requires `url`"))?;
    let url = reqwest::Url::parse(raw_url)
        .map_err(|err| Error::string(&format!("invalid http url: {err}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Error::string("only http/https URLs are allowed"));
    }
    if url.username() != "" || url.password().is_some() {
        return Err(Error::string("URLs with credentials are not allowed"));
    }
    let host = url
        .host_str()
        .ok_or_else(|| Error::string("http url has no host"))?
        .to_ascii_lowercase();
    if is_private_host(&host) {
        return Err(Error::string("private/local hosts are not allowed"));
    }
    if !host_allowed(&host, &settings.charts.http_allowlist) {
        return Err(Error::string(&format!(
            "host `{host}` is not in charts.http_allowlist"
        )));
    }

    let permit = match ctx.shared_store.get::<Limits>() {
        Some(limits) => Some(limits.http_permit().await?),
        None => None,
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(
            settings.charts.http_timeout_secs.max(1),
        ))
        .user_agent("weft-chart-fetcher/1.0")
        .build()
        .map_err(|err| Error::string(&format!("http client build failed: {err}")))?;
    let method = config
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .to_ascii_uppercase();
    let mut request = match method.as_str() {
        "GET" => client.get(url),
        "POST" => client.post(url),
        other => {
            drop(permit);
            return Err(Error::string(&format!("http method `{other}` not allowed")));
        }
    };
    request = request.header("accept", "application/json");
    if let Some(headers) = config.get("headers").and_then(Value::as_object) {
        for (name, value) in headers {
            if let Some(value) = value.as_str() {
                request = request.header(name, value);
            }
        }
    }
    let response = request
        .send()
        .await
        .map_err(|err| Error::string(&format!("http fetch failed: {err}")))?;
    if let Some(length) = response.content_length() {
        if length > settings.charts.http_max_bytes as u64 {
            drop(permit);
            return Err(Error::string("http response exceeds configured size limit"));
        }
    }
    if !response.status().is_success() {
        drop(permit);
        return Err(Error::string(&format!(
            "http data source returned status {}",
            response.status()
        )));
    }
    let mut response = response;
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| Error::string(&format!("http body read failed: {err}")))?
    {
        body.extend_from_slice(&chunk);
        if body.len() > settings.charts.http_max_bytes {
            drop(permit);
            return Err(Error::string("http response exceeds configured size limit"));
        }
    }
    drop(permit);
    let mut value: Value = serde_json::from_slice(&body)
        .map_err(|err| Error::string(&format!("http response is not JSON: {err}")))?;
    if let Some(path) = config.get("data_path").and_then(Value::as_str) {
        value = json_path(&value, path)
            .ok_or_else(|| Error::string(&format!("data_path `{path}` not found")))?
            .clone();
    }
    Ok(value)
}

fn json_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for part in path.split('.') {
        if part.is_empty() {
            continue;
        }
        current = match current {
            Value::Object(map) => map.get(part)?,
            Value::Array(array) => array.get(part.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

fn host_allowed(host: &str, allowlist: &[String]) -> bool {
    allowlist.iter().any(|entry| {
        let entry = entry.trim().to_ascii_lowercase();
        if let Some(suffix) = entry.strip_prefix("*.") {
            host == suffix || host.ends_with(&format!(".{suffix}"))
        } else if let Some(suffix) = entry.strip_prefix('.') {
            host == suffix || host.ends_with(&format!(".{suffix}"))
        } else {
            host == entry
        }
    })
}

fn is_private_host(host: &str) -> bool {
    if host == "localhost" || host.ends_with(".local") || host == "::1" {
        return true;
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(ip) => {
                ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified()
            }
            std::net::IpAddr::V6(ip) => ip.is_loopback() || ip.is_unspecified(),
        };
    }
    false
}

/// Whether a chart's refresh cron says it should run now. A future
/// `next_attempt_at` always wins (backoff).
///
/// # Errors
/// Returns an error when the stored cron expression is invalid.
pub fn is_due(chart: &chart_specs::Model, now: DateTime<Utc>) -> Result<bool> {
    if chart.status == chart_specs::STATUS_ERROR && chart.next_attempt_at.is_none() {
        // Backoff was exhausted; require a manual refresh.
        return Ok(false);
    }
    if let Some(next_attempt) = chart.next_attempt_at {
        if next_attempt.with_timezone(&Utc) > now {
            return Ok(false);
        }
    }
    let Some(expression) = chart.refresh_cron.as_deref() else {
        return Ok(false);
    };
    if expression.trim().is_empty() {
        return Ok(false);
    }
    let schedule = cron::Schedule::from_str(expression.trim())
        .map_err(|err| Error::string(&format!("invalid refresh_cron: {err}")))?;
    let last = chart.last_run_at.map(|dt| dt.with_timezone(&Utc));
    if let Some(last) = last {
        let next = schedule.after(&last).next();
        if next.is_some_and(|next| next > now) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Read the cached payload, if any. The API serves stale data while a refresh
/// is in flight; freshness is reported to the client.
///
/// # Errors
/// Database failure.
pub async fn cached(
    db: &DatabaseConnection,
    chart_id: i64,
) -> Result<Option<chart_data_caches::Model>> {
    Ok(chart_data_caches::Model::find_for_chart(db, chart_id).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_writes_and_unknown_tables() {
        let allowed = vec!["posts".to_string()];
        assert!(validate_readonly_sql("SELECT id FROM posts", &allowed).is_ok());
        assert!(validate_readonly_sql("DELETE FROM posts", &allowed).is_err());
        assert!(validate_readonly_sql("SELECT * FROM users", &allowed).is_err());
        assert!(validate_readonly_sql("SELECT 1; DROP TABLE posts", &allowed).is_err());
        assert!(validate_readonly_sql("SELECT * FROM posts -- x", &allowed).is_err());
    }

    #[test]
    fn allowlist_supports_subdomains() {
        assert!(host_allowed("api.example.com", &[".example.com".into()]));
        assert!(host_allowed("example.com", &[".example.com".into()]));
        assert!(!host_allowed("evil-example.com", &[".example.com".into()]));
    }
}
