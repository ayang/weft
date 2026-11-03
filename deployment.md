# 生产部署指南（Linux）

本文描述如何把 Weft 以**单二进制 + SQLite + systemd** 的方式部署到一台 Linux 服务器（1–2 GB 内存的 VPS 即可），并用 nginx 终止 TLS。

仓库里已经附带可直接安装的文件：

| 文件 | 用途 |
|---|---|
| [`deploy/weft.service`](deploy/weft.service) | Web 服务 unit |
| [`deploy/weft-scheduler.service`](deploy/weft-scheduler.service) | 定时任务 unit（图表刷新、索引补偿、缓存清理） |
| [`deploy/weft-backup.service`](deploy/weft-backup.service) / [`deploy/weft-backup.timer`](deploy/weft-backup.timer) | 每日数据库 + 媒体备份 |
| [`deploy/weft-backup.sh`](deploy/weft-backup.sh) | 备份脚本（安装到 `/usr/local/bin/weft-backup`） |
| [`deploy/weft.env.example`](deploy/weft.env.example) | 环境变量模板 |
| [`deploy/nginx.conf.example`](deploy/nginx.conf.example) | 反向代理与 TLS 配置 |
| [`deploy/install.sh`](deploy/install.sh) | 一键安装（放文件、建用户、装 unit，生成随机密钥，不自动启动） |
| [`packaging/build-deb.sh`](packaging/build-deb.sh) | 构建 `.deb`（可选 `--install` 直接安装） |
| [`packaging/debian/`](packaging/debian) | dpkg 的 `postinst` / `prerm` / `postrm` |
| [`packaging/systemd/`](packaging/systemd) | 包内使用的 systemd unit（路径为 `/usr/bin`、`/usr/share/weft`） |

## 目录规划

固定这几个路径，升级时只替换二进制和主题，数据不动：

| 路径 | 内容 | 所有者 |
|---|---|---|
| `/usr/local/bin/weft-cli` | release 二进制 | root |
| `/srv/weft/` | 工作目录：`themes/`、`assets/`（i18n） | root |
| `/etc/weft/` | `production.yaml`、`scheduler.yaml`、`weft.env` | root:weft（目录 0750，文件 0640） |
| `/var/lib/weft/` | `weft.sqlite`（+ WAL/SHM）、`media/uploads/` | weft |
| `/var/backups/weft/` | 数据库备份、媒体归档 | weft |

进程以系统用户 `weft` 运行，工作目录是 `/srv/weft`。**配置不在工作目录里**：通过 `LOCO_CONFIG_FOLDER=/etc/weft` 指定；主题与 i18n 资源必须留在工作目录，因为模板、静态文件和翻译包都按相对路径加载。

```text
浏览器 ──TLS──▶ nginx ──127.0.0.1:5150──▶ weft-cli start
                                            │
                                            ├── /var/lib/weft/weft.sqlite (WAL)
                                            ├── /var/lib/weft/media/uploads
                                            └── workers（进程内异步）

weft-cli scheduler ── spawned task ──▶ 同一个二进制 ./weft-cli task <name>
```

## 1. 前置条件

- Linux（Debian/Ubuntu 或 RHEL 系均可）、systemd
- nginx（或 Caddy）、`certbot`（可选，用于 Let's Encrypt）
- 构建机：Rust stable（建议在 Linux 上构建；跨平台构建见文末）
- 服务器上不需要 Rust 工具链

## 2. 构建 release 二进制

在仓库根目录：

```bash
cargo build --release --bin weft-cli
strip target/release/weft-cli        # 可选，减小体积
ls -lh target/release/weft-cli
```

若在别的机器构建，把 `target/release/weft-cli` 拷到服务器即可；**不要**只拷 `weft`、也不要拷 `target/` 其余内容。运行时只需要这个二进制 + `themes/` + `assets/` + 配置文件。

> glibc 版本不匹配时，可在与目标系统相同或更旧的发行版上构建，或使用 musl 目标：`cargo build --release --bin weft-cli --target x86_64-unknown-linux-musl`。

## 3. 一键安装（推荐）

```bash
# 在仓库根目录，已构建 release 二进制
sudo bash ./deploy/install.sh
```

脚本会：

