use loco_rs::prelude::*;

use crate::models::users;

pub struct AdminCreate;

#[async_trait]
impl Task for AdminCreate {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "admin:create".to_string(),
            detail: "Create the single admin account.\nUsage: cargo loco task admin:create email:you@example.com name:\"Your Name\" password:\"strong-password\"".to_string(),
        }
    }

    async fn run(&self, ctx: &AppContext, vars: &task::Vars) -> Result<()> {
        let email = vars
            .cli_arg("email")
            .map_err(|_| Error::string("email is mandatory"))?;
        let name = vars
            .cli_arg("name")
            .map_err(|_| Error::string("name is mandatory"))?;
        let password = vars
            .cli_arg("password")
            .map_err(|_| Error::string("password is mandatory"))?;
        if password.chars().count() < 10 {
            return Err(Error::string("password must be at least 10 characters"));
        }
        if users::Model::find_by_email(&ctx.db, email).await.is_ok() {
            return Err(Error::string(
                "a user with that email already exists; use `admin:reset email:... password:...` to reset it without recreating the database",
            ));
        }
        let params = users::RegisterParams {
            email: email.to_string(),
            password: password.to_string(),
            name: name.to_string(),
        };
        let user = users::Model::create_with_password(&ctx.db, &params).await?;
        let _ = user
            .into_active_model()
            .verified(&ctx.db)
            .await
            .map_err(|err| tracing::warn!(error = %err, "could not mark admin verified"));
        println!("admin created: {email}");
        Ok(())
    }
}
