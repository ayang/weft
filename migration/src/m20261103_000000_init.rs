use loco_rs::schema::*;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        create_table(
            m,
            "users",
            &[
                ("id", ColType::PkAuto),
                ("pid", ColType::Uuid),
                ("email", ColType::StringUniq),
                ("password", ColType::String),
                ("api_key", ColType::StringUniq),
                ("name", ColType::String),
                ("reset_token", ColType::StringNull),
                ("reset_sent_at", ColType::TimestampWithTimeZoneNull),
                ("email_verification_token", ColType::StringNull),
                (
                    "email_verification_sent_at",
                    ColType::TimestampWithTimeZoneNull,
                ),
                ("email_verified_at", ColType::TimestampWithTimeZoneNull),
                ("magic_link_token", ColType::StringNull),
                ("magic_link_expiration", ColType::TimestampWithTimeZoneNull),
            ],
            &[],
        )
        .await?;

        create_table(
            m,
            "posts",
            &[
                ("id", ColType::PkAuto),
                ("slug", ColType::StringUniq),
                ("title", ColType::String),
                ("summary", ColType::TextNull),
                ("body_md", ColType::Text),
                ("body_ref", ColType::StringNull),
                ("cover_image", ColType::StringNull),
                ("status", ColType::String),
                ("pinned", ColType::BooleanNull),
                ("reading_minutes", ColType::BigInteger),
                ("published_at", ColType::TimestampWithTimeZoneNull),
                ("category_id", ColType::BigIntegerNull),
            ],
            &[],
        )
        .await?;

        create_table(
            m,
            "categories",
            &[
                ("id", ColType::PkAuto),
                ("name", ColType::String),
                ("slug", ColType::StringUniq),
                ("description", ColType::TextNull),
            ],
            &[],
        )
        .await?;

        create_table(
            m,
            "tags",
            &[
                ("id", ColType::PkAuto),
                ("name", ColType::StringUniq),
                ("slug", ColType::StringUniq),
            ],
            &[],
        )
        .await?;

        create_table(
            m,
            "post_tags",
            &[("id", ColType::PkAuto)],
            &[("post", ""), ("tag", "")],
        )
        .await?;

        create_table(
            m,
            "ai_sessions",
            &[
                ("id", ColType::PkAuto),
                ("purpose", ColType::String),
                ("provider", ColType::String),
                ("model", ColType::String),
                ("messages", ColType::Json),
                ("tool_calls", ColType::JsonNull),
                ("status", ColType::String),
                ("input_tokens", ColType::BigIntegerNull),
                ("output_tokens", ColType::BigIntegerNull),
                ("cost_estimate", ColType::DoubleNull),
                ("error", ColType::TextNull),
                ("completed_at", ColType::TimestampWithTimeZoneNull),
            ],
            &[("post?", "")],
        )
        .await?;

        create_table(
            m,
            "chart_specs",
            &[
                ("id", ColType::PkAuto),
                ("title", ColType::String),
                ("intent", ColType::TextNull),
                ("kind", ColType::String),
                ("data_source_type", ColType::String),
                ("data_source_config", ColType::Json),
                ("render_config", ColType::Json),
                ("refresh_cron", ColType::StringNull),
                ("last_run_at", ColType::TimestampWithTimeZoneNull),
                ("last_success_at", ColType::TimestampWithTimeZoneNull),
                ("status", ColType::String),
                ("error_message", ColType::TextNull),
                ("failure_count", ColType::BigInteger),
                ("next_attempt_at", ColType::TimestampWithTimeZoneNull),
            ],
            &[("post", "")],
        )
        .await?;

        create_table(
            m,
            "chart_data_caches",
            &[
                ("id", ColType::PkAuto),
                ("payload", ColType::Json),
                ("etag", ColType::String),
                ("fetched_at", ColType::TimestampWithTimeZone),
                ("expires_at", ColType::TimestampWithTimeZoneNull),
            ],
            &[("chart_spec", "")],
        )
        .await?;

        create_table(
            m,
            "chart_recipes",
            &[
                ("id", ColType::PkAuto),
                ("name", ColType::StringUniq),
                ("description", ColType::TextNull),
                ("prompt_pattern", ColType::Text),
                ("data_schema", ColType::Json),
                ("render_template", ColType::Json),
                ("created_by", ColType::StringNull),
            ],
            &[],
        )
        .await?;

        create_table(
            m,
            "embeddings",
            &[
                ("id", ColType::PkAuto),
                ("chunk_index", ColType::BigInteger),
                ("content", ColType::Text),
                ("embedding", ColType::BlobNull),
                ("model", ColType::String),
            ],
            &[("post", "")],
        )
        .await?;

        create_table(
            m,
            "ai_settings",
            &[
                ("id", ColType::PkAuto),
                ("provider", ColType::String),
                ("base_url", ColType::String),
                ("model", ColType::String),
                ("api_key", ColType::TextNull),
                ("enabled", ColType::Boolean),
            ],
            &[],
        )
        .await?;

        create_table(
            m,
            "site_settings",
            &[
                ("id", ColType::PkAuto),
                ("title", ColType::String),
                ("brand", ColType::String),
                ("tagline", ColType::String),
                ("description", ColType::Text),
                ("author", ColType::String),
                ("base_url", ColType::String),
                ("posts_per_page", ColType::Integer),
                ("hero_image", ColType::StringNull),
            ],
            &[],
        )
        .await?;

        create_table(
            m,
            "media",
            &[
                ("id", ColType::PkAuto),
                ("filename", ColType::String),
                ("original_name", ColType::String),
                ("mime", ColType::String),
                ("kind", ColType::String),
                ("size", ColType::Integer),
            ],
            &[],
        )
        .await?;

        for (name, table, columns) in [
            (
                "ux_post_tags_post_tag",
                "post_tags",
                vec!["post_id", "tag_id"],
            ),
            (
                "ux_chart_data_caches_chart",
                "chart_data_caches",
                vec!["chart_spec_id"],
            ),
            (
                "ux_embeddings_post_chunk",
                "embeddings",
                vec!["post_id", "chunk_index"],
            ),
            (
                "ix_posts_status_published",
                "posts",
                vec!["status", "published_at"],
            ),
            (
                "ix_chart_specs_due",
                "chart_specs",
                vec!["status", "next_attempt_at"],
            ),
        ] {
            let mut index = Index::create()
                .name(name)
                .table(Alias::new(table))
                .to_owned();
            for column in columns {
                index = index.col(Alias::new(column)).to_owned();
            }
            if name.starts_with("ux_") {
                index = index.unique().to_owned();
            }
            m.create_index(index).await?;
        }

        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            "medias",
            "site_settings",
            "ai_settings",
            "embeddings",
            "chart_recipes",
            "chart_data_caches",
            "chart_specs",
            "ai_sessions",
            "post_tags",
            "tags",
            "categories",
            "posts",
            "users",
        ] {
            drop_table(m, table).await?;
        }
        Ok(())
    }
}