1. 创建系统用户 `weft`；
2. 建立 `/srv/weft`、`/var/lib/weft`、`/var/lib/weft/media/uploads`、`/var/backups/weft`、`/etc/weft`；
3. 把 `themes/`、`assets/` 同步到 `/srv/weft`；
4. 安装 `weft-cli` 与 `weft-backup`；
5. 首次运行时生成 `/etc/weft/production.yaml`、`scheduler.yaml`、`weft.env`（已存在则保留，`weft.env` 内含随机 `JWT_SECRET` / `AGENT_API_TOKEN`）；
6. 安装 systemd unit 并 `daemon-reload`。

然后按第 5、6 节填配置、初始化数据库，再启动服务。

想手动安装就跳过本节，照第 4–8 节做，命令等价。

### 构建 .deb 并一键安装

在 Debian/Ubuntu（或 WSL）上：

```bash
bash packaging/build-deb.sh              # 产物：dist/weft_<version>_<arch>.deb
sudo apt install ./dist/weft_<version>_<arch>.deb
```

`bash packaging/build-deb.sh --install` 可在构建后直接安装。脚本不需要网络，也不依赖 `cargo-deb`，只用 `dpkg-deb`。

包安装后的路径由 dpkg 管理，与源码安装略不同：

| 路径 | 内容 |
|---|---|
| `/usr/bin/weft-cli` | 二进制 |
| `/usr/share/weft/` | `themes/`、`assets/`、`weft.env.example`（升级时覆盖） |
| `/etc/weft/` | `production.yaml`、`scheduler.yaml`（conffile）、首次安装生成的 `weft.env` |
| `/var/lib/weft/` | 数据库与上传文件 |
| `/usr/share/doc/weft/` | README、部署与功能文档 |

`postinst` 会自动完成：创建 `weft` 系统用户与目录；生成随机 `JWT_SECRET` 与 `AGENT_API_TOKEN`；执行 `weft-cli db migrate`；启用并启动 `weft.service`、`weft-scheduler.service`、`weft-backup.timer`。

安装后仍需设置 `ADMIN_EMAIL` / `SITE_BASE_URL` 并创建管理员：

```bash
sudoedit /etc/weft/weft.env
sudo systemctl restart weft.service weft-scheduler.service
sudo -u weft sh -c 'cd /usr/share/weft && set -a && . /etc/weft/weft.env && set +a && weft-cli task admin:create email:you@example.com name:"Admin" password:"change-me-12345"'
```

升级时重新构建并 `sudo apt install ./dist/weft_*.deb`：`postinst` 会再次迁移并启动服务；迁移失败时不会启动，避免新二进制跑在旧表结构上。`apt remove weft` 保留数据；`apt purge weft` 删除配置和系统用户，但**仍然保留** `/var/lib/weft` 与 `/var/backups/weft`。

## 4. 手动安装文件

```bash
sudo useradd --system --home-dir /srv/weft --shell /usr/sbin/nologin --user-group weft || true

sudo install -d -m 0755 /srv/weft
sudo install -d -m 0750 -o weft -g weft /var/lib/weft /var/lib/weft/media/uploads /var/backups/weft
sudo install -d -m 0750 -o root -g weft /etc/weft

# 主题与 i18n（必须与工作目录同级）
sudo rsync -a --delete themes/ /srv/weft/themes/
sudo rsync -a --delete assets/ /srv/weft/assets/

# 二进制与备份脚本
sudo install -m 0755 target/release/weft-cli /usr/local/bin/weft-cli
sudo install -m 0755 deploy/weft-backup.sh /usr/local/bin/weft-backup

# 配置模板
sudo install -m 0640 -o root -g weft config/production.yaml /etc/weft/production.yaml
sudo install -m 0640 -o root -g weft config/scheduler.yaml  /etc/weft/scheduler.yaml
sudo install -m 0640 -o root -g weft deploy/weft.env.example /etc/weft/weft.env

# systemd
sudo install -m 0644 deploy/weft.service deploy/weft-scheduler.service \
  deploy/weft-backup.service deploy/weft-backup.timer /etc/systemd/system/
sudo systemctl daemon-reload
```

## 5. 配置

### 5.1 `/etc/weft/weft.env`

systemd 通过 `EnvironmentFile=` 读取。`weft.env.example` 里的每一项都有注释，最少要改：

