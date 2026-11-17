# Agent API：让外部 Agent 写作、配图与建图表

Weft 的公开阅读端一直是 Agent 友好的：`/llms.txt`、`/posts/:slug.md`、`/api/agent/*` 都返回结构化内容。现在 `/api/agent/*` 也可以**写入**，用于 Codex、Claude Code、Cursor 或任何能发 HTTP 请求的 Agent。

> 设计目标：Agent 不必打开浏览器、不必模拟登录后台。它拿一个可撤销的 Key，就能读草稿、写文章、上传图片、建/改图表、发布或删除；人类仍然可以在 `/admin` 里审查。

## 1. 创建 Key

两种方式，结果相同：

**管理后台（推荐）**

打开 `/admin/settings/api-keys`，填写名称、勾选权限，点击“生成 Key”。Key 只显示一次，形如：

```text
weft_1a2b3c4d_<64 hex chars>
```

**命令行（无浏览器 / 首次部署）**

```bash
cargo loco task agent:key-create name:codex scopes:read,write
# 生产 .deb 安装：
sudo -u weft sh -c 'cd /usr/share/weft && set -a && . /etc/weft/weft.env && WEFT_APP_DIR=/usr/share/weft && set +a && weft-cli task agent:key-create name:codex scopes:read,write'
```

其他命令：

```bash
cargo loco task agent:key-list
cargo loco task agent:key-revoke id:1
```

Key 只以 SHA-256 哈希存进 `agent_api_keys` 表；丢失后无法找回，只能撤销重建。数据库备份会带上 Key 的哈希，但不会带明文。

### 把技能装到 Agent 里

- **直接下载（推荐）**：登录后台 `/admin/settings/api-keys`，点“下载 Skill 包”。生成的 `weft-content-skill.zip` 内含 `SKILL.md`、`README.md`、`references/agent-api.md`、`references/agent-openapi.yaml`、`env.example` 和 `bundle.json`，解压到 Agent 的 skill 目录即可，不需要本仓库代码。`env.example` 已填入当前站点的 API URL，只需补 `WEFT_API_KEY`。这些文件在编译时嵌入二进制，修改 `SKILL.md` / `docs/agent-api.md` 后需重新构建并重启服务，下载包才会更新。
- **Claude Code / 支持 Agent Skills 的工具**：把仓库里的 `.claude/skills/weft-content/` 复制或软链到用户目录：
  ```bash
  mkdir -p ~/.claude/skills
  ln -sfn "$PWD/.claude/skills/weft-content" ~/.claude/skills/weft-content
  ```
- **Codex / 读 `AGENTS.md` 的工具**：把整个 `.claude/skills/weft-content/` 目录复制到 workspace（如 `.agents/skills/weft-content/`），并在 `AGENTS.md` 里引用其中的 `SKILL.md`；这样 `references/` 相对链接才完整。
- **DeepSeek Harness**：DSH 不扫描 `.claude/skills`，它扫描项目根的 `.dsh/skills`、`.agents/skills` 以及 `~/.dsh/skills`、`~/.agents/skills`。本仓库已把 `.agents/skills/weft-content` 软链到 `.claude/skills/weft-content`，所以 DSH 会自动发现。若想在任意目录使用，可把它再链到用户级目录：`mkdir -p ~/.agents/skills && ln -sfn /path/to/weft/.claude/skills/weft-content ~/.agents/skills/weft-content`。在 DSH Web 里可以输入 `/weft-content` 显式调用，或直接描述任务让模型按 catalog 自动加载。
- 无论哪种方式，Agent 都需要拿到 `WEFT_BASE_URL` 和 `WEFT_API_KEY` 两个环境变量。DSH 可在 `~/.dsh/.env` 里设置（启动时加载，修改后需重启 DSH），或从启动 DSH 的进程环境继承。仓库里另有 `.weft-agent.env.example`，复制成 git-ignored 的 `.weft-agent.env` 后在命令前 `set -a; . ./.weft-agent.env; set +a` 即可。

## 2. 权限与认证

先用 `GET /api/agent/me` 验证 Key 和环境：

