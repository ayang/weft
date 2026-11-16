use loco_rs::prelude::*;

use crate::models::agent_api_keys::{self, Scopes};

pub struct AgentKeyCreate;

#[async_trait]
impl Task for AgentKeyCreate {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "agent:key-create".to_string(),
            detail: "Create an Agent API key and print it once.\nUsage: cargo loco task agent:key-create name:codex scopes:read,write".to_string(),
        }
    }

    async fn run(&self, ctx: &AppContext, vars: &task::Vars) -> Result<()> {
        let name = vars.cli_arg("name").unwrap_or("agent");
        let scope_value = vars.cli_arg("scopes").unwrap_or("read,write");
        let scopes = Scopes::parse(scope_value)
            .map_err(|error| Error::string(&format!("invalid scopes: {error}")))?;
        let (key, plaintext) = agent_api_keys::Model::create_key(&ctx.db, name, scopes)
            .await
            .map_err(|error| Error::string(&format!("could not create key: {error}")))?;
        println!(
            "agent key created: {} (prefix weft_{}_)",
            key.name, key.prefix
        );
        println!("scopes: {}", key.scopes);
        println!();
        println!("{plaintext}");
        println!();
        println!("Store it outside the repository, e.g. WEFT_API_KEY in the agent's environment.");
        Ok(())
    }
}