```bash
LOCO_ENV=production
LOCO_CONFIG_FOLDER=/etc/weft
BINDING=127.0.0.1
PORT=5150

DATABASE_URL=sqlite:///var/lib/weft/weft.sqlite?mode=rwc
MEDIA_DIR=/var/lib/weft/media/uploads
DB_AUTO_MIGRATE=false

JWT_SECRET=REPLACE_WITH_OPENSSL_RAND_BASE64_48
ADMIN_EMAIL=you@example.com
SITE_BASE_URL=https://weft.example.com
AGENT_API_TOKEN=REPLACE_WITH_OPENSSL_RAND_HEX_32   # 可留空，留空则 Agent API 公开

SITE_THEME=default
```

> `JWT_SECRET` 用 `openssl rand -base64 48` 生成，`AGENT_API_TOKEN` 用 `openssl rand -hex 32` 生成。

`/etc/weft` 应为 `0750 root:weft`，配置与 `weft.env` 为 `0640 root:weft`：服务进程以 `weft` 运行，`sudo -u weft ... db migrate` 这类维护命令也需要读取它们，所以不能是 `0600 root:root`。

注意：

- `SITE_BASE_URL` 同时用于生成 `llms.txt`、`sitemap.xml`、Agent API 的绝对链接，且是 `server.host` 的来源，**必须是最终对外地址**。
- `DATABASE_URL` 用三个斜杠表示绝对路径：`sqlite:///var/lib/weft/weft.sqlite?mode=rwc`。
- `MEDIA_DIR` 必须是**绝对路径**，否则相对工作目录 `/srv/weft`，升级时容易被覆盖。
- `JWT_SECRET` 一旦上线不要再改，否则所有管理员 cookie 失效；换密钥等于强制重新登录。
- `DB_AUTO_MIGRATE=false` 是推荐值：迁移作为发布步骤显式执行，避免 Web 进程和 scheduler 进程抢着迁移。
- 含有空格的值要加引号（如 `SITE_TAGLINE="人与 AI 的编织现场"`）；备份脚本会 `source` 这个文件，引号能保证 systemd 和 shell 两种解析都正确。

### 5.2 `/etc/weft/production.yaml`

仓库里的 `config/production.yaml` 已按生产环境写好：日志 JSON、`binding` 默认 `0.0.0.0`（由 `BINDING` 覆盖为回环）、静态目录指向 `themes/<SITE_THEME>/static`、数据库与密钥强制从环境变量读取、缺失即启动失败。

一般不需要改。要改的话保持这两条：

```yaml
server:
  middlewares:
    static:
      folder:
        path: "themes/<%= get_env(name="SITE_THEME", default="default") %>/static"
```

以及 `settings.media.upload_dir` 由 `MEDIA_DIR` 注入。

### 5.3 环境变量一览

| 变量 | 生产默认 / 是否必填 | 用途 |
|---|---|---|
| `LOCO_ENV` | 必填 `production` | 选择 `config/<env>.yaml` |
| `LOCO_CONFIG_FOLDER` | 建议 `/etc/weft` | 配置目录（目录内含 `production.yaml`） |
| `DATABASE_URL` | 必填 | SQLite 绝对路径 URI |
| `JWT_SECRET` | 必填 | 管理员 cookie 签名 |
| `ADMIN_EMAIL` | 建议必填 | 唯一允许登录 `/admin` 的邮箱 |
| `SITE_BASE_URL` | 必填 | 对外 URL、sitemap/llms.txt 绝对链接 |
| `BINDING` / `PORT` | `127.0.0.1` / `5150` | 监听地址 |
| `MEDIA_DIR` | 建议绝对路径 | 上传文件目录，默认 `media/uploads` |
| `SITE_THEME` | `default` | 主题目录名，改后需重启 |
| `SITE_TITLE` / `SITE_BRAND` / `SITE_TAGLINE` / `SITE_DESCRIPTION` / `SITE_AUTHOR` | 可选 | 站点文案默认值（后台可覆盖） |
| `DB_MAX_CONNECTIONS` | `5` | SQLite 连接池上限 |
| `DB_AUTO_MIGRATE` | 建议 `false` | 启动时自动迁移 |
| `LOG_LEVEL` | `info` | `trace/debug/info/warn/error` |
| `AGENT_API_TOKEN` | 可空 | 设置后 `/api/agent/*` 需要 Bearer token |
| `AGENT_RATE_LIMIT_PER_MIN` | `30` | Agent 读请求限流 |
| `AI_PROVIDER` | `offline` | `offline` 或 OpenAI-compatible 提供商 |
| `OPENAI_API_KEY` / `OPENAI_BASE_URL` / `AI_MODEL` | 可选 | 模型默认值；后台 AI 设置优先 |
| `AI_DAILY_COST_LIMIT_USD` | `1.0` | 24 小时估算成本上限 |
| `SCHEDULER_CONFIG` | 可选 | 不传 `--config` 时的调度器配置路径 |