```json
{
  "authenticated": true,
  "actor": "codex",
  "scopes": {"read": true, "write": true, "private_read": true},
  "site": {"title": "Weft", "base_url": "https://example.com"},
  "agent_api": "https://example.com/api/agent"
}
```

匿名请求返回 `authenticated: false`，只能读取已发布内容。旧 `AGENT_API_TOKEN` 返回 `authenticated: true` 但 `private_read: false`。

| Scope | 能做什么 |
|---|---|
| `read` | 读取文章（含草稿）、图表、媒体库 |
| `write` | 自动包含 `read`；创建/修改/发布/删除文章，上传媒体，创建/修改/删除/刷新图表 |

请求头二选一：

```http
Authorization: Bearer weft_...
X-Agent-Token: weft_...
```

公开阅读通常不需要 Key：

- `GET /api/agent/posts` 只返回已发布文章；
- `GET /api/agent/posts/:slug` 只返回已发布文章；
- `GET /api/agent/charts/:id` 只返回已发布文章下、未暂停的图表。

一旦请求里带了无效/已撤销的 Key，直接 `401`，不会静默降级为匿名。旧的 `AGENT_API_TOKEN` 环境变量仍然有效，但**只保留只读权限**，避免历史配置悄悄获得写权限。

每个 Key 与旧 token 独立按分钟限流（默认 30 次/分钟，`AGENT_RATE_LIMIT_PER_MIN` 可调）；匿名公开读取不计入限流。超限返回 `429` 和 `Retry-After: 60`。无效 Key 的尝试另有一个全局每分钟上限，避免无成本地撞库。

## 3. 端点总览

所有路径以 `WEFT_BASE_URL` 为前缀（例如 `https://example.com/api/agent`）。

| 方法 | 路径 | Scope | 说明 |
|---|---|---|---|
| GET | `/me` | 公开 / read | 验证 Key：返回是否认证、actor、scopes、站点信息 |
| GET | `/posts?status=&since=&limit=` | 公开 / read | `status=published` 默认；`draft`、`review`、`all` 需要 Key |
| GET | `/posts/{slug}` | 公开 / read | 带 Key 可读草稿；返回 `body_md` 与 `markdown` |
| POST | `/posts` | write | 新建文章，默认 `draft` |
| PATCH | `/posts/{slug}` | write | 局部修改；支持 `body_mode` 与乐观锁 |
| DELETE | `/posts/{slug}` | write | 删除文章及其图表/标签关联 |
| GET | `/posts/{slug}/charts` | read | 列出该文章的图表 |
| POST | `/posts/{slug}/charts` | write | 新建图表 |
| GET | `/charts/{id}` | 公开 / read | 图表 spec + 最新数据 + ETag |
| PATCH | `/charts/{id}` | write | 局部修改图表 |
| DELETE | `/charts/{id}` | write | 删除图表 |
| POST | `/charts/{id}/refresh` | write | 立即拉取数据并返回结果 |
| GET | `/media?q=&kind=&limit=` | read | 搜索媒体库（`kind=image|video|file`） |
| POST | `/media` | write | multipart 上传图片、视频或附件 |

完整机器可读契约见 [`agent-openapi.yaml`](agent-openapi.yaml)。给 Agent 的技能说明见 [`.claude/skills/weft-content/SKILL.md`](../.claude/skills/weft-content/SKILL.md)。

## 4. 文章

### 4.1 新建

`POST /api/agent/posts`

```json
{
  "title": "把数据讲清楚：三种图表选择",
  "slug": "chart-choices",
  "summary": "从数据形状出发，而不是从图表类型出发。",
  "body_md": "# 开头\n\n正文 Markdown。",
  "status": "draft",
  "pinned": false,
  "cover_image": "",
  "tags": ["数据可视化", "写作"]
}
```

- `title` 必填；`slug` 留空时由标题生成，重复返回 `409`。
- `status`：`draft`（默认）、`review`、`published`。
- `tags` 是字符串数组；服务端自动建标签、按 slug 去重。
- 返回 `201`，body 是文章详情（见 4.3）。

