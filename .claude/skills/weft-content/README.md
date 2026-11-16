# Weft Content Agent Skill

This is a self-contained skill bundle for publishing and editing content on a
Weft site through its Agent API. It was exported from the site's admin page
**Agent API** (`/admin/settings/api-keys`).

## Contents

| File | Purpose |
|---|---|
| `SKILL.md` | The Agent Skill itself: rules, endpoints, workflows |
| `references/agent-api.md` | Full endpoint and field reference |
| `references/agent-openapi.yaml` | OpenAPI 3.1 contract |
| `env.example` | Site-specific `WEFT_BASE_URL`; add your key |
| `bundle.json` | Bundle name, version, generation time, site URL |

## Install

### Claude Code / Agent Skills tools

```bash
mkdir -p ~/.claude/skills
cp -a weft-content ~/.claude/skills/
```

### DeepSeek Harness

DSH scans `.dsh/skills`, `.agents/skills`, `~/.dsh/skills`, and
`~/.agents/skills`, not `.claude/skills`:

```bash
mkdir -p ~/.agents/skills
cp -a weft-content ~/.agents/skills/
```

Then in DSH Web, type `/weft-content <task>`, or describe the task and let the
model load the skill from the catalog.

### Codex / `AGENTS.md` tools

Copy the whole `weft-content` directory into the workspace (for example
`.agents/skills/weft-content/`) and add one line to `AGENTS.md`:

```text
For Weft publishing, read .agents/skills/weft-content/SKILL.md and follow it.
```

Keeping the directory together preserves the relative `references/` links.

## Configure

The bundle already knows the site URL in `env.example`:

```bash
set -a
. ./env.example
set +a
```

Set `WEFT_API_KEY` first. Create a key at the site's
`/admin/settings/api-keys` page (use **read + write** for publishing), or run:

```bash
cargo loco task agent:key-create name:codex scopes:read,write
```

For DeepSeek Harness, put both variables in `~/.dsh/.env` and restart DSH, or
source `env.example` at the start of each command group.

## Quick test

```bash
curl -sS "$WEFT_BASE_URL/me" \
  -H "Authorization: Bearer $WEFT_API_KEY"
```

Expect `"authenticated": true` and `"scopes": {"write": true, ...}`.

## Safety

- The key is a secret. Do not commit it, put it in a URL, or paste it into the
  article.
- Use a `read + write` key only for agents you trust to edit/publish.
- The skill defaults to drafts; review `admin_edit_url` before publishing.
- Always send `expected_updated_at` on edits. A `409` means re-fetch and merge.
- Do not delete posts or charts unless the user explicitly asked.
