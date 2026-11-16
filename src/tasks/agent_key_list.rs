use loco_rs::prelude::*;

use crate::models::agent_api_keys;

pub struct AgentKeyList;

#[async_trait]
impl Task for AgentKeyList {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "agent:key-list".to_string(),
            detail: "List Agent API keys (prefixes only; plaintext keys are never stored).\nUsage: cargo loco task agent:key-list".to_string(),
        }
    }

    async fn run(&self, ctx: &AppContext, _vars: &task::Vars) -> Result<()> {
        let keys = agent_api_keys::Model::all_for_admin(&ctx.db)
            .await
            .map_err(|error| Error::string(&format!("could not list keys: {error}")))?;
        if keys.is_empty() {
            println!("no Agent API keys; create one with `cargo loco task agent:key-create name:codex scopes:read,write`");
            return Ok(());
        }
        println!(
            "{:<4} {:<24} {:<12} {:<12} {:<20} STATUS",
            "ID", "NAME", "PREFIX", "SCOPES", "LAST USED"
        );
        for key in keys {
            println!(
                "{:<4} {:<24} weft_{:<7} {:<12} {:<20} {}",
                key.id,
                crate::util::truncate_chars(&key.name, 24),
                key.prefix,
                key.scopes,
                key.last_used_at
                    .map(|value| value.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "never".to_string()),
                if key.revoked_at.is_some() {
                    "revoked"
                } else {
                    "active"
                }
            );
        }
        Ok(())
    }
}
