use loco_rs::prelude::*;

use crate::models::agent_api_keys;

pub struct AgentKeyRevoke;

#[async_trait]
impl Task for AgentKeyRevoke {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "agent:key-revoke".to_string(),
            detail: "Revoke an Agent API key by id.\nUsage: cargo loco task agent:key-revoke id:1"
                .to_string(),
        }
    }

    async fn run(&self, ctx: &AppContext, vars: &task::Vars) -> Result<()> {
        let id = vars
            .cli_arg("id")
            .map_err(|_| Error::string("id is mandatory"))?
            .parse::<i64>()
            .map_err(|_| Error::string("id must be an integer"))?;
        let key = agent_api_keys::Model::find_by_id(&ctx.db, id)
            .await
            .map_err(|error| Error::string(&format!("could not find key: {error}")))?;
        key.revoke(&ctx.db)
            .await
            .map_err(|error| Error::string(&format!("could not revoke key: {error}")))?;
        println!("agent key revoked: {} (weft_{}_)", key.name, key.prefix);
        Ok(())
    }
}
