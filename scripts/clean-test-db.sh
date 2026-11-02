#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

# Only ever remove the isolated test database. Never use wildcards here:
# a wildcard such as weft_*.sqlite would also match weft_development.sqlite.
for name in weft_test.sqlite weft_test.sqlite-shm weft_test.sqlite-wal; do
  if [[ -e "$name" ]]; then
    echo "removing test artifact: $name"
    rm -f -- "$name"
  fi
done

echo "development database left untouched: weft_development.sqlite"
