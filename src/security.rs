use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Json,
};
use loco_rs::{auth::jwt::JWT, prelude::*};
use serde_json::json;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::{data::settings::Settings, models::users, util::secure_eq};

/// Per-process resource gates. Small VPS deployments cannot afford unbounded
/// background work or unbounded AI fan-out, so every expensive path takes one
/// of these permits. The rate limiter is deliberately in-memory: the target is
/// a single binary, not a fleet.
#[derive(Clone)]
pub struct Limits {
    ai: Arc<Semaphore>,
    charts: Arc<Semaphore>,
    http: Arc<Semaphore>,
    windows: Arc<Mutex<HashMap<String, VecDeque<Instant>>>>,
}

impl Limits {
    #[must_use]
    pub fn new(settings: &Settings) -> Self {
        Self {
            ai: Arc::new(Semaphore::new(settings.ai.max_concurrency.max(1))),
            charts: Arc::new(Semaphore::new(settings.charts.refresh_concurrency.max(1))),
            http: Arc::new(Semaphore::new(4)),
            windows: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// # Errors
    /// Never fails in practice; the semaphore is never closed.
    pub async fn ai_permit(&self) -> Result<OwnedSemaphorePermit> {
        self.ai
            .clone()
            .acquire_owned()
            .await
            .map_err(|err| Error::string(&format!("ai limiter closed: {err}")))
    }

    /// # Errors
    /// Never fails in practice; the semaphore is never closed.
    pub async fn chart_permit(&self) -> Result<OwnedSemaphorePermit> {
        self.charts
            .clone()
            .acquire_owned()
            .await
            .map_err(|err| Error::string(&format!("chart limiter closed: {err}")))
    }

    /// # Errors
    /// Never fails in practice; the semaphore is never closed.
    pub async fn http_permit(&self) -> Result<OwnedSemaphorePermit> {
        self.http
            .clone()
            .acquire_owned()
            .await
            .map_err(|err| Error::string(&format!("http limiter closed: {err}")))
    }

    /// Fixed-window (60s) rate limit. Returns `false` when the caller is over
    /// budget.
    pub fn allow(&self, key: &str, limit_per_min: u32) -> bool {
        if limit_per_min == 0 {
            return true;
        }
        let Ok(mut windows) = self.windows.lock() else {
            return true;
        };
        let now = Instant::now();
        let window = windows.entry(key.to_string()).or_default();
        while window
            .front()
            .is_some_and(|start| now.duration_since(*start) > Duration::from_secs(60))
        {
            window.pop_front();
        }
        if window.len() >= limit_per_min as usize {
            return false;
        }
        window.push_back(now);
        true
    }
}

pub struct AdminSession {
    pub user: users::Model,
}

impl FromRequestParts<AppContext> for AdminSession {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        ctx: &AppContext,
    ) -> std::result::Result<Self, Self::Rejection> {
        match admin_from_parts(parts, ctx).await {
            Ok(user) => Ok(Self { user }),
            Err(error) => {
                if parts.uri.path().starts_with("/admin/api") {
                    Err((
                        StatusCode::UNAUTHORIZED,
                        Json(json!({"error": "unauthorized", "message": error.to_string()})),
                    )
                        .into_response())
                } else {
                    Err(Redirect::to("/admin/login").into_response())
                }
            }
        }
    }
}

async fn admin_from_parts(parts: &mut Parts, ctx: &AppContext) -> Result<users::Model> {
    let token = token_from_parts(&parts.headers).ok_or_else(|| Error::string("missing token"))?;
    let jwt_config = ctx.config.get_jwt_config()?;
    let token_data = JWT::new(&jwt_config.secret)
        .validate(&token)
        .map_err(|_| Error::string("invalid token"))?;
    let user = users::Model::find_by_pid(&ctx.db, &token_data.claims.pid).await?;
    let settings = Settings::from_context(ctx)?;
    if let Some(admin_email) = settings.admin_email() {
        if !user.email.eq_ignore_ascii_case(admin_email) {
            return Err(Error::string("not an admin account"));
        }
    }
    Ok(user)
}

fn token_from_parts(headers: &HeaderMap) -> Option<String> {
    if let Some(value) = headers.get(header::AUTHORIZATION) {
        if let Ok(value) = value.to_str() {
            if let Some(token) = value.strip_prefix("Bearer ") {
                return Some(token.trim().to_string());
            }
        }
    }
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    cookie.split(';').find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        (name == "weft_admin").then(|| value.to_string())
    })
}

/// # Errors
/// When JWT configuration is missing or token generation fails.
pub fn issue_admin_cookie(ctx: &AppContext, user: &users::Model) -> Result<String> {
    let jwt_config = ctx.config.get_jwt_config()?;
    let token = user
        .generate_jwt(&jwt_config.secret, jwt_config.expiration)
        .map_err(|err| Error::string(&err.to_string()))?;
    let secure = if ctx
        .environment
        .to_string()
        .eq_ignore_ascii_case("production")
    {
        "; Secure"
    } else {
        ""
    };
    Ok(format!(
        "weft_admin={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{secure}",
        jwt_config.expiration
    ))
}

#[must_use]
pub fn clear_admin_cookie() -> String {
    "weft_admin=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0".to_string()
}

pub struct AgentAuth {
    pub key: String,
}

impl FromRequestParts<AppContext> for AgentAuth {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        ctx: &AppContext,
    ) -> std::result::Result<Self, Self::Rejection> {
        let settings = Settings::from_context(ctx).map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "configuration_error", "message": err.to_string()})),
            )
                .into_response()
        })?;
        let provided = header_token(&parts.headers).unwrap_or_default();
        if !settings.agents.token.trim().is_empty()
            && !secure_eq(provided.trim(), settings.agents.token.trim())
        {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "unauthorized", "message": "invalid agent token"})),
            )
                .into_response());
        }
        let key = if provided.trim().is_empty() {
            "anonymous".to_string()
        } else {
            format!("token:{}", short_hash(&provided))
        };
        let limit = settings.agents.rate_limit_per_min;
        let Some(limits) = ctx.shared_store.get::<Limits>() else {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": "starting", "message": "rate limiter not ready"})),
            )
                .into_response());
        };
        if !limits.allow(&key, limit) {
            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                [(header::RETRY_AFTER, "60")],
                Json(json!({"error": "rate_limited", "message": "try again in a minute"})),
            )
                .into_response());
        }
        Ok(Self { key })
    }
}

fn header_token(headers: &HeaderMap) -> Option<String> {
    if let Some(value) = headers.get("x-agent-token") {
        return value.to_str().ok().map(str::to_string);
    }
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    value
        .strip_prefix("Bearer ")
        .map(|token| token.trim().to_string())
}

fn short_hash(value: &str) -> String {
    crate::util::etag_for_str(value)
        .trim_matches('"')
        .chars()
        .take(12)
        .collect()
}