### 5.4 自定义配置位置

`LOCO_CONFIG_FOLDER` 指向的目录只需包含 `<env>.yaml`；调度器另用 `--config` 指定自己的文件。若你想把配置放到别处（例如 `/opt/weft/etc`），改这两处即可：

```bash
# weft.env
LOCO_CONFIG_FOLDER=/opt/weft/etc
```

```ini
# weft-scheduler.service
ExecStart=/usr/local/bin/weft-cli scheduler --config /opt/weft/etc/scheduler.yaml
```

## 6. 初始化数据库与管理员

以 `weft` 用户、带上环境变量执行（首次运行会创建 `/var/lib/weft/weft.sqlite`）：

```bash
sudo -u weft bash -c '
  cd /srv/weft && set -a && . /etc/weft/weft.env && set +a &&
  weft-cli db migrate
'
```

创建唯一管理员（邮箱必须与 `ADMIN_EMAIL` 一致，密码至少 10 位）：

```bash
sudo -u weft bash -c '
  cd /srv/weft && set -a && . /etc/weft/weft.env && set +a &&
  weft-cli task admin:create email:you@example.com name:"Admin" password:"change-me-12345"
'
```

可选：写入示例内容（只在空库上做）：

```bash
sudo -u weft bash -c '
  cd /srv/weft && set -a && . /etc/weft/weft.env && set +a &&
  weft-cli task demo:seed
'
```

忘记密码时**不要**重建数据库：

```bash
weft-cli task admin:reset email:you@example.com password:"new-strong-password"
```

## 7. 启动服务

```bash
sudo systemctl enable --now weft.service
sudo systemctl enable --now weft-scheduler.service
sudo systemctl enable --now weft-backup.timer

systemctl status weft.service --no-pager
journalctl -u weft.service -f
```

健康检查（不需要登录，返回数据库/缓存状态）：

```bash
curl -fsS http://127.0.0.1:5150/health
```

### systemd unit 说明

`deploy/weft.service` 的关键点：

- `WorkingDirectory=/srv/weft`：主题、`assets/i18n`、静态文件都按这个目录解析。
- `EnvironmentFile=/etc/weft/weft.env`：所有密钥和地址只在这里。
- `StateDirectory=weft`：systemd 创建并授权 `/var/lib/weft`，配合 `ProtectSystem=strict` 后应用只能写这一个运行目录。
- `ReadWritePaths=/var/backups/weft`：备份任务需要写这里。
- `UMask=0027`：数据库和上传文件默认不对其他用户可读。
- `ProtectSystem=strict`、`ProtectHome=true`、`NoNewPrivileges=true`：应用被攻破时减少影响面。
- `MemoryMax=900M`：1 GB 机器建议 700–900M；2 GB 机器可放宽到 1.5G。

`weft-scheduler.service` 与 Web 服务共用二进制和配置。调度器触发任务时会 `exec` 同一个二进制执行 `task <name>`，因此它必须有相同的 `EnvironmentFile`、`WorkingDirectory` 和写权限——unit 里已配好。**不要**把 scheduler 和 `start --all` 同时开两份，否则任务会重复执行。

## 8. nginx 反向代理与 HTTPS

```bash
sudo cp deploy/nginx.conf.example /etc/nginx/conf.d/weft.conf
sudo $EDITOR /etc/nginx/conf.d/weft.conf   # 改成你的域名与证书路径
sudo nginx -t && sudo systemctl reload nginx
```

示例配置里已经处理：

