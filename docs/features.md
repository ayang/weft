# 功能详解

本文是 [README](../README.md) 的展开说明，覆盖公开阅读端、管理端、图表、AI、媒体、主题与数据模型。部署相关内容见 [deployment.md](../deployment.md)。

## 公开阅读端

- `/` 首页：置顶、最新文章、标签、分类、开放阅读
- `/posts/:slug`：Markdown 渲染、相关阅读、阅读进度、按 `chart_id` 动态加载图表；正文开头重复标题会在渲染时去掉
- `/posts/:slug.md`：动态 Markdown（含 front matter 与正文）
- `/tags/:slug`、`/categories/:slug`、`/archive`、`/search?q=...`
- `/llms.txt`、`/llms-full.txt`：动态内容地图、文章清单、Agent API 与更新策略
- `/sitemap.xml`、`/robots.txt`
- `GET /api/charts/:id/data`：`{spec,data,updated_at,etag,stale}`，支持 ETag / 304
- `/health`：数据库与缓存健康检查

读者优先：主导航只保留首页、归档、搜索和管理；Agent 通过 `/llms.txt`、`/sitemap.xml` 自行发现站点。

## 管理端

- `/admin/login`：密码登录，JWT 写入 `HttpOnly; SameSite=Strict` cookie（生产自动加 `Secure`），只允许 `ADMIN_EMAIL` 对应账号
- `/admin`：文章、图表、AI 会话概览
- `/admin/posts`：文章管理、Markdown 编辑、标签、草稿/审核/发布流转
- `/admin/categories`：AI 全站分类
- `/admin/charts`：图表列表、手动刷新、cron、失败退避状态
- `/admin/media`：媒体库上传、搜索、复用
- `/admin/settings/site`：站点名称、品牌名、副标题、描述、作者、首页 Banner、站点 URL、每页文章数；保存后立即生效
- `/admin/settings/ai`：AI 提供商、Base URL、模型与 Key；支持 DeepSeek / OpenAI / OpenRouter / Moonshot / Qwen / Groq / SiliconFlow / Ollama 预设

站点设置在 `site_settings` 表中，运行时覆盖 `config/<env>.yaml` 的默认值，不需要重启。`SITE_TITLE` 等环境变量只作为首次启动或数据库为空时的默认值。

## AI 文章编辑（Diff First）

- 提示词 + 多文件材料（Markdown/TXT/HTML/PDF/DOCX/CSV/JSON）+ 最多 5 个公开网页链接
- 可修改现有正文；正文为空时可直接生成标题、摘要、标签和完整正文
- 返回后进入并排/统一 diff，点击“应用修改”才写回编辑器，再手动保存
- 不引入版本历史表；不提供无上下文的“一键生成摘要/卡片/Mermaid/SQL/代码”按钮

## Agent API（只读）

| 端点 | 说明 |
|---|---|
| `GET /api/agent/posts?since=RFC3339&limit=50` | 已发布文章清单，支持 ETag / Last-Modified |
| `GET /api/agent/posts/:slug` | 元数据 + Markdown + 图表引用 |
| `GET /api/agent/charts/:id` | 图表 spec + 最新数据 + ETag |

设置 `AGENT_API_TOKEN` 后，所有 Agent 请求需要 `Authorization: Bearer <token>`，并按分钟限流；不提供任何公开模型调用端点。

## 图表

图表不是图片，而是：

```text
chart_specs（标题、意图、kind、数据源配置、ECharts option、refresh_cron）
       +
chart_data_caches（最近一次成功 payload、ETag、fetched_at、expires_at）
       ↓
GET /api/charts/:id/data → 前端 ECharts 渲染
```

### Chart Studio

1. 输入提示词，可选上传 CSV/TSV/JSON/Markdown/文本文件；
2. AI 解析数据并生成/修改标题、类型、X/Y 字段和格式；
3. 在可编辑表格中修改/增删行列，或粘贴 CSV/TSV；
4. 用下拉框和开关调整类型、配色、排序、堆叠、平滑、横向、图例、标签；
5. 主栏顶部 ECharts 大图实时预览，右侧是 AI 提示词；保存时服务端重新生成并校验受控 spec。

