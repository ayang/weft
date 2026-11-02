use loco_rs::prelude::*;

use crate::models::users;

pub struct AdminReset;

#[async_trait]
impl Task for AdminReset {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "admin:reset".to_string(),
            detail: "Reset an existing admin password without deleting the database.\nUsage: cargo loco task admin:reset email:you@example.com password:\"new-strong-password\"".to_string(),
        }
    }

    async fn run(&self, ctx: &AppContext, vars: &task::Vars) -> Result<()> {
        let email = vars
            .cli_arg("email")
            .map_err(|_| Error::string("email is mandatory"))?;
        let password = vars
            .cli_arg("password")
            .map_err(|_| Error::string("password is mandatory"))?;
        if password.chars().count() < 10 {
            return Err(Error::string("password must be at least 10 characters"));
        }
        let user = users::Model::find_by_email(&ctx.db, email).await?;
        let user = user
            .into_active_model()
            .reset_password(&ctx.db, password)
            .await
            .map_err(|err| Error::string(&format!("could not reset password: {err}")))?;
        let _ = user
            .into_active_model()
            .verified(&ctx.db)
            .await
            .map_err(|err| tracing::warn!(error = %err, "could not mark admin verified"));
        println!("admin password reset: {email}");
        Ok(())
    }
}