- `client_max_body_size 20m`：媒体上传（图片 8 MB / 视频 12 MB）需要；
- `proxy_read_timeout 120s`：AI 文章编辑是同步调用，可能接近 60s；
- `X-Forwarded-Proto`：让应用知道外部是 HTTPS；
- HSTS 默认注释掉，确认全站 HTTPS 后再开。

Let's Encrypt 首次签发：

```bash
sudo apt install certbot python3-certbot-nginx   # Debian/Ubuntu
sudo certbot --nginx -d weft.example.com
```

## 9. 自定义媒体目录

上传文件默认在 `media/uploads`，生产建议放到 `/var/lib/weft/media/uploads`：

```bash
# /etc/weft/weft.env
MEDIA_DIR=/var/lib/weft/media/uploads
```

应用在启动时会自动创建该目录；目录权限由 systemd `StateDirectory=weft` 保证（`weft:weft 0750`）。检查：

```bash
ls -ld /var/lib/weft/media/uploads
```

公开访问路径始终是 `/media/<filename>`，与 `MEDIA_DIR` 无关，所以换目录不需要改模板、也不需要迁移公开 URL。

想迁移已有上传文件：

```bash
sudo systemctl stop weft.service
sudo rsync -a media/uploads/ /var/lib/weft/media/uploads/
sudo chown -R weft:weft /var/lib/weft/media
sudo systemctl start weft.service
```

**数据库只存媒体元数据，文件本身在这个目录。** 备份数据库不等于备份媒体，二者都要备份（见第 10 节）。

如果让 nginx 直接提供 `/media/`（少一次 Rust 读取），要打开 `nginx.conf.example` 中注释的 `location /media/`，并让 nginx 能遍历目录：

```bash
sudo chmod 0711 /var/lib/weft
sudo chmod 0755 /var/lib/weft/media /var/lib/weft/media/uploads
```

`alias` 必须与 `MEDIA_DIR` 保持一致。默认配置不启用这一项，代理给应用最简单、权限也最少。

## 10. 备份与恢复

### 自动备份

`weft-backup.timer` 每天 03:15（带随机延迟）执行 `weft-backup`：

1. `weft-cli task db:backup` 用 SQLite `VACUUM INTO` 做一致性快照（WAL 下也可用）；
2. 把 `MEDIA_DIR` 打成 `media-*.tar.gz`；
3. 删除 `/var/backups/weft` 下 14 天前的文件（`WEFT_BACKUP_KEEP_DAYS` 可改）。

```bash
systemctl list-timers weft-backup.timer
sudo systemctl start weft-backup.service   # 手动跑一次
ls -lh /var/backups/weft/
```

备份目录建议再同步到另一块盘或对象存储，例如 `restic` / `rclone`。

### 恢复

```bash
sudo systemctl stop weft.service
sudo systemctl stop weft-scheduler.service

# 1) 数据库
sudo install -m 0640 -o weft -g weft weft-YYYYmmdd-HHMMSS.sqlite /var/lib/weft/weft.sqlite
sudo rm -f /var/lib/weft/weft.sqlite-wal /var/lib/weft/weft.sqlite-shm

# 2) 媒体
sudo tar -C /var/lib/weft/media/uploads -xzf media-YYYYmmdd-HHMMSS.tar.gz
sudo chown -R weft:weft /var/lib/weft

sudo systemctl start weft.service
sudo systemctl start weft-scheduler.service
curl -fsS http://127.0.0.1:5150/health
```

## 11. 主题与静态文件

```text
/srv/weft/themes/<name>/templates/   Tera 模板
/srv/weft/themes/<name>/static/      CSS / JS / ECharts / 图片
```

- 修改现有模板或 CSS：直接改 `/srv/weft/themes/...`，生产环境不热重载，需要 `systemctl restart weft`。
- 切换主题：`SITE_THEME=mytheme` 后重启。
- 自定义主题：在仓库里 `cp -a themes/default themes/mytheme`，重新执行 `install.sh` 或手动 `rsync` 到 `/srv/weft/themes/`。
- 上传的媒体**不要**放进主题目录。

## 12. 升级与回滚

