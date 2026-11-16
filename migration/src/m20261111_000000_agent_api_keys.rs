use loco_rs::schema::*;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        create_table(
            m,
            "agent_api_keys",
            &[
                ("id", ColType::PkAuto),
                ("name", ColType::String),
                // First segment of the plaintext key. Unique so authentication
                // can look up one candidate row without scanning every hash.
                ("prefix", ColType::StringUniq),
                // SHA-256 of the full plaintext key. The plaintext is shown once
                // when the key is created and is never stored.
                ("key_hash", ColType::StringUniq),
                // Comma-separated scopes: `read`, `write`.
                ("scopes", ColType::String),
                ("last_used_at", ColType::TimestampWithTimeZoneNull),
                ("revoked_at", ColType::TimestampWithTimeZoneNull),
            ],
            &[],
        )
        .await?;
        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        drop_table(m, "agent_api_keys").await?;
        Ok(())
    }
}
