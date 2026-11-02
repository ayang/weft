use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};

/// Application settings declared under the free-form `settings:` key in
/// `config/*.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    pub site: SiteSettings,
    pub ai: AiSettings,
    pub agents: AgentSettings,
    pub charts: ChartSettings,
    pub jobs: JobSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SiteSettings {
    pub title: String,
    pub brand: String,
    pub tagline: String,
    pub description: String,
    pub author: String,
    pub base_url: String,
    /// Homepage banner image URL; empty falls back to the bundled banner.
    pub hero_image: String,
    /// The only account allowed into `/admin`. Empty means "any user".
    pub admin_email: String,
    pub posts_per_page: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    /// `offline` produces deterministic, extractive output and never calls the
    /// network. Any other value is treated as an OpenAI-compatible provider.
    pub provider: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub timeout_secs: u64,
    pub max_output_tokens: u32,
    /// Longer output budget for whole-article edit/create requests.
    pub article_max_output_tokens: u32,
    pub temperature: f32,
    pub daily_cost_limit_usd: f64,
    pub input_price_per_million: f64,
    pub output_price_per_million: f64,
    pub max_concurrency: usize,
    pub embedding_model: String,
    pub embeddings_enabled: bool,
}

impl AiSettings {
    #[must_use]
    pub fn is_offline(&self) -> bool {
        self.provider.eq_ignore_ascii_case("offline")
            || (self.api_key.trim().is_empty()
                && !self.provider.eq_ignore_ascii_case("ollama")
                && !self.provider.eq_ignore_ascii_case("local"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentSettings {
    /// Optional read-only bearer token for `/api/agent/*`.
    pub token: String,
    pub rate_limit_per_min: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ChartSettings {
    pub http_allowlist: Vec<String>,
    pub http_timeout_secs: u64,
    pub http_max_bytes: usize,
    pub sql_timeout_secs: u64,
    pub sql_allowed_tables: Vec<String>,
    pub sql_max_rows: usize,
    /// How long a successfully fetched payload stays fresh.
    pub cache_ttl_secs: u64,
    /// How long a stale payload may still be served before a refresh is due.
    pub stale_ttl_secs: u64,
    pub refresh_concurrency: usize,
    pub manual_refresh_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct JobSettings {
    pub max_attempts: i64,
    pub page_size: i64,
}

impl Default for SiteSettings {
    fn default() -> Self {
        Self {
            title: "Weft".to_string(),
            brand: String::new(),
            tagline: "人与 AI 的编织现场".to_string(),
            description: "一个人在写作、图表与 AI 之间编织的个人站点".to_string(),
            author: "Weft".to_string(),
            base_url: "http://localhost:5150".to_string(),
            hero_image: String::new(),
            admin_email: "admin@example.com".to_string(),
            posts_per_page: 20,
        }
    }
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            provider: "offline".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: String::new(),
            model: "gpt-4o-mini".to_string(),
            timeout_secs: 45,
            max_output_tokens: 900,
            article_max_output_tokens: 4_000,
            temperature: 0.2,
            daily_cost_limit_usd: 1.0,
            input_price_per_million: 0.15,
            output_price_per_million: 0.60,
            max_concurrency: 1,
            embedding_model: "text-embedding-3-small".to_string(),
            embeddings_enabled: false,
        }
    }
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            token: String::new(),
            rate_limit_per_min: 30,
        }
    }
}

impl Default for ChartSettings {
    fn default() -> Self {
        Self {
            http_allowlist: Vec::new(),
            http_timeout_secs: 15,
            http_max_bytes: 2_000_000,
            sql_timeout_secs: 10,
            sql_allowed_tables: vec![
                "posts".to_string(),
                "tags".to_string(),
                "post_tags".to_string(),
                "chart_specs".to_string(),
                "chart_data_caches".to_string(),
                "embeddings".to_string(),
            ],
            sql_max_rows: 500,
            cache_ttl_secs: 300,
            stale_ttl_secs: 86_400,
            refresh_concurrency: 1,
            manual_refresh_only: false,
        }
    }
}

impl Default for JobSettings {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            page_size: 20,
        }
    }
}

impl Settings {
    /// # Errors
    /// Returns an error when the `settings:` block is missing or malformed.
    pub fn from_context(ctx: &AppContext) -> Result<Self> {
        let raw = ctx
            .config
            .settings
            .clone()
            .ok_or_else(|| Error::string("missing `settings:` block in config"))?;
        let mut settings: Self = serde_json::from_value(raw)
            .map_err(|err| Error::string(&format!("invalid `settings:` block: {err}")))?;
        if let Some(row) = ctx
            .shared_store
            .get::<crate::models::site_settings::Model>()
        {
            settings.site.title = row.title.clone();
            settings.site.brand = row.brand.clone();
            settings.site.tagline = row.tagline.clone();
            settings.site.description = row.description.clone();
            settings.site.author = row.author.clone();
            settings.site.base_url = row.base_url.clone();
            settings.site.hero_image = row.hero_image.clone().unwrap_or_default();
            settings.site.posts_per_page = row.posts_per_page.clamp(1, 100);
        }
        Ok(settings)
    }

    /// # Errors
    /// Returns an error when the settings block is malformed.
    pub fn from_config(config: &loco_rs::config::Config) -> Result<Self> {
        let raw = config
            .settings
            .clone()
            .ok_or_else(|| Error::string("missing `settings:` block in config"))?;
        serde_json::from_value(raw)
            .map_err(|err| Error::string(&format!("invalid `settings:` block: {err}")))
    }

    #[must_use]
    pub fn admin_email(&self) -> Option<&str> {
        let email = self.site.admin_email.trim();
        (!email.is_empty()).then_some(email)
    }
}
