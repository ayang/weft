#![allow(elided_lifetimes_in_paths)]
#![allow(clippy::wildcard_imports)]
pub use sea_orm_migration::prelude::*;

mod m20261103_000000_init;
mod m20261111_000000_agent_api_keys;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20261103_000000_init::Migration),
            Box::new(m20261111_000000_agent_api_keys::Migration),
        ]
    }
}
