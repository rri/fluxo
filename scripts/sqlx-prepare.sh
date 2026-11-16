#!/bin/sh
# Refresh the offline query metadata in .sqlx/ after changing SQL or migrations.
# Builds use that metadata (SQLX_OFFLINE=true in .cargo/config.toml), so no database is needed
# to compile. CI runs `cargo sqlx prepare --workspace --check` to catch stale metadata.
set -eu
cd "$(dirname "$0")/.."
mkdir -p target
export DATABASE_URL="sqlite://target/sqlx-dev.db"
rm -f target/sqlx-dev.db
sqlx database create
sqlx migrate run --source migrations
SQLX_OFFLINE=false cargo sqlx prepare --workspace -- --all-targets
