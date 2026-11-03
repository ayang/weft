#!/usr/bin/env bash
# 构建 Weft 的 Debian 安装包并（可选）安装。
#
# 用法（在任意目录执行）：
#   bash packaging/build-deb.sh                 # 构建 dist/weft_<version>_<arch>.deb
#   bash packaging/build-deb.sh --install       # 构建后立即用 apt 安装
#   BIN=/path/to/weft-cli VERSION=0.2.0 bash packaging/build-deb.sh
#
# 需要 Debian/Ubuntu（或 WSL）提供 dpkg-deb；在 Windows 原生环境无法打包。
set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR"

usage() {
  sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'
}

INSTALL=0
for arg in "$@"; do
  case "$arg" in
    --install) INSTALL=1 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "未知参数：$arg" >&2; usage >&2; exit 1 ;;
  esac
done

if ! command -v dpkg-deb >/dev/null 2>&1; then
  echo "错误：找不到 dpkg-deb。请在 Debian/Ubuntu、WSL 或 Debian 容器中运行。" >&2
  echo "容器示例：" >&2
  echo "  docker run --rm -v \"\$(pwd):/src\" -w /src rust:1-bookworm bash -c 'cargo build --release --bin weft-cli && bash packaging/build-deb.sh'" >&2
  exit 1
fi

PKG="weft"
VERSION="${VERSION:-$(awk -F'"' '/^version = /{print $2; exit}' Cargo.toml)}"
ARCH="${ARCH:-$(dpkg --print-architecture)}"
MAINTAINER="${MAINTAINER:-Weft Maintainers <weft@example.com>}"
BIN="${BIN:-target/release/weft-cli}"
OUT="${OUT:-dist}"

if [[ -z "$VERSION" ]]; then
  echo "错误：无法从 Cargo.toml 读取版本，请设置 VERSION=x.y.z" >&2
  exit 1
fi

if [[ ! -x "$BIN" ]]; then
  echo "==> 未找到 $BIN，开始构建 release 二进制"
  cargo build --release --bin weft-cli
fi

STAGE="$OUT/.build/${PKG}_${VERSION}_${ARCH}"
DEB="$OUT/${PKG}_${VERSION}_${ARCH}.deb"

echo "==> 准备打包目录 $STAGE"
rm -rf "$STAGE"
install -d \
  "$STAGE/DEBIAN" \
  "$STAGE/usr/bin" \
  "$STAGE/usr/lib/weft" \
  "$STAGE/usr/share/weft" \
  "$STAGE/usr/share/doc/weft" \
  "$STAGE/etc/weft" \
  "$STAGE/lib/systemd/system"

install -m 0755 "$BIN" "$STAGE/usr/bin/weft-cli"
install -m 0755 deploy/weft-backup.sh "$STAGE/usr/lib/weft/weft-backup"

cp -a themes "$STAGE/usr/share/weft/themes"
cp -a assets "$STAGE/usr/share/weft/assets"
# Source checkouts on mounted/NTFS filesystems can report 0777; normalize so
# the package looks the same regardless of where it was built.
find "$STAGE/usr/share/weft" -type d -exec chmod 0755 {} +
find "$STAGE/usr/share/weft" -type f -exec chmod 0644 {} +
install -m 0644 deploy/weft.env.example "$STAGE/usr/share/weft/weft.env.example"

install -m 0640 config/production.yaml "$STAGE/etc/weft/production.yaml"
install -m 0640 config/scheduler.yaml "$STAGE/etc/weft/scheduler.yaml"

install -m 0644 packaging/systemd/weft.service "$STAGE/lib/systemd/system/weft.service"
install -m 0644 packaging/systemd/weft-scheduler.service "$STAGE/lib/systemd/system/weft-scheduler.service"
install -m 0644 packaging/systemd/weft-backup.service "$STAGE/lib/systemd/system/weft-backup.service"
install -m 0644 packaging/systemd/weft-backup.timer "$STAGE/lib/systemd/system/weft-backup.timer"

install -m 0644 README.md "$STAGE/usr/share/doc/weft/README.md"
install -m 0644 deployment.md "$STAGE/usr/share/doc/weft/deployment.md"
install -m 0644 docs/features.md "$STAGE/usr/share/doc/weft/features.md"

SIZE="$(du -sk "$STAGE" | cut -f1)"

cat > "$STAGE/DEBIAN/control" <<EOF
Package: $PKG
Version: $VERSION
Section: web
Priority: optional
Architecture: $ARCH
Maintainer: $MAINTAINER
Installed-Size: $SIZE
Depends: libc6, ca-certificates, adduser, systemd
Recommends: nginx | caddy
Description: AI-native personal publishing site
 Weft is a single-binary Loco application for an AI-assisted personal
 site: Markdown posts, charts, media library and a read-only Agent API.
 All content is stored in SQLite under /var/lib/weft.
EOF

cat > "$STAGE/DEBIAN/conffiles" <<'EOF'
/etc/weft/production.yaml
/etc/weft/scheduler.yaml
EOF

install -m 0755 packaging/debian/postinst "$STAGE/DEBIAN/postinst"
install -m 0755 packaging/debian/prerm "$STAGE/DEBIAN/prerm"
install -m 0755 packaging/debian/postrm "$STAGE/DEBIAN/postrm"

echo "==> 构建 $DEB"
mkdir -p "$OUT"
dpkg-deb --root-owner-group --build "$STAGE" "$DEB" >/dev/null

echo "==> 完成"
ls -lh "$DEB"
sha256sum "$DEB"

ABS_DEB="$DEB"
if [[ "$ABS_DEB" != /* ]]; then
  ABS_DEB="$REPO_DIR/$DEB"
fi
INSTALL_PATH="./$DEB"
if [[ "$DEB" = /* ]]; then
  INSTALL_PATH="$DEB"
fi

cat <<EOF

安装：
  sudo apt install $INSTALL_PATH

安装后：
  1. 编辑 /etc/weft/weft.env，设置 ADMIN_EMAIL、SITE_BASE_URL
  2. 创建管理员：
       sudo -u weft sh -c 'cd /usr/share/weft && set -a && . /etc/weft/weft.env && set +a && weft-cli task admin:create email:you@example.com name:"Admin" password:"change-me-12345"'
  3. 反向代理参考 /usr/share/doc/weft/deployment.md

EOF

if [[ $INSTALL -eq 1 ]]; then
  echo "==> 安装 $DEB"
  if command -v apt-get >/dev/null 2>&1; then
    apt-get install -y "$ABS_DEB"
  else
    dpkg -i "$ABS_DEB"
  fi
fi
