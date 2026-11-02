use async_trait::async_trait;
use axum::{Extension, Router as AxumRouter};
use fluent_templates::{ArcLoader, FluentLoader};
use loco_rs::{
    app::{AppContext, Initializer},
    controller::views::{engines, ViewEngine},
    Error, Result,
};
use tracing::info;

const I18N_DIR: &str = "assets/i18n";
// NOTE: must live OUTSIDE `I18N_DIR` so the locale scan doesn't also pick it up;
// otherwise the shared resource is registered twice and bundle building fails
// ("Failed to add FTL resources to the bundle"). See loco-rs/loco#1749.
const I18N_SHARED: &str = "assets/shared.ftl";
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
        let views_dir = std::path::PathBuf::from("themes")
            .join(theme)
            .join("templates");
        let tera_engine = if std::path::Path::new(I18N_DIR).exists() {
            let arc = std::sync::Arc::new(
                ArcLoader::builder(&I18N_DIR, unic_langid::langid!("en-US"))
                    .shared_resources(Some(&[I18N_SHARED.into()]))
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