### 4.2 修改

`PATCH /api/agent/posts/{slug}`，只传要改的字段：

```json
{
  "body_md": "\n## 新增章节\n\n…",
  "body_mode": "append",
  "tags": ["数据可视化", "图表"],
  "expected_updated_at": "2026-11-11T10:00:00+08:00"
}
```

- `title`、`slug`、`summary`、`body_md`、`status`、`pinned`、`cover_image`、`tags` 都可单独出现。
- `body_mode`：`replace`（默认）、`append`、`prepend`，只作用于本次传入的 `body_md`。
- `summary`、`cover_image` 传空字符串表示清空；`tags: []` 清空标签。
- 把 `status` 改为 `published` 会触发索引重建和 `llms.txt` 失效；改回 `draft` 会从公开端消失。
- `expected_updated_at` 是乐观锁。先 GET 拿到 `post.updated_at`，修改时原样带回；若文章已被别人改过，返回 `409` 和当前时间，不会覆盖。

### 4.3 读取

`GET /api/agent/posts/{slug}`：

```json
{
  "post": {
    "id": 12,
    "slug": "chart-choices",
    "title": "把数据讲清楚：三种图表选择",
    "summary": "…",
    "status": "draft",
    "pinned": false,
    "cover_image": "",
    "url": "https://example.com/posts/chart-choices",
    "markdown_url": "https://example.com/posts/chart-choices.md",
    "admin_edit_url": "https://example.com/admin/posts/12/edit",
    "published_at": "",
    "updated_at": "2026-11-11T10:00:00+08:00",
    "reading_minutes": 3,
    "tags": [{"name": "数据可视化", "slug": "数据可视化"}]
  },
  "body_md": "# 开头\n\n正文 Markdown。",
  "markdown": "---\nslug: chart-choices\n…",
  "charts": [
    {
      "chart_id": 7,
      "title": "发布趋势",
      "kind": "line",
      "status": "active",
      "marker": "[[chart:7]]",
      "data_url": "https://example.com/api/agent/charts/7"
    }
  ]
}
```

`body_md` 是纯正文；`markdown` 带 front matter，适合直接写文件。`charts[].marker` 就是插入正文的标记。未发布文章的 `url` / `markdown_url` 是发布后的地址，当前访问会 404；审阅时用 `admin_edit_url`。

列表 `GET /api/agent/posts?status=draft&limit=20` 返回摘要数组，字段同上但不含 `body_md`/`markdown`。`since` 对已发布列表按 `published_at` 过滤，对草稿列表按 `updated_at` 过滤。

## 5. 媒体

`POST /api/agent/media`，`multipart/form-data`，字段名 `file`：

```bash
curl -sS -X POST "$WEFT_BASE_URL/media" \
  -H "Authorization: Bearer $WEFT_API_KEY" \
  -F "file=@cover.png"
```

返回：

```json
{
  "id": 3,
  "url": "/media/9f8e....png",
  "filename": "9f8e....png",
  "original_name": "cover.png",
  "kind": "image",
  "mime": "image/png",
  "size": 204800,
  "size_label": "200 KB",
  "type_label": "PNG",
  "download_url": "/media/9f8e....png/download",
  "markdown": "![cover.png](/media/9f8e....png)",
  "video_marker": "[[video:9f8e....png]]",
  "attachment_marker": "[[file:9f8e....png|cover.png]]",
  "created_at": "2026-11-11T10:00:00+08:00"
}
```

- 图片 JPG/PNG/WebP/GIF/AVIF，≤ 8 MB；视频 MP4/WebM，≤ 12 MB；其他文件作为附件，≤ 15 MB。图片/视频按文件内容嗅探，扩展名写错也能识别；HTML/SVG/脚本类附件出于安全考虑拒收，请打包成 ZIP。
- 图片把 `markdown` 插入正文；视频把 `video_marker` 单独一行放入正文；附件把 `attachment_marker` 单独一行放入正文。附件下载使用 `download_url`（响应带原始文件名）。
- `GET /media?q=cover&kind=image` 复用已上传素材；`kind=file` 只看附件。