可控制的格式：类型、X/Y 字段、配色、排序、平滑、堆叠、横向、图例、数值标签、高度、轴标题、单位、数值格式（自动/整数/1 位小数/百分比/紧凑）、Y 轴 min/max、对数轴、缩放条、图例位置、双 Y 轴。这些字段同时进入 ECharts spec 与公开图表渲染；`_weft.height` / `_weft.number_format` / `_weft.y_unit` 用于前端高度与数值格式。特殊图表（kline/sankey 等）仍可用“高级 JSON 模式”，该模式也支持实时预览与 `dataset.source` 注入。

### 插入位置

正文单独一行写 `[[chart:ID]]`，图表会渲染在该位置；没有标记的图表仍汇总到文末“数据图表”区域。文章编辑页和图表工作室都提供插入标记的入口。

### 数据源适配器

- **static**：可视化表格保存为 `{"data":[...],"columns":[...],"blueprint":{...}}`，也兼容旧的手工 `{"data":[...]}`。
- **sql**：`{"query":"SELECT label, value FROM posts ...","limit":200}`。
  - 仅允许单条 `SELECT`；禁止 `INSERT/UPDATE/DELETE/PRAGMA/ATTACH/...`、注释和多语句；
  - 所有 `FROM`/`JOIN` 表必须在 `settings.charts.sql_allowed_tables` 中；
  - 有超时与行数上限。
- **http**：`{"url":"https://...","method":"GET","data_path":"data.items"}`。
  - 域名必须在 `settings.charts.http_allowlist` 中（默认空，显式拒绝）；
  - 禁止私有/回环主机、URL 凭据；
  - 有连接超时、响应体大小上限、JSON 校验。

`refresh_cron` 使用 7 段 cron（秒 分 时 日 月 周 年），例如每天 04:00：`"0 0 4 * * * *"`。`charts:refresh_due` 每分钟由 scheduler 调用，按 cron 与 `last_run_at` 判断是否到期；失败写入 `error_message`、`failure_count`、`next_attempt_at`，按指数退避重试，达到 `settings.jobs.max_attempts` 后停止自动重试，可在管理端手动触发。

## AI 自动分类

后台 `/admin/categories` 点击“AI 重新分类全站文章”：

1. 读取全部文章标题、摘要和正文片段；
2. AI 生成 3–8 个可复用分类；
3. 每篇文章归入一个分类；
4. 覆盖旧分类，不修改正文；
5. 分类立即出现在首页、文章卡片、文章页和 `/categories/<slug>`。

离线模式使用关键词规则；真实模型使用 AI 设置中的提供商。操作写入 `ai_sessions`，只允许登录后台执行。

## AI 运行模式

- **离线（默认）**：不访问网络，返回确定性的图表 spec 与草稿框架，方便本地运行和 CI。
- **真实模型**：在 `/admin/settings/ai` 配置提供商预设、Base URL、模型与 Key；环境变量 `OPENAI_API_KEY` / `AI_PROVIDER` / `OPENAI_BASE_URL` / `AI_MODEL` 作为默认值。数据库中的运行时配置优先，Key 只保存在服务端，页面只显示“已配置/未配置”。
- 所有调用经过超时、并发信号量、24h 成本门禁和 `ai_sessions` 审计。
- 公开 AI 端点已移除；AI 仅限登录后台的文章编辑、图表工作室、AI 设置和发布后的后台 Worker。
- LLM 只生成受控 JSON/文本：图表 spec 做 JSON 对象与大小校验，SQL 做只读 + 表白名单校验，模型生成的代码不会被执行。

## 媒体与视频

- 图片：JPG/PNG/WebP/GIF/AVIF，最大 8 MB
- 视频：MP4/WebM，最大 12 MB
- 按文件内容识别类型，扩展名写错也能正常入库
- 文件保存到 `MEDIA_DIR`（默认 `media/uploads/`），公开地址 `/media/<filename>`
- 图片复制 Markdown 插入正文；视频使用 `[[video:文件名]]` 单独一行
- 文章封面可直接上传，也可从媒体库按缩略图选择
- 视频不转码，建议上传 H.264 MP4 或 WebM

`MEDIA_DIR` 是运行时持久数据（已加入 `.gitignore`），备份数据库时必须同时备份该目录。