```bash
# 1) 构建新版本
cargo build --release --bin weft-cli
strip target/release/weft-cli

# 2) 备份
sudo systemctl start weft-backup.service

# 3) 停服务（scheduler 依赖 weft.service，会一起停）
sudo systemctl stop weft.service weft-scheduler.service

# 4) 替换二进制（保留旧版本以便回滚）
sudo cp /usr/local/bin/weft-cli /usr/local/bin/weft-cli.prev
sudo install -m 0755 target/release/weft-cli /usr/local/bin/weft-cli

# 5) 同步主题与 i18n
sudo rsync -a --delete themes/ /srv/weft/themes/
sudo rsync -a --delete assets/ /srv/weft/assets/

# 6) 迁移
sudo -u weft bash -c 'cd /srv/weft && set -a && . /etc/weft/weft.env && set +a && weft-cli db migrate'

# 7) 启动并验证
sudo systemctl start weft.service weft-scheduler.service
curl -fsS http://127.0.0.1:5150/health
```

回滚：

```bash
sudo systemctl stop weft.service weft-scheduler.service
sudo install -m 0755 /usr/local/bin/weft-cli.prev /usr/local/bin/weft-cli
# 数据库迁移通常不可逆；若有迁移，先用备份恢复数据库
sudo systemctl start weft.service weft-scheduler.service
```

发布前建议在 `DB_AUTO_MIGRATE=false` 的前提下显式跑迁移，这样迁移失败不会让服务进入半启动状态。

## 13. 日志与排错

日志走 stdout，由 journald 收集（`config/production.yaml` 中 `logger.format: json`）：

```bash
journalctl -u weft.service -f
journalctl -u weft.service --since "10 min ago" -o cat
journalctl -u weft-scheduler.service -n 100
journalctl -u weft-backup.service -n 50
```

| 现象 | 排查 |
|---|---|
| 启动即退出，日志提示缺变量 | `EnvironmentFile` 里少了 `DATABASE_URL` / `JWT_SECRET`；`systemctl show weft -p Environment` 检查是否被加载 |
| `unable to open database file` | `/var/lib/weft` 权限或 `DATABASE_URL` 路径错误；确认 `weft` 用户可写 |
| 页面 404 / 样式丢失 | `WorkingDirectory` 不是 `/srv/weft`，或 `themes/`、`assets/` 未同步 |
| 上传失败 | `MEDIA_DIR` 不可写，或 nginx `client_max_body_size` 太小 |
| 图表不刷新 | `weft-scheduler.service` 未启动；`journalctl -u weft-scheduler` |
| 管理端登录后立刻掉线 | `SITE_BASE_URL` 与实际访问协议/域名不一致，或改过 `JWT_SECRET` |
| 502 | nginx 指向的端口与应用 `PORT` 不一致，或应用绑定在 `0.0.0.0` 之外 |

## 14. 上线检查清单

- [ ] `JWT_SECRET` 与 `AGENT_API_TOKEN` 是随机生成、未提交到仓库的
- [ ] `/etc/weft/weft.env` 权限 `0640`，所有者为 `root:weft`
- [ ] `ADMIN_EMAIL` 与实际管理员邮箱一致，密码不是示例密码
- [ ] `SITE_BASE_URL` 是 HTTPS 的最终域名
- [ ] `BINDING=127.0.0.1`，5150 端口没有直接暴露到公网
- [ ] `MEDIA_DIR` 是绝对路径，位于 `/var/lib/weft` 下
- [ ] `weft-backup.timer` 已启用，且成功跑过一次
- [ ] `/var/backups/weft` 之外还有一份异地/对象存储备份
- [ ] `DB_AUTO_MIGRATE=false`，迁移在发布流程里显式执行
- [ ] nginx 已配置 `client_max_body_size` 与较长的 `proxy_read_timeout`
- [ ] `journalctl -u weft` 无持续报错

## 附：从 Windows 交叉构建

最省事的方式是在 Linux 容器/CI 里构建（`rust:1-bookworm` 镜像执行 `cargo build --release --bin weft-cli`）。若坚持在 Windows 上构建：

```powershell
rustup target add x86_64-unknown-linux-musl
cargo install cargo-zigbuild
cargo zigbuild --release --bin weft-cli --target x86_64-unknown-linux-musl
```

产物在 `target/x86_64-unknown-linux-musl/release/weft-cli`。**不要**把 Windows 的 `target/release/weft-cli.exe` 传到 Linux。
