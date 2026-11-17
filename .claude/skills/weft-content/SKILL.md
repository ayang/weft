---
name: weft-content
description: Use when the user asks you to publish, edit, illustrate, chart, or unpublish content on their Weft site (a self-hosted AI-native personal site), or when they mention WEFT_API_KEY / a Weft Agent API. Covers the HTTP workflow for reading drafts, writing Markdown, uploading images, building ECharts charts, publishing, and optimistic-lock edits, without opening the admin UI in a browser.
---

# Publishing to Weft from an external Agent

Weft exposes a JSON + multipart API at `$WEFT_BASE_URL/api/agent`. The human
runs the site; you get a scoped, revocable key. Do not log the key, put it in a
URL, commit it, or paste it into the article.

## Environment

Ask the user for these if they are missing — do not guess, and do not try to
log into `/admin` with a password:

```bash
WEFT_BASE_URL   # e.g. https://example.com/api/agent  (include /api/agent)
WEFT_API_KEY    # e.g. weft_1a2b3c4d_...
```

If the key is missing, tell the user to create one at
`/admin/settings/api-keys` (choose **read + write**) or run
`cargo loco task agent:key-create name:codex scopes:read,write`, then set
`WEFT_API_KEY`. Never invent a key.

In DeepSeek Harness, the bash tool does not inherit variables you export in
another terminal. Either put both lines in `~/.dsh/.env` and restart DSH, or
keep a git-ignored `.weft-agent.env` in the workspace and start each command
group with `set -a; . ./.weft-agent.env; set +a`.

## Rules of engagement

1. **Read before you write.** `GET /posts/{slug}`; keep `post.updated_at`.
2. **Draft by default.** Create with `"status": "draft"` unless the user
   explicitly asked to publish. Give the user `post.admin_edit_url` to review.
3. **Always send `expected_updated_at`** on edits. If you get `409`, re-fetch,
   merge your intended change into the new body, and retry once. Never blind
   overwrite.
4. **Prefer partial PATCH over re-sending the whole document.** Use
   `body_mode: "append"` to add a section or a chart marker.
5. **Markdown is the source of truth.** Use `body_md`; the `markdown` field is
   a convenience with front matter. A chart goes on its own line as
   `[[chart:ID]]`; a video as `[[video:filename]]`.
6. **Respect the chart system.** Charts are native ECharts specs, not PNGs.
   Send rows + `kind` + `x_field` / `y_fields`; the server validates and builds
   the option. Do not hand-write ECharts JSON unless the user needs a special
   chart type.
7. **Be conservative with destructive calls.** Do not delete a post or chart
   unless the user explicitly asked. Prefer unpublishing (`status: "draft"`).

## Verify the key

```bash
curl -sS "$WEFT_BASE_URL/me" -H "Authorization: Bearer $WEFT_API_KEY"
```

Expect `authenticated: true` and `scopes.write: true`. If `write` is false,
stop and ask the user for a key with the write scope; do not try to bypass it.

## Read

```bash
# publish feed (anonymous is fine)
curl -sS "$WEFT_BASE_URL/posts?limit=20" \
  -H "Authorization: Bearer $WEFT_API_KEY"

# drafts and the full body (key required for non-published)
curl -sS "$WEFT_BASE_URL/posts?status=draft&limit=20" \
  -H "Authorization: Bearer $WEFT_API_KEY"

# one post: post.updated_at, body_md, existing charts
curl -sS "$WEFT_BASE_URL/posts/my-slug" \
  -H "Authorization: Bearer $WEFT_API_KEY"
```

Response shape:

```json
{
  "post": {
    "id": 12, "slug": "my-slug", "title": "…", "status": "draft",
    "updated_at": "2026-11-11T10:00:00+08:00",
    "admin_edit_url": "https://example.com/admin/posts/12/edit",
    "tags": [{"name": "Rust", "slug": "rust"}]
  },
  "body_md": "# …",
  "markdown": "---\nslug: …",
  "charts": [{"chart_id": 7, "marker": "[[chart:7]]", "data_url": "…"}]
}
```

## Create a post

```bash
curl -sS -X POST "$WEFT_BASE_URL/posts" \
  -H "Authorization: Bearer $WEFT_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "title": "标题",
    "slug": "optional-ascii-slug",
    "summary": "一句话摘要",
    "body_md": "# 开头\n\n正文 Markdown。",
    "status": "draft",
    "pinned": false,
    "cover_image": "",
    "tags": ["标签一", "标签二"]
  }'
```

- `title` required. Omit `slug` to derive it from the title.
- `status`: `draft` (default), `review`, `published`.
- Duplicate slug → `409`.
- Response is the same detail shape as GET, status `201`.

## Edit an existing post

```bash
# append a section (keeps the rest of the body byte-for-byte)
curl -sS -X PATCH "$WEFT_BASE_URL/posts/my-slug" \
  -H "Authorization: Bearer $WEFT_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "body_md": "\n## 新增一节\n\n内容。\n",
    "body_mode": "append",
    "expected_updated_at": "2026-11-11T10:00:00+08:00"
  }'
```