Agent 需要从网页取图时，先用它自己的下载能力拿到文件，再 `-F file=@...` 上传；服务端不代抓任意 URL，避免 SSRF。

## 6. 图表

图表不是图片，而是「受控 ECharts spec + 数据源 + 缓存数据」。Agent 有两种写法。

### 6.1 Blueprint 模式（推荐）

只给数据和字段，服务端校验并生成 ECharts option：

`POST /api/agent/posts/{slug}/charts`

```json
{
  "title": "每月发布量",
  "intent": "展示 2026 年上半年的发布节奏",
  "kind": "line",
  "data": [
    {"月份": "1月", "发布数": 3},
    {"月份": "2月", "发布数": 5},
    {"月份": "3月", "发布数": 4}
  ],
  "x_field": "月份",
  "y_fields": ["发布数"],
  "options": {
    "smooth": true,
    "labels": true,
    "y_unit": "篇",
    "number_format": "integer"
  },
  "refresh_cron": "0 0 4 * * * *",
  "status": "active"
}
```

- `kind`：`bar`、`line`、`area`、`pie`、`scatter`。
- `data`：对象数组（也接受数组的数组）；最多 1000 行、40 列。
- `columns` 可省略，服务端从行键推断；`x_field` / `y_fields` 省略时自动推断。
- `options` 可控制：`smooth`、`stack`、`horizontal`、`legend`、`labels`、`palette`（`default`/`warm`/`cool`/`mono`）、`sort`（`none`/`asc`/`desc`）、`height`、`x_label`、`y_label`、`y_unit`、`number_format`（`auto`/`integer`/`one_decimal`/`percent`/`compact`）、`y_min`、`y_max`、`log_y`、`data_zoom`、`legend_position`（`top`/`bottom`/`right`）、`dual_axis`。
- `data_source_type` 默认 `static`。也支持：
  - `sql`：传 `sql_query`、`sql_limit`；只允许单条 `SELECT`，表必须在 `settings.charts.sql_allowed_tables` 白名单。
  - `http`：传 `http_url`、`http_data_path`；域名必须在 `settings.charts.http_allowlist` 中。
- 非静态图表没有 `data` 时，需要提供 `columns`、`x_field`、`y_fields`，让服务端知道字段映射。
- `refresh_cron` 使用 7 段 cron（秒 分 时 日 月 周 年），留空表示不自动刷新。
- `status`：`active`（默认）或 `paused`。

### 6.2 高级模式（任意 ECharts option）

同时传 `render_config` 和 `data_source_config` 两个 JSON 对象：

```json
{
  "title": "自定义 K 线",
  "kind": "kline",
  "data_source_type": "static",
  "data_source_config": {"data": [{"date": "2026-01-01", "open": 1, "close": 2}]},
  "render_config": {"xAxis": {"type": "category"}, "series": [{"type": "candlestick"}]}
}
```

高级模式适合 `kline`、`sankey`、`graph` 等特殊图表；服务端只做 JSON 对象/大小校验，不执行其中的代码。两种模式不要混用。

### 6.3 修改、刷新、删除

`PATCH /api/agent/charts/{id}` 是局部更新：

- 只传 `title`、`intent`、`status`、`refresh_cron` 时，不重建 spec。
- 传 `data` / `kind` / `x_field` / `y_fields` / `columns` / `options` / 数据源字段时，会从现有 blueprint 重建 spec。`options` 是**合并**，例如只传 `{"smooth": true}` 不会清掉其他选项。
- 传 `render_config` / `data_source_config` 时走高级模式；不能和 blueprint 字段混用。

```json
{"title": "每月发布量（更新）", "options": {"labels": true}}
```

`POST /api/agent/charts/{id}/refresh` 立即拉数据。静态图表创建时已同步填充；SQL/HTTP 图表创建后由后台 Worker 拉取，响应里的 `data` 可能为 `null`、`stale: true`，稍后 GET 即可。

`DELETE /api/agent/charts/{id}` 返回 `204`。

