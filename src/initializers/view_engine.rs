use async_trait::async_trait;
use axum::{Extension, Router as AxumRouter};
use fluent_templates::{ArcLoader, FluentLoader};
use loco_rs::{
    app::{AppContext, Initializer},
    controller::views::{engines, ViewEngine},
    Error, Result,
};
use tracing::info;

// NOTE: `I18N_SHARED` must live OUTSIDE `I18N_DIR` so the locale scan doesn't
// also pick it up; otherwise the shared resource is registered twice and bundle
// building fails ("Failed to add FTL resources to the bundle").
// See loco-rs/loco#1749.
//
// Runtime resources are resolved against `WEFT_APP_DIR` rather than the current
// working directory. A `.deb` installs them under `/usr/share/weft` and points
// the systemd units here, so CLI tools and the service agree no matter where
// they are started from.
fn app_dir() -> std::path::PathBuf {
    match std::env::var("WEFT_APP_DIR") {
        Ok(dir) if !dir.trim().is_empty() => std::path::PathBuf::from(dir),
        _ => std::path::PathBuf::from("."),
    }
}

#[allow(clippy::module_name_repetitions)]
pub struct ViewEngineInitializer;

#[async_trait]
impl Initializer for ViewEngineInitializer {
    fn name(&self) -> String {
        "view-engine".to_string()
    }

    async fn after_routes(&self, router: AxumRouter, _ctx: &AppContext) -> Result<AxumRouter> {
        let theme = std::env::var("SITE_THEME").unwrap_or_else(|_| "default".to_string());
        let theme = if theme.trim().is_empty() {
            "default"
        } else {
            theme.trim()
        };
        let base = app_dir();
        let views_dir = base.join("themes").join(theme).join("templates");
        let i18n_dir = base.join("assets/i18n");
        let i18n_shared = base.join("assets/shared.ftl");
        let tera_engine = if i18n_dir.exists() {
            let shared = [i18n_shared];
            let arc = std::sync::Arc::new(
                ArcLoader::builder(&i18n_dir, unic_langid::langid!("en-US"))
                    .shared_resources(Some(&shared))
                    .customize(|bundle| bundle.set_use_isolating(false))
                    .build()
                    .map_err(|e| Error::string(&e.to_string()))?,
            );
            info!("locales loaded");

            engines::TeraView::from_custom_dir(&views_dir, move |tera| {
                tera.register_function("t", FluentLoader::new(arc.clone()));
                Ok(())
            })?
        } else {
            engines::TeraView::from_custom_dir(&views_dir, |_| Ok(()))?
        };

        Ok(router.layer(Extension(ViewEngine::from(tera_engine))))
    }
}