## 首页 Banner

`/admin/settings/site → 首页 Banner 图片`：

- 留空使用内置 `themes/default/static/hero-banner.svg`
- 可填写外链 URL、直接上传图片，或从媒体库选择
- 图片覆盖整个首页 Hero，左侧叠加纸张色渐变保证文字可读；移动端改为底部渐变

## 主题模板

```text
themes/<name>/templates/  Tera 页面模板
themes/<name>/static/     CSS / JS / ECharts / 静态图片
```

```bash
cp -a themes/default themes/mytheme
SITE_THEME=mytheme ./scripts/start-dev.sh
```

修改模板和 CSS 不需要编译 Rust；切换 `SITE_THEME` 需要重启。详见 [themes/README.md](../themes/README.md)。

## 数据模型

迁移基线位于 `migration/src/m20261103_000000_init.rs`：

- `posts`：slug、标题、作者摘要、Markdown、status、pinned、published_at、reading_minutes
- `tags` / `post_tags`
- `ai_sessions`：purpose、provider、model、messages、tool_calls、status、token/成本估算、错误
- `ai_settings`：管理端保存的运行时提供商、Base URL、模型、Key 与启用状态（Key 不回显）
- `site_settings`：站点名称、品牌、副标题、描述、作者、首页 Banner、URL、每页文章数
- `media`：本地上传图片/视频的元数据；文件位于 `MEDIA_DIR`
- `categories`：AI 生成的分类；`posts.category_id` 关联文章
- `chart_specs`：意图、kind、数据源配置、render_config、cron、最后成功/失败、退避
- `chart_data_caches`：payload、ETag、获取时间、过期时间（chart_id 唯一）
- `chart_recipes`：可复用 prompt 模式、data schema、渲染模板
- `embeddings`：文章分块、可选向量 BLOB、模型；当前默认关键词检索，保留向量接口

## 资源占用

- **单进程单二进制**：SQLite + WAL；启动时设置 `journal_mode=WAL`、`synchronous=NORMAL`、`busy_timeout`、`foreign_keys=ON`
- **连接池**：开发 4、生产 5；SQLite 不需要更大
- **并发限制**：AI、图表刷新、HTTP 拉取各有 `Semaphore`
- **缓存**：Loco InMem 8 MB（进程内）+ SQLite `chart_data_caches`；页面/Agent API 使用 ETag，`llms.txt` 缓存 5 分钟
- **预算**：AI 24h 估算成本上限，超出后拒绝新调用
- **后台任务**：发布只入队，不阻塞请求；scheduler 独立进程负责周期刷新与补偿

## 安全边界

- 不挂载公开注册路由；管理端只接受 `ADMIN_EMAIL` 对应账号
- 管理 cookie：`HttpOnly`、`SameSite=Strict`、生产加 `Secure`；登录有每分钟限流
- Agent API 只读、可选 Bearer token，并按分钟限流
- 所有外部 HTTP 数据源必须显式域名白名单，禁止私有地址与 URL 凭据
- AI Key 可保存在 SQLite（`ai_settings`）；数据库文件应限制权限并纳入备份/加密策略
- AI 文章修改不会自动落库：必须看 diff、点击“应用修改”，再保存
- 外部网页按公开 DNS 校验抓取，拒绝 localhost/私网/链路本地地址，并限制正文大小与超时
- 图表工作室上传限制 1 MiB、UTF-8 文本，解析结果最多 1000 行、40 列
- SQL 数据源只读、表白名单、超时、行数上限
- 日志不打印 API Key、密码或完整用户隐私正文；AI 会话正文存库供管理员审计

## 已知取舍

- Worker 使用 `BackgroundAsync`：请求不阻塞，但作业本身不进入持久队列；发布后的索引由 `posts:enrich_due` 定时补偿，图表由 scheduler 重试。若需要多实例/严格至少一次，可切到 Loco 的 `queue-sqlite`
- 限流与缓存是单进程内存；多实例部署需要换成共享后端
- Mermaid 目前以 Markdown 代码块/文本保存与展示，不引入 Mermaid 运行时；图表前端固定用 ECharts，作为本地静态文件随站点提供，不依赖 CDN 或 Node 构建链