### 6.4 把图表放进正文

图表响应里有：

```json
"marker": "[[chart:7]]"
```

在正文单独一行写这个标记，图表会渲染在该位置；没有标记的图表仍会汇总到文末“数据图表”区域。典型流程：

1. 新建图表拿 `chart_id` / `marker`；
2. `PATCH /posts/{slug}`，`body_mode: "append"`，`body_md` 加 `\n\n[[chart:7]]\n`；
3. 发布文章。

## 7. 错误格式

所有错误都是 JSON：

```json
{"error": "bad_request", "message": "title is required"}
```

| 状态码 | 含义 |
|---|---|
| 400 | 字段缺失/格式错误、图表数据不合法、SQL/HTTP 配置不合法 |
| 401 | 缺少/无效/已撤销 Key |
| 403 | Key 只有 read，调用了写接口 |
| 404 | 文章/图表不存在，或匿名读取未发布内容 |
| 409 | slug 冲突，或 `expected_updated_at` 过期 |
| 429 | 超过每分钟限流 |
| 502 | 图表 refresh 时数据源抓取失败 |

## 8. 端到端示例

```bash
export WEFT_BASE_URL="https://example.com/api/agent"
export WEFT_API_KEY="weft_..."

# 1. 建草稿
curl -sS -X POST "$WEFT_BASE_URL/posts" \
  -H "Authorization: Bearer $WEFT_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"title":"Agent 写的文章","body_md":"# 开头\n\n正文。","status":"draft","tags":["Agent"]}'

# 2. 上传封面
curl -sS -X POST "$WEFT_BASE_URL/media" \
  -H "Authorization: Bearer $WEFT_API_KEY" \
  -F "file=@cover.png"

# 3. 建图表
curl -sS -X POST "$WEFT_BASE_URL/posts/agent-写的文章/charts" \
  -H "Authorization: Bearer $WEFT_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"title":"发布趋势","kind":"line","data":[{"月份":"1月","数量":3},{"月份":"2月","数量":5}],"x_field":"月份","y_fields":["数量"]}'

# 4. 读回文章，拿 updated_at
curl -sS "$WEFT_BASE_URL/posts/agent-写的文章" \
  -H "Authorization: Bearer $WEFT_API_KEY"

# 5. 追加图表标记（把 EXPECTED 换成上一步的 updated_at）
curl -sS -X PATCH "$WEFT_BASE_URL/posts/agent-写的文章" \
  -H "Authorization: Bearer $WEFT_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"body_md":"\n\n[[chart:1]]\n","body_mode":"append","expected_updated_at":"EXPECTED"}'

# 6. 发布
curl -sS -X PATCH "$WEFT_BASE_URL/posts/agent-写的文章" \
  -H "Authorization: Bearer $WEFT_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"status":"published"}'
```

> 路径里的 slug 含中文时，客户端需要 URL 编码；上例为了可读性保留中文。实际使用建议让 Agent 显式给一个 ASCII `slug`。

## 9. 安全与运维

- Key 明文只在创建时返回一次；数据库存 SHA-256，后台只显示前缀。
- 撤销立即生效；每个 Key 可分别限流。
- 写操作（创建/修改/发布/删除文章、建/改/删图表、上传媒体）写 `tracing` 审计日志，带 Key 名称与资源 id，不打印 Key。
- 公开列表/图表仍走 ETag；带 Key 的草稿列表和图表详情返回 `private, no-store`。
- `MEDIA_DIR`、SQLite 数据库（含 Key 哈希）都要纳入备份。
- 旧的 `AGENT_API_TOKEN` 只读；如果生产环境仍依赖它，升级不会破坏读取。

## 10. 部署与迁移

升级后首次启动默认会自动执行 `agent_api_keys` 迁移（`auto_migrate: true`）。如果生产环境设置了 `DB_AUTO_MIGRATE=false`，先运行 `weft-cli db migrate` 再启动。没有新增环境变量；Key 存在 SQLite 里。`.deb` 安装的 `weft-cli task agent:key-create` 可直接用。
