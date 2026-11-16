use std::io::{Cursor, Write};

use axum::{body::Body, extract::State, http::header};
use chrono::Utc;
use loco_rs::prelude::*;
use serde_json::json;
use zip::write::SimpleFileOptions;

use crate::{data::settings::Settings, security::AdminSession};

const SKILL_MD: &str = include_str!("../../.claude/skills/weft-content/SKILL.md");
const SKILL_README: &str = include_str!("../../.claude/skills/weft-content/README.md");
const AGENT_API_MD: &str = include_str!("../../docs/agent-api.md");
const AGENT_OPENAPI: &str = include_str!("../../docs/agent-openapi.yaml");

/// Download a self-contained skill bundle from the admin UI.
///
/// The files are embedded in the binary at compile time, so a `.deb` install
/// does not need to ship `.claude/` or `docs/` alongside the service.
#[debug_handler]
async fn download(State(ctx): State<AppContext>, _admin: AdminSession) -> Result<Response> {
    let settings = Settings::from_context(&ctx)?;
    let bytes = build_bundle(&settings)?;
    Ok(format::render()
        .header(header::CONTENT_TYPE, "application/zip")
        .header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"weft-content-skill.zip\"",
        )
        .header(header::CACHE_CONTROL, "no-store")
        .header("x-content-type-options", "nosniff")
        .response()
        .body(Body::from(bytes))?)
}

fn build_bundle(settings: &Settings) -> Result<Vec<u8>> {
    let base = settings.site.base_url.trim_end_matches('/');
    let api_base = format!("{base}/api/agent");
    let generated_at = Utc::now().to_rfc3339();
    let env_example = format!(
        "# Weft Agent API environment\n\
         # Generated from {base} at {generated_at}\n\
         # Create a key at {base}/admin/settings/api-keys, then fill it in.\n\
         WEFT_BASE_URL={api_base}\n\
         WEFT_API_KEY=\n"
    );
    // The repository doc links to the source-tree skill path; inside the
    // bundle the same skill sits one level up.
    let agent_api = format!(
        "> This is the bundled copy of the Weft Agent API reference. Links to the source-tree README, deployment, and feature docs are not part of this archive.\n\n{}",
        AGENT_API_MD.replace("../.claude/skills/weft-content/SKILL.md", "../SKILL.md")
    );
    let metadata = serde_json::to_string_pretty(&json!({
        "name": "weft-content",
        "version": env!("CARGO_PKG_VERSION"),
        "generated_at": generated_at,
        "site_base_url": base,
        "api_base_url": api_base,
        "admin_url": format!("{base}/admin/settings/api-keys"),
        "files": [
            "SKILL.md",
            "README.md",
            "references/agent-api.md",
            "references/agent-openapi.yaml",
            "env.example",
            "bundle.json"
        ]
    }))?;

    let files: [(&str, &str, u32); 6] = [
        ("weft-content/SKILL.md", SKILL_MD, 0o644),
        ("weft-content/README.md", SKILL_README, 0o644),
        ("weft-content/references/agent-api.md", &agent_api, 0o644),
        (
            "weft-content/references/agent-openapi.yaml",
            AGENT_OPENAPI,
            0o644,
        ),
        ("weft-content/env.example", &env_example, 0o644),
        ("weft-content/bundle.json", &metadata, 0o644),
    ];

    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, content, mode) in files {
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(mode);
        zip.start_file(name, options).map_err(|error| {
            Error::string(&format!("could not add {name} to skill bundle: {error}"))
        })?;
        zip.write_all(content.as_bytes()).map_err(|error| {
            Error::string(&format!("could not write {name} to skill bundle: {error}"))
        })?;
    }
    let cursor = zip
        .finish()
        .map_err(|error| Error::string(&format!("could not finish skill bundle: {error}")))?;
    Ok(cursor.into_inner())
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/admin/settings")
        .add("/api-keys/skill.zip", get(download))
}
