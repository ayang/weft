# Weft

Weft 是一个 **AI 原生个人站点**：人类写作，AI 辅助编辑、作图、分类和索引，站点以人类读者为主。它不是静态站点生成器——首页、文章、Markdown、`llms.txt`、图表数据与 Agent API 都由 Loco 服务端在请求时生成。

- 单一 Rust 二进制 + SQLite（WAL），无 Node 构建链、无 SPA、无 Redis / 消息队列
- Loco 1.2 / Axum 0.8 / SeaORM 2
- 服务端 Tera 模板 + 少量原生 JS + 本地 ECharts
- 后台 Worker + Loco Scheduler

![首页](docs/screenshots/home.png)

![文章页：标题、元信息、正文与相关阅读](docs/screenshots/post.png)

![图表工作室：主栏实时预览、数据与格式，右侧 AI 提示词](docs/screenshots/chart-studio.png)

![AI 提供商、模型与 Key 设置](docs/screenshots/ai-settings.png)

![AI 文章修改 diff：窄行号、并排/统一视图，手动应用](docs/screenshots/ai-diff.png)

## 核心能力

| 区域 | 能力 |
|---|---|
| 公开阅读端 | 首页、文章、`.md`、标签/分类、归档、搜索、`llms.txt`、`sitemap.xml`、ETag |
| 管理端 | 密码登录（仅 `ADMIN_EMAIL`）、文章 CRUD 与发布流转、分类、媒体库、站点设置、AI 设置 |
| 图表 | Chart Studio 可视化编辑 + AI 修改；static / SQL / HTTP 数据源；cron 刷新与退避重试 |
| AI | 文章修订（材料 + 网页引用 + diff 应用）、从零创作、图表生成、自动分类；全部限定在登录后台 |
| Agent API | 只读文章/图表接口，可选 Bearer token 与限流 |
| 媒体 | 图片 8 MB / 视频 12 MB，内容嗅探、媒体库复用、封面与 Banner 上传 |
| 后台任务 | `refresh_chart`、`index_post`、`cleanup_cache` + Loco Scheduler 定时补偿 |

功能细节（数据源、图表格式、AI 模式、数据模型、安全边界）见 [docs/features.md](docs/features.md)。

## 快速开始

要求：Rust stable。只有生成脚手架 / 实体时才需要 `loco` 与 `sea-orm-cli`。

```bash
# 1. 初始化数据库（默认 weft_development.sqlite）
cargo loco db migrate

# 2. 创建唯一管理员（邮箱需与 settings.site.admin_email / ADMIN_EMAIL 一致）
cargo loco task admin:create \
  email:admin@example.com \
  name:"Admin" \
  password:"change-me-12345"

# 3. 可选：示例文章、标签、索引和一个图表
cargo loco task demo:seed

# 4. 启动
cargo loco start
# http://localhost:5150
```

之后修改 Rust 代码、模板、CSS/JS 都**不需要**重建数据库。日常启动建议：

```bash
./scripts/start-dev.sh   # 启动前自动备份；数据库缺失/为空时拒绝启动
```

忘记密码时不要重建用户：

```bash
cargo loco task admin:reset email:admin@example.com password:"new-strong-password"
```

数据库备份（SQLite `VACUUM INTO`，兼容 WAL）：

```bash
cargo loco task db:backup                        # backups/weft-YYYYmmdd-HHMMSS.sqlite
cargo loco task db:backup path:backups/before-migration.sqlite
```

恢复前先停止服务，再把备份复制回数据库文件，并删除旧的 `-wal` / `-shm`。

## 配置

所有配置在 `config/<env>.yaml`（`development` / `test` / `production`），密钥通过 `get_env(...)` 插值，应用代码不直接读环境变量。

常用变量：

