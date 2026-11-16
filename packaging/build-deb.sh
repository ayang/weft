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
umask 022

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR"

usage() {
  sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
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
HOMEPAGE="${HOMEPAGE:-https://github.com/ayang/weft}"
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
  "$STAGE/usr/lib/systemd/system" \
  "$STAGE/usr/share/weft" \
  "$STAGE/usr/share/doc/weft/examples" \
  "$STAGE/usr/share/lintian/overrides" \
  "$STAGE/usr/share/man/man1" \
  "$STAGE/etc/weft"

install -m 0755 "$BIN" "$STAGE/usr/bin/weft-cli"
# The release profile keeps symbols by default; strip the *packaged copy* only,
# never the build artifact the developer may want to debug.
if command -v strip >/dev/null 2>&1; then
  strip --strip-unneeded "$STAGE/usr/bin/weft-cli" 2>/dev/null || \
    echo "警告：strip 失败，保留未剥离的二进制" >&2
fi
install -m 0755 deploy/weft-backup.sh "$STAGE/usr/lib/weft/weft-backup"

cp -a themes "$STAGE/usr/share/weft/themes"
cp -a assets "$STAGE/usr/share/weft/assets"
install -m 0644 deploy/weft.env.example "$STAGE/usr/share/weft/weft.env.example"

# Source checkouts on mounted/NTFS filesystems can report 0775/0777; normalize
# so the package looks the same regardless of where it was built.
find "$STAGE/usr/share" -type d -exec chmod 0755 {} +
find "$STAGE/usr/share" -type f -exec chmod 0644 {} +

install -m 0644 config/production.yaml "$STAGE/etc/weft/production.yaml"
install -m 0644 config/scheduler.yaml "$STAGE/etc/weft/scheduler.yaml"

install -m 0644 packaging/systemd/weft.service "$STAGE/usr/lib/systemd/system/weft.service"
install -m 0644 packaging/systemd/weft-scheduler.service "$STAGE/usr/lib/systemd/system/weft-scheduler.service"
install -m 0644 packaging/systemd/weft-backup.service "$STAGE/usr/lib/systemd/system/weft-backup.service"
install -m 0644 packaging/systemd/weft-backup.timer "$STAGE/usr/lib/systemd/system/weft-backup.timer"

install -m 0644 README.md "$STAGE/usr/share/doc/weft/README.md"
install -m 0644 deployment.md "$STAGE/usr/share/doc/weft/deployment.md"
install -m 0644 docs/features.md "$STAGE/usr/share/doc/weft/features.md"
install -m 0644 deploy/nginx.conf.example "$STAGE/usr/share/doc/weft/examples/nginx.conf.example"
install -m 0644 packaging/debian/copyright "$STAGE/usr/share/doc/weft/copyright"
install -m 0644 packaging/debian/weft.lintian-overrides "$STAGE/usr/share/lintian/overrides/weft"

# A native package needs a changelog whose version matches the control file.
{
  echo "$PKG ($VERSION) unstable; urgency=medium"
  echo
  echo "  * Package built from the local source tree."
  echo
  echo " -- $MAINTAINER  $(date -R)"
} | gzip -9n > "$STAGE/usr/share/doc/weft/changelog.gz"

install -m 0644 packaging/debian/weft-cli.1 "$STAGE/usr/share/man/man1/weft-cli.1"
gzip -9n "$STAGE/usr/share/man/man1/weft-cli.1"

# Be honest about the C library the binary was linked against: building on a
# newer distro makes the package unusable on older ones with a clear dependency
# error instead of a runtime "GLIBC_2.xx not found".
GLIBC_MIN=""
if command -v readelf >/dev/null 2>&1; then
  GLIBC_MIN="$(readelf -V "$STAGE/usr/bin/weft-cli" 2>/dev/null \
    | grep -o 'GLIBC_[0-9.]*' | sed 's/^GLIBC_//' | sort -Vu | tail -n 1)"
fi
LIBC_DEP="libc6"
if [[ -n "$GLIBC_MIN" ]]; then
  LIBC_DEP="libc6 (>= $GLIBC_MIN)"
fi

SIZE="$(du -sk "$STAGE" | cut -f1)"

cat > "$STAGE/DEBIAN/control" <<EOF
Package: $PKG
Version: $VERSION
Section: web
Priority: optional
Architecture: $ARCH
Maintainer: $MAINTAINER
Homepage: $HOMEPAGE
Installed-Size: $SIZE
Depends: $LIBC_DEP, ca-certificates, systemd
Recommends: nginx | caddy
Description: AI-native personal publishing site
 Weft is a single-binary Loco application for an AI-assisted personal
 site: Markdown posts, charts, media library and a read-only Agent API.
 All content is stored in SQLite under /var/lib/weft. The package
 installs and starts the web, scheduler and backup units; configuring
 an nginx reverse proxy in front of 127.0.0.1:5150 is left to the
 administrator.
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

安装后服务会自动启用并启动：
  weft.service            127.0.0.1:5150
  weft-scheduler.service  图表刷新 / 索引补偿 / 缓存清理
  weft-backup.timer       每日数据库 + 媒体备份

验证：
  systemctl status weft.service
  curl -I http://127.0.0.1:5150/

然后：
  1. 用 nginx 把域名反代到 127.0.0.1:5150（示例见
     /usr/share/doc/weft/examples/nginx.conf.example）
  2. 编辑 /etc/weft/weft.env：至少把 ADMIN_EMAIL 和 SITE_BASE_URL
     改成真实值，然后 systemctl restart weft.service weft-scheduler.service
  3. 创建管理员（邮箱必须与 ADMIN_EMAIL 一致）：
       sudo -u weft sh -c 'cd /usr/share/weft && set -a && . /etc/weft/weft.env && WEFT_APP_DIR=/usr/share/weft && set +a && weft-cli task admin:create email:admin@example.com name:"Admin" password:"change-me-12345"'

完整说明见 /usr/share/doc/weft/deployment.md。

EOF

if [[ $INSTALL -eq 1 ]]; then
  echo "==> 安装 $DEB"
  if [[ ${EUID} -ne 0 ]]; then
    echo "错误：--install 需要 root（请用 sudo 运行）。" >&2
    exit 1
  fi
  if command -v apt-get >/dev/null 2>&1; then
    apt-get install -y "$ABS_DEB"
  else
    dpkg -i "$ABS_DEB"
  fi
fi
