#!/usr/bin/env bash
# Weft 生产环境安装脚本。在仓库根目录以 root 运行：
#
#   cargo build --release --bin weft-cli
#   sudo bash ./deploy/install.sh
#
# 可用环境变量覆盖默认路径：
#   APP_DIR=/srv/weft CONF_DIR=/etc/weft DATA_DIR=/var/lib/weft BACKUP_DIR=/var/backups/weft
#
# 脚本只负责“把文件放到正确位置”，不会自动填写密钥，也不会自动启动服务。
set -euo pipefail

if [[ ${EUID} -ne 0 ]]; then
  echo "请用 root 运行：sudo $0" >&2
  exit 1
fi

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
APP_DIR="${APP_DIR:-/srv/weft}"
CONF_DIR="${CONF_DIR:-/etc/weft}"
DATA_DIR="${DATA_DIR:-/var/lib/weft}"
BACKUP_DIR="${BACKUP_DIR:-/var/backups/weft}"
BIN_SRC="${BIN_SRC:-$REPO_DIR/target/release/weft-cli}"

if [[ ! -x "$BIN_SRC" ]]; then
  echo "找不到 release 二进制：$BIN_SRC" >&2
  echo "请先运行：cargo build --release --bin weft-cli" >&2
  exit 1
fi

echo "==> 创建系统用户 weft"
if ! id -u weft >/dev/null 2>&1; then
  useradd --system --home-dir "$APP_DIR" --shell /usr/sbin/nologin --user-group weft
fi

echo "==> 创建目录"
install -d -m 0755 "$APP_DIR"
install -d -m 0750 -o weft -g weft "$DATA_DIR" "$DATA_DIR/media/uploads" "$BACKUP_DIR"
install -d -m 0750 -o root -g weft "$CONF_DIR"

copy_tree() {
  local src="$1" dst="$2"
  if command -v rsync >/dev/null 2>&1; then
    rsync -a --delete "$src/" "$dst/"
  else
    cp -a "$src/." "$dst/"
  fi
}

echo "==> 同步主题与静态资源到 $APP_DIR"
copy_tree "$REPO_DIR/themes" "$APP_DIR/themes"
copy_tree "$REPO_DIR/assets" "$APP_DIR/assets"
chown -R root:root "$APP_DIR/themes" "$APP_DIR/assets"

echo "==> 安装二进制"
install -m 0755 "$BIN_SRC" /usr/local/bin/weft-cli
install -m 0755 "$REPO_DIR/deploy/weft-backup.sh" /usr/local/bin/weft-backup

echo "==> 安装配置"
if [[ ! -f "$CONF_DIR/production.yaml" ]]; then
  install -m 0640 -o root -g weft "$REPO_DIR/config/production.yaml" "$CONF_DIR/production.yaml"
else
  echo "    保留已有 $CONF_DIR/production.yaml"
fi
if [[ ! -f "$CONF_DIR/scheduler.yaml" ]]; then
  install -m 0640 -o root -g weft "$REPO_DIR/config/scheduler.yaml" "$CONF_DIR/scheduler.yaml"
else
  echo "    保留已有 $CONF_DIR/scheduler.yaml"
fi
if [[ ! -f "$CONF_DIR/weft.env" ]]; then
  install -m 0640 -o root -g weft "$REPO_DIR/deploy/weft.env.example" "$CONF_DIR/weft.env"
  secret="$(head -c 48 /dev/urandom | base64 | tr -d '\n')"
  token="$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')"
  sed -i "s|^JWT_SECRET=.*|JWT_SECRET=$secret|" "$CONF_DIR/weft.env"
  sed -i "s|^AGENT_API_TOKEN=.*|AGENT_API_TOKEN=$token|" "$CONF_DIR/weft.env"
  chown root:weft "$CONF_DIR/weft.env"
  chmod 0640 "$CONF_DIR/weft.env"
  echo "    已生成 $CONF_DIR/weft.env（含随机 JWT_SECRET / AGENT_API_TOKEN）"
else
  echo "    保留已有 $CONF_DIR/weft.env"
fi

echo "==> 安装 systemd unit"
install -m 0644 "$REPO_DIR"/deploy/weft.service "$REPO_DIR"/deploy/weft-scheduler.service \
  "$REPO_DIR"/deploy/weft-backup.service "$REPO_DIR"/deploy/weft-backup.timer \
  /etc/systemd/system/
systemctl daemon-reload

cat <<EOF

安装完成。接下来：

1. 编辑 $CONF_DIR/weft.env 并填写密钥：
     JWT_SECRET=\$(openssl rand -base64 48)
     ADMIN_EMAIL=you@example.com
     SITE_BASE_URL=https://your.domain

2. 初始化数据库（生成 /var/lib/weft/weft.sqlite）：
     sudo -u weft bash -c 'cd $APP_DIR && set -a && . $CONF_DIR/weft.env && set +a && weft-cli db migrate'

3. 创建管理员（邮箱必须与 ADMIN_EMAIL 一致）：
     sudo -u weft bash -c 'cd $APP_DIR && set -a && . $CONF_DIR/weft.env && set +a && weft-cli task admin:create email:you@example.com name:"Admin" password:"change-me-12345"'

4. 启用服务：
     systemctl enable --now weft.service weft-scheduler.service weft-backup.time

5. 配置 nginx 反向代理，参考 deploy/nginx.conf.example。

EOF
