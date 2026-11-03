#!/usr/bin/env bash
# Consistent SQLite backup + media archive, driven by weft-backup.timer.
# Manual use:
#   sudo -u weft /usr/local/bin/weft-backup
set -euo pipefail

WEFT_CLI="${WEFT_CLI:-/usr/local/bin/weft-cli}"
if [[ ! -x "$WEFT_CLI" ]]; then
  WEFT_CLI="$(command -v weft-cli || true)"
fi
if [[ -z "$WEFT_CLI" || ! -x "$WEFT_CLI" ]]; then
  echo "weft-backup: weft-cli not found (tried /usr/local/bin/weft-cli and PATH)" >&2
  exit 1
fi

ENV_FILE="${ENV_FILE:-/etc/weft/weft.env}"
if [[ -f "$ENV_FILE" ]]; then
  set -a
  # shellcheck disable=SC1090
  . "$ENV_FILE"
  set +a
fi

BACKUP_DIR="${WEFT_BACKUP_DIR:-/var/backups/weft}"
KEEP_DAYS="${WEFT_BACKUP_KEEP_DAYS:-14}"
MEDIA_DIR="${MEDIA_DIR:-/var/lib/weft/media/uploads}"
STAMP="$(date -u +%Y%m%d-%H%M%S)"

mkdir -p "$BACKUP_DIR"

# VACUUM INTO is consistent while the database is live in WAL mode.
"$WEFT_CLI" task db:backup "path:$BACKUP_DIR/weft-$STAMP.sqlite"

if [[ -d "$MEDIA_DIR" ]]; then
  tar -C "$MEDIA_DIR" -czf "$BACKUP_DIR/media-$STAMP.tar.gz" .
fi

# Retention. Uploads are only in the media archive, so do not delete those too
# aggressively; 14 days is a starting point, not a policy.
find "$BACKUP_DIR" -maxdepth 1 -type f -mtime "+$KEEP_DAYS" -delete

echo "backup done: $BACKUP_DIR (stamp $STAMP)"
