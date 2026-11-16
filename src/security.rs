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

use crate::{
    data::settings::Settings,
    models::{agent_api_keys, users},
    util::secure_eq,
};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentPrincipal {
    Anonymous,
    /// The long-lived `AGENT_API_TOKEN` from config. Kept read-only so an
    /// existing deployment token cannot silently gain write access.
    Legacy,
    /// A database-backed key created in `/admin/settings/api-keys` or by the
    /// `agent:key-create` task.
    Key,
}

pub struct AgentAuth {
    /// Rate-limit bucket. Never the plaintext key.
    pub key: String,
    pub scopes: agent_api_keys::Scopes,
    pub principal: AgentPrincipal,
    pub label: String,
}

impl AgentAuth {
    #[must_use]
    pub fn is_authenticated(&self) -> bool {
        self.principal != AgentPrincipal::Anonymous
    }

    /// Drafts, paused charts, and the media library are manager-only. A
    /// database key can read them; the legacy config token deliberately stays
    /// on published public content, matching its old behaviour.
    #[must_use]
    pub fn can_read_private(&self) -> bool {
        self.principal == AgentPrincipal::Key
    }

    /// 401 for anonymous callers, 403 for a legacy token that only has public
    /// read access, or `None` when the caller may read manager-only data.
    #[must_use]
    pub fn private_read_error(&self) -> Option<Response> {
        if self.can_read_private() {
            None
        } else if self.is_authenticated() {
            Some(agent_error(
                StatusCode::FORBIDDEN,
                "forbidden",
                "the legacy config token can only read published public content; use a database Agent Key for drafts and the media library",
            ))
        } else {
            self.anonymous_error()
        }
    }

    /// A 401 response for anonymous callers, or `None` when authenticated.
    #[must_use]
    pub fn anonymous_error(&self) -> Option<Response> {
        (!self.is_authenticated()).then(|| {
            agent_error(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "this endpoint requires an agent API key",
            )
        })
    }

    /// 401 for anonymous callers, 403 for read-only keys, or `None` when the
    /// key can write.
    #[must_use]
    pub fn write_error(&self) -> Option<Response> {
        if !self.is_authenticated() {
            Some(agent_error(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "write endpoints require an agent API key",
            ))
        } else if !self.scopes.can_write() {
            Some(agent_error(
                StatusCode::FORBIDDEN,
                "forbidden",
                "this key is read-only; create a key with the write scope",
            ))
        } else {
            None
        }
    }
}

impl FromRequestParts<AppContext> for AgentAuth {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        ctx: &AppContext,
    ) -> std::result::Result<Self, Self::Rejection> {
        let settings = Settings::from_context(ctx).map_err(|err| {
            agent_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "configuration_error",
                &err.to_string(),
            )
        })?;
        let provided = header_token(&parts.headers).unwrap_or_default();
        let provided = provided.trim();
        let legacy = settings.agents.token.trim();

        let Some(limits) = ctx.shared_store.get::<Limits>() else {
            return Err(agent_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "starting",
                "rate limiter not ready",
            ));
        };

        let (principal, scopes, label, rate_key) = if provided.is_empty() {
            if !legacy.is_empty() {
                if !limits.allow("invalid-agent-key", 60) {
                    return Err(rate_limited_response());
                }
                return Err(agent_error(
                    StatusCode::UNAUTHORIZED,
                    "unauthorized",
                    "missing agent token",
                ));
            }
            (
                AgentPrincipal::Anonymous,
                agent_api_keys::Scopes::read_only(),
                "anonymous".to_string(),
                "anonymous".to_string(),
            )
        } else if !legacy.is_empty() && secure_eq(provided, legacy) {
            (
                AgentPrincipal::Legacy,
                agent_api_keys::Scopes::read_only(),
                "config-token".to_string(),
                format!("token:{}", short_hash(provided)),
            )
        } else {
            match agent_api_keys::Model::authenticate(&ctx.db, provided).await {
                Ok(Some(key)) => {
                    if let Err(err) = key.touch(&ctx.db).await {
                        tracing::warn!(key_id = key.id, error = %err, "could not record key usage");
                    }
                    let scopes = key.scopes();
                    (
                        AgentPrincipal::Key,
                        scopes,
                        key.name.clone(),
                        format!("key:{}", key.prefix),
                    )
                }
                Ok(None) => {
                    if !limits.allow("invalid-agent-key", 60) {
                        return Err(rate_limited_response());
                    }
                    return Err(agent_error(
                        StatusCode::UNAUTHORIZED,
                        "unauthorized",
                        "invalid or revoked agent key",
                    ));
                }
                Err(err) => {
                    tracing::error!(error = %err, "agent key lookup failed");
                    return Err(agent_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "internal_error",
                        "could not verify the agent key",
                    ));
                }
            }
        };

        let limit = settings.agents.rate_limit_per_min;
        // Anonymous public reads are deliberately unrated, like the HTML
        // pages. Rate limits protect keys and the legacy token, not readers.
        if principal != AgentPrincipal::Anonymous && !limits.allow(&rate_key, limit) {
            return Err(rate_limited_response());
        }
        Ok(Self {
            key: rate_key,
            scopes,
            principal,
            label,
        })
    }
}

#[must_use]
pub fn agent_error(status: StatusCode, code: &str, message: &str) -> Response {
    (status, Json(json!({"error": code, "message": message}))).into_response()
}

fn rate_limited_response() -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(header::RETRY_AFTER, "60")],
        Json(json!({"error": "rate_limited", "message": "try again in a minute"})),
    )
        .into_response()
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
