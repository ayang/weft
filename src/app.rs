use async_trait::async_trait;
use loco_rs::{
    app::{AppContext, Hooks, Initializer},
    bgworker::{BackgroundWorker, Queue},
    boot::{create_app, BootResult, StartMode},
    config::Config,
    controller::AppRoutes,
    db::{self, truncate_table},
    environment::Environment,
    task::Tasks,
    Result,
};
use migration::Migrator;
use sea_orm::ConnectionTrait;
use std::path::Path;

#[allow(unused_imports)]
use crate::{
    controllers, initializers,
    models::{
        ai_sessions, ai_settings, categories, chart_data_caches, chart_recipes, chart_specs,
        embeddings, medias, post_tags, posts, site_settings, tags, users,
    },
    security::Limits,
    tasks,
    workers::{
        cleanup_cache::Worker as CleanupCacheWorker, index_post::Worker as IndexPostWorker,
        refresh_chart::Worker as RefreshChartWorker,
    },
};

pub struct App;

#[async_trait]
impl Hooks for App {
    fn app_name() -> &'static str {
        env!("CARGO_CRATE_NAME")
    }

    fn app_version() -> String {
        format!(
            "{} ({})",
            env!("CARGO_PKG_VERSION"),
            option_env!("BUILD_SHA")
                .or(option_env!("GITHUB_SHA"))
                .unwrap_or("dev")
        )
    }

    async fn boot(
        mode: StartMode,
        environment: &Environment,
        config: Config,
    ) -> Result<BootResult> {
        create_app::<Self, Migrator>(mode, environment, config).await
    }

    async fn initializers(_ctx: &AppContext) -> Result<Vec<Box<dyn Initializer>>> {
        Ok(vec![Box::new(
            initializers::view_engine::ViewEngineInitializer,
        )])
    }

    async fn after_context(ctx: AppContext) -> Result<AppContext> {
        // WAL is persistent in the database file; the other PRAGMAs are cheap
        // per-process settings. Keep the pool small: this app targets 1-2 GB
        // VPS instances, and reads are short.
        for pragma in [
            "PRAGMA journal_mode=WAL",
            "PRAGMA synchronous=NORMAL",
            "PRAGMA busy_timeout=5000",
            "PRAGMA foreign_keys=ON",
        ] {
            if let Err(err) = ctx.db.execute_unprepared(pragma).await {
                tracing::warn!(pragma, error = %err, "could not apply sqlite pragma");
            }
        }
        match site_settings::Model::singleton(&ctx.db).await {
            Ok(Some(row)) => ctx.shared_store.insert(row),
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(error = %err, "could not load site settings; using config defaults")
            }
        }
        let settings = crate::data::settings::Settings::from_context(&ctx)?;
        ctx.shared_store.insert(Limits::new(&settings));
        if let Err(err) = chart_recipes::Model::seed_defaults(&ctx.db).await {
            tracing::warn!(error = %err, "could not seed default chart recipes");
        }
        Ok(ctx)
    }

    fn routes(_ctx: &AppContext) -> AppRoutes {
        AppRoutes::with_default_routes()
            .add_route(controllers::public_pages::routes())
            .add_route(controllers::llms::routes())
            .add_route(controllers::chart_api::routes())
            .add_route(controllers::agent_api::routes())
            .add_route(controllers::admin::routes())
    }

    async fn connect_workers(ctx: &AppContext, queue: &Queue) -> Result<()> {
        queue.register(RefreshChartWorker::build(ctx)).await?;
        queue.register(IndexPostWorker::build(ctx)).await?;
        queue.register(CleanupCacheWorker::build(ctx)).await?;
        Ok(())
    }

    #[allow(unused_variables)]
    fn register_tasks(tasks: &mut Tasks) {
        tasks.register(tasks::user_create::UserCreate);
        tasks.register(tasks::user_delete::UserDelete);
        tasks.register(tasks::admin_create::AdminCreate);
        tasks.register(tasks::admin_reset::AdminReset);
        tasks.register(tasks::db_backup::DbBackup);
        tasks.register(tasks::demo_seed::DemoSeed);
        tasks.register(tasks::charts_refresh_due::ChartsRefreshDue);
        tasks.register(tasks::posts_enrich_due::PostsEnrichDue);
        tasks.register(tasks::cleanup_cache::CleanupCache);
    }

    async fn truncate(ctx: &AppContext) -> Result<()> {
        // Children before parents; foreign keys may be enabled.
        truncate_table(&ctx.db, chart_data_caches::Entity).await?;
        truncate_table(&ctx.db, embeddings::Entity).await?;
        truncate_table(&ctx.db, ai_sessions::Entity).await?;
        truncate_table(&ctx.db, ai_settings::Entity).await?;
        truncate_table(&ctx.db, site_settings::Entity).await?;
        truncate_table(&ctx.db, medias::Entity).await?;
        truncate_table(&ctx.db, chart_specs::Entity).await?;
        truncate_table(&ctx.db, post_tags::Entity).await?;
        truncate_table(&ctx.db, posts::Entity).await?;
        truncate_table(&ctx.db, tags::Entity).await?;
        truncate_table(&ctx.db, categories::Entity).await?;
        truncate_table(&ctx.db, chart_recipes::Entity).await?;
        truncate_table(&ctx.db, users::Entity).await?;
        Ok(())
    }

    async fn seed(ctx: &AppContext, base: &Path) -> Result<()> {
        db::seed::<users::ActiveModel>(&ctx.db, &base.join("users.yaml").display().to_string())
            .await?;
        Ok(())
    }
}
