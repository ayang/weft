#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

DB_URL="${DATABASE_URL:-sqlite://weft_development.sqlite?mode=rwc}"
DB_PATH="${DB_URL#sqlite://}"
DB_PATH="${DB_PATH%%\?*}"
if [[ "$DB_PATH" != /* ]]; then
  DB_PATH="$(pwd)/$DB_PATH"
fi

if [[ ! -s "$DB_PATH" ]]; then
  cat >&2 <<MSG
Refusing to start: development database is missing or empty:
  $DB_PATH

This prevents silently creating a fresh database after an accidental delete.
If this is the first setup, run:
  cargo loco db migrate
  cargo loco task admin:create email:... name:... password:...

To restore a backup:
  cp backups/<file>.sqlite "$DB_PATH"
  rm -f "${DB_PATH}-wal" "${DB_PATH}-shm"
MSG
  exit 1
fi

echo "Backing up $DB_PATH before start..."
cargo loco task db:backup

exec cargo loco start "$@"