PATCH accepts any subset of `title`, `slug`, `summary`, `body_md`, `status`,
`pinned`, `cover_image`, `tags`, `body_mode`, `expected_updated_at`.
`body_mode` is `replace` (default), `append`, or `prepend`.
Empty string clears `summary` / `cover_image`; `tags: []` clears tags.
Setting `status: "published"` publishes and rebuilds the index.

## Upload an image, video or attachment

```bash
curl -sS -X POST "$WEFT_BASE_URL/media" \
  -H "Authorization: Bearer $WEFT_API_KEY" \
  -F "file=@cover.png"
```

Response includes `markdown` (`![name](/media/....png)`) and `url`. Put the
markdown in `body_md` (e.g. after the intro), or use `url` for `cover_image`.
Limits: images ≤ 8 MB (JPG/PNG/WebP/GIF/AVIF), video ≤ 12 MB (MP4/WebM),
attachments ≤ 15 MB (PDF/Office/ZIP/text…; HTML/SVG/scripts are refused, ZIP
them instead). To reuse a previous upload, `GET /media?q=cover&kind=image` or
`kind=file` for attachments.

For a downloadable attachment, put `attachment_marker` on its own line in
`body_md`; the public page renders a download card. The server returns the
original name in the download's `Content-Disposition`.

If the source is a web image, download it with your own tools first, then
upload the file. The Weft server does not fetch arbitrary image URLs.

## Create a chart

Charts are attached to a post. The simplest request sends data rows:

```bash
curl -sS -X POST "$WEFT_BASE_URL/posts/my-slug/charts" \
  -H "Authorization: Bearer $WEFT_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "title": "每月发布量",
    "intent": "展示上半年的发布节奏",
    "kind": "line",
    "data": [
      {"月份": "1月", "发布数": 3},
      {"月份": "2月", "发布数": 5},
      {"月份": "3月", "发布数": 4}
    ],
    "x_field": "月份",
    "y_fields": ["发布数"],
    "options": {"smooth": true, "labels": true, "y_unit": "篇"},
    "refresh_cron": "0 0 4 * * * *",
    "status": "active"
  }'
```

- `kind`: `bar`, `line`, `area`, `pie`, `scatter`.
- `data`: up to 1000 rows / 40 columns; `columns` can be omitted.
- `options`: `smooth`, `stack`, `horizontal`, `legend`, `labels`, `palette`,
  `sort` (`none`/`asc`/`desc`), `height`, `x_label`, `y_label`, `y_unit`,
  `number_format` (`auto`/`integer`/`one_decimal`/`percent`/`compact`),
  `y_min`, `y_max`, `log_y`, `data_zoom`, `legend_position` (`top`/`bottom`/`right`),
  `dual_axis`.
- `data_source_type` defaults to `static`. `sql` needs `sql_query` +
  `sql_limit` (single SELECT, allowlisted tables); `http` needs `http_url` /
  `http_data_path` (allowlisted host).
- `refresh_cron` is a 7-field cron (`sec min hour day month weekday year`);
  omit for no refresh.
- For special ECharts types, send `render_config` **and**
  `data_source_config` JSON objects instead of `data`; do not mix modes.

Response (`201`) contains `chart_id`, `marker` (`[[chart:7]]`), `spec`, `data`,
`stale`, and `data_url`. Insert the marker into the article:

```bash
curl -sS -X PATCH "$WEFT_BASE_URL/posts/my-slug" \
  -H "Authorization: Bearer $WEFT_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"body_md":"\n\n[[chart:7]]\n","body_mode":"append","expected_updated_at":"…"}'
```

Update a chart with `PATCH /charts/{id}` (partial; `options` is merged), refresh
with `POST /charts/{id}/refresh`, delete with `DELETE /charts/{id}`.
Static charts have data immediately; SQL/HTTP charts may return `data: null`,
`stale: true` until the background worker finishes — GET again later.

## Publish / unpublish / delete

```bash
# publish
curl -sS -X PATCH "$WEFT_BASE_URL/posts/my-slug" \
  -H "Authorization: Bearer $WEFT_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"status":"published"}'

# back to draft
curl -sS -X PATCH "$WEFT_BASE_URL/posts/my-slug" \
  -H "Authorization: Bearer $WEFT_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"status":"draft"}'

# delete (only when explicitly requested)
curl -sS -X DELETE "$WEFT_BASE_URL/posts/my-slug" \
  -H "Authorization: Bearer $WEFT_API_KEY"
```

## Errors

JSON envelope: `{"error": "...", "message": "..."}`.

| Status | What to do |
|---|---|
| 400 | Fix the payload; read `message` |
| 401 | Key missing/invalid/revoked → ask the user for a new key |
| 403 | Key is read-only → ask for `write` |
| 404 | Wrong slug/id, or anonymous draft access |
| 409 | Slug conflict, or stale `expected_updated_at` → re-fetch and merge |
| 429 | Back off; `Retry-After` is 60 seconds |
| 502 | Chart data source failed; fix the query/URL or retry later |

## Full reference

- Human docs: `references/agent-api.md` (in this bundle).
- OpenAPI: `references/agent-openapi.yaml` (in this bundle).
- Install/quickstart: `README.md` (in this bundle).
- Admin key management: `/admin/settings/api-keys`.