| 变量 | 默认 | 用途 |
|---|---|---|
| `LOCO_ENV` | `development` | `development` / `test` / `production` |
| `DATABASE_URL` | 开发 `weft_development.sqlite`，生产必填 | `sqlite://...` |
| `JWT_SECRET` | 生产必填 | 管理员 cookie 签名 |
| `ADMIN_EMAIL` | `admin@example.com` | 唯一允许登录 `/admin` 的邮箱 |
| `SITE_BASE_URL` | `http://localhost:5150` | sitemap / llms.txt / Agent API 绝对链接 |
| `SITE_THEME` | `default` | 主题目录名，修改后重启 |
| `MEDIA_DIR` | `media/uploads` | 上传文件目录；生产建议用绝对路径 |
| `AGENT_API_TOKEN` | 空 | 设置后 `/api/agent/*` 需要 Bearer token |
| `AI_PROVIDER` | `offline` | `offline` 或 OpenAI-compatible 提供商 |
| `OPENAI_API_KEY` / `OPENAI_BASE_URL` / `AI_MODEL` | 空 / OpenAI / `gpt-4o-mini` | 模型默认值；后台 AI 设置优先 |

完整变量表、AI 提供商说明和生产环境要求见：

- **部署**：[deployment.md](deployment.md)（systemd、nginx、独立配置目录、自定义 `MEDIA_DIR`、备份与升级；也可用 `packaging/build-deb.sh` 打 `.deb` 一键安装）
- **功能**：[docs/features.md](docs/features.md)
- **主题**：[themes/README.md](themes/README.md)

管理后台：`/admin/settings/site`（站点名称、品牌、副标题、描述、作者、Banner、URL、每页文章数）、`/admin/settings/ai`（提供商、模型、Key）。数据库中的设置为运行时配置，保存后立即生效；`SITE_TITLE` 等环境变量只是首次启动的默认值。

后台 Worker 默认为 `BackgroundAsync`。需要 scheduler 时：

```bash
# 与 Web 一起跑
SCHEDULER_CONFIG=config/scheduler.yaml cargo loco start --all

# 或独立进程（生产用，见 deployment.md）
cargo loco scheduler --config config/scheduler.yaml
```

## 媒体目录

上传文件默认写入 `media/uploads/`，公开路径固定为 `/media/<filename>`。生产环境把 `MEDIA_DIR` 指向持久目录（例如 `/var/lib/weft/media/uploads`），升级时就不会被覆盖：

```bash
MEDIA_DIR=/var/lib/weft/media/uploads
```

`MEDIA_DIR` 是运行时数据，已加入 `.gitignore`；备份数据库时必须同时备份这个目录。

## 开发与验证

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

集成测试使用隔离的 `weft_test.sqlite`、离线 AI 与 `ForegroundBlocking`，不访问外网。清理测试库用 `./scripts/clean-test-db.sh`，不要用 `rm weft_*.sqlite` 这类通配符。

## 目录结构

```text
src/
  controllers/      public_pages, admin, chart_api, agent_api, llms
  models/           posts, tags, chart_*, embeddings, ai_sessions, ai_settings, site_settings, media
  charts/           数据源适配器、SQL 守卫、cron 到期判断、图表蓝图
  ai/               OpenAI-compatible 客户端 + 离线回退 + prompt/JSON 校验
  media.rs          上传校验、存储与公开 URL
  security.rs       管理 cookie、Agent token、限流、并发信号量
  workers/          refresh_chart, index_post, cleanup_cache
  tasks/            admin:create, admin:reset, db:backup, demo:seed, charts:refresh_due, ...
  app.rs            AppRoutes / Worker / Task / WAL Pragma / Limits
themes/<name>/      templates + static（Tera、CSS、JS、ECharts）
migration/src/      初始化基线 + 后续增量迁移
config/             development / test / production + scheduler.yaml
deploy/             systemd unit、env 模板、nginx 示例、备份脚本、安装脚本
tests/              集成测试与模型/任务测试
```

## 已知取舍

- Worker 为 `BackgroundAsync`：请求不阻塞，但作业不入持久队列；由 scheduler 定时补偿。多实例/严格至少一次可切到 `queue-sqlite`。
- 限流与缓存是单进程内存；多实例部署需要共享后端。
- 图表固定用本地 ECharts，不依赖 CDN 或 Node 构建链。
