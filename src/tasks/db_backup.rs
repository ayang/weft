use std::path::Path;

use chrono::Local;
use loco_rs::prelude::*;
use sea_orm::ConnectionTrait;

pub struct DbBackup;

#[async_trait]
impl Task for DbBackup {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "db:backup".to_string(),
            detail: "Create a consistent SQLite backup with VACUUM INTO.\nUsage: cargo loco task db:backup [path:backups/weft-YYYYmmdd-HHMMSS.sqlite]".to_string(),
        }
    }

    async fn run(&self, ctx: &AppContext, vars: &task::Vars) -> Result<()> {
        let path = vars.cli_arg("path").map_or_else(
            |_| {
                format!(
                    "backups/weft-{}.sqlite",
                    Local::now().format("%Y%m%d-%H%M%S")
                )
            },
            ToString::to_string,
        );
        if let Some(parent) = Path::new(&path).parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| Error::string(&format!("could not create backup dir: {err}")))?;
        }
        // VACUUM INTO is SQLite's consistent, single-file backup mechanism and
        // works while the live database is in WAL mode.
        let escaped = path.replace('\'', "''");
        ctx.db
            .execute_unprepared(&format!("VACUUM INTO '{escaped}'"))
            .await
            .map_err(|err| Error::string(&format!("database backup failed: {err}")))?;
        println!("database backup created: {path}");
        Ok(())
    }
}
