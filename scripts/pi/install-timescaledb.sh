#!/usr/bin/env bash
# Native TimescaleDB on the Raspberry Pi 5. Safe to re-run. Usage: sudo ./install-timescaledb.sh
set -euo pipefail

PG_VERSION="${PG_VERSION:-17}"
PG_PORT=5432
PG_DATA_MOUNT="${PG_DATA_MOUNT:-/mnt/pgdata}"
PG_DATA_IMAGE="${PG_DATA_IMAGE:-/var/lib/pgdata.img}"
PG_DATA_SIZE="${PG_DATA_SIZE:-8G}"
ENV_FILE="${ENV_FILE:-/etc/telemetry/database.env}"
LAN_CIDR="${LAN_CIDR:-}"
TS_SOURCE_REF="${TS_SOURCE_REF:-}"

TS_REPO="https://packagecloud.io/timescale/timescaledb/debian"
CLUSTER_DIR="$PG_DATA_MOUNT/postgresql/$PG_VERSION/main"
CONF_DIR="/etc/postgresql/$PG_VERSION/main"
PG_BIN="/usr/lib/postgresql/$PG_VERSION/bin"

# ── Colours ──────────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'
info()    { echo -e "${GREEN}[INFO]${NC}  $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error()   { echo -e "${RED}[ERROR]${NC} $*" >&2; exit 1; }

psql_su() { runuser -u postgres -- psql -X -q -v ON_ERROR_STOP=1 -p "$PG_PORT" "$@"; }

# ── Preflight ────────────────────────────────────────────────────────────────
[[ $EUID -eq 0 ]] || error "Run as root: sudo $0"
[[ "$(dpkg --print-architecture)" == arm64 ]] || error "Expected arm64 (64-bit Raspberry Pi OS)."
# shellcheck source=/dev/null
. /etc/os-release
CODENAME="$VERSION_CODENAME"
info "Debian $VERSION_ID ($CODENAME), PostgreSQL $PG_VERSION"

if ! mountpoint -q "$PG_DATA_MOUNT"; then
    FSTAB_SRC=$(findmnt -s -n -o SOURCE "$PG_DATA_MOUNT" || true)
    [[ -z "$FSTAB_SRC" || "$FSTAB_SRC" == "$PG_DATA_IMAGE" ]] \
        || error "/etc/fstab mounts $FSTAB_SRC at $PG_DATA_MOUNT, but it is not mounted. Fix or remove that entry and re-run."
    if [[ ! -e "$PG_DATA_IMAGE" ]]; then
        info "Creating $PG_DATA_SIZE data filesystem image $PG_DATA_IMAGE"
        fallocate -l "$PG_DATA_SIZE" "$PG_DATA_IMAGE"
        chmod 600 "$PG_DATA_IMAGE"
        mkfs.ext4 -q -L pgdata "$PG_DATA_IMAGE"
    fi
    install -d "$PG_DATA_MOUNT"
    [[ -n "$FSTAB_SRC" ]] || echo "$PG_DATA_IMAGE $PG_DATA_MOUNT ext4 loop,defaults,noatime 0 2" >> /etc/fstab
    systemctl daemon-reload
    mount "$PG_DATA_MOUNT"
fi
DATA_SRC=$(findmnt -n -o SOURCE --target "$PG_DATA_MOUNT")
ROOT_SRC=$(findmnt -n -o SOURCE --target /)
[[ "$DATA_SRC" != "$ROOT_SRC" ]] || error "$PG_DATA_MOUNT is on the root filesystem ($ROOT_SRC)."
findmnt -s "$PG_DATA_MOUNT" &>/dev/null || error "$PG_DATA_MOUNT has no /etc/fstab entry, so it would not mount at boot."
info "Data filesystem: $DATA_SRC on $PG_DATA_MOUNT"

# ── PostgreSQL (PGDG) ────────────────────────────────────────────────────────
apt-get update
apt-get install -y postgresql-common ca-certificates curl gnupg openssl
sed -i -E 's/^#?[[:space:]]*create_main_cluster[[:space:]]*=.*/create_main_cluster = false/' \
    /etc/postgresql-common/createcluster.conf
grep -qx 'create_main_cluster = false' /etc/postgresql-common/createcluster.conf \
    || echo 'create_main_cluster = false' >> /etc/postgresql-common/createcluster.conf
[[ -f /etc/apt/sources.list.d/pgdg.list || -f /etc/apt/sources.list.d/pgdg.sources ]] \
    || /usr/share/postgresql-common/pgdg/apt.postgresql.org.sh -y

# ── TimescaleDB ──────────────────────────────────────────────────────────────
ts_has_arm64() {
    local index
    index=$(curl -fsSL "$TS_REPO/dists/$CODENAME/main/binary-arm64/Packages" 2>/dev/null) || return 1
    grep -Fx "Package: timescaledb-2-postgresql-$PG_VERSION" <<<"$index" >/dev/null
}

install_from_packages() {
    info "Timescale's apt repo has arm64 packages for $CODENAME, installing them."
    curl -fsSL "$TS_REPO/gpgkey" | gpg --dearmor --yes -o /usr/share/keyrings/timescaledb.gpg
    echo "deb [signed-by=/usr/share/keyrings/timescaledb.gpg] $TS_REPO/ $CODENAME main" \
        > /etc/apt/sources.list.d/timescaledb.list
    apt-get update
    apt-get install -y "timescaledb-2-postgresql-$PG_VERSION" timescaledb-tools "postgresql-client-$PG_VERSION"
}

install_from_source() {
    warn "No arm64 timescaledb-2-postgresql-$PG_VERSION for $CODENAME in Timescale's apt repo, building from source."
    apt-get install -y "postgresql-$PG_VERSION" "postgresql-server-dev-$PG_VERSION" \
        build-essential cmake git libssl-dev libkrb5-dev golang-go
    local ref="$TS_SOURCE_REF" src
    if [[ -z "$ref" ]]; then
        ref=$(git ls-remote --tags --refs https://github.com/timescale/timescaledb \
            | awk -F/ '{print $3}' | grep -E '^[0-9]+\.[0-9]+\.[0-9]+$' | sort -V | tail -n1)
    fi
    info "Building TimescaleDB $ref"
    src=$(mktemp -d)
    git clone --depth 1 --branch "$ref" https://github.com/timescale/timescaledb "$src"
    (cd "$src" \
        && ./bootstrap -DREGRESS_CHECKS=OFF -DTAP_CHECKS=OFF -DWARNINGS_AS_ERRORS=OFF -DPG_CONFIG="$PG_BIN/pg_config" \
        && make -C build -j"$(nproc)" install)
    rm -rf "$src"
    GOBIN=/usr/local/bin go install github.com/timescale/timescaledb-tune/cmd/timescaledb-tune@latest
}

if ts_has_arm64; then install_from_packages; else install_from_source; fi

# ── Cluster on the data filesystem ───────────────────────────────────────────
EXISTING_DIR=$(pg_lsclusters -h | awk -v v="$PG_VERSION" '$1 == v && $2 == "main" {print $6}')
if [[ -z "$EXISTING_DIR" ]]; then
    install -d -o postgres -g postgres -m 700 "$PG_DATA_MOUNT/postgresql" "$PG_DATA_MOUNT/postgresql/$PG_VERSION"
    pg_createcluster "$PG_VERSION" main -d "$CLUSTER_DIR" -p "$PG_PORT"
elif [[ "$EXISTING_DIR" != "$CLUSTER_DIR" ]]; then
    error "Cluster $PG_VERSION/main already exists at $EXISTING_DIR. Move it to $CLUSTER_DIR or drop it (pg_dropcluster --stop $PG_VERSION main) and re-run."
fi
CLUSTER_PORT=$(pg_lsclusters -h | awk -v v="$PG_VERSION" '$1 == v && $2 == "main" {print $3}')
[[ "$CLUSTER_PORT" == "$PG_PORT" ]] || error "Cluster $PG_VERSION/main is on port $CLUSTER_PORT, expected $PG_PORT. Another cluster holds $PG_PORT (see pg_lsclusters)."

install -d /etc/systemd/system/postgresql@.service.d
printf '[Unit]\nRequiresMountsFor=%s\n' "$PG_DATA_MOUNT" > /etc/systemd/system/postgresql@.service.d/data-mount.conf
systemctl daemon-reload

# ── Tuning and network ───────────────────────────────────────────────────────
timescaledb-tune --yes --quiet --pg-config "$PG_BIN/pg_config" --conf-path "$CONF_DIR/postgresql.conf"

if [[ -n "$LAN_CIDR" ]]; then
    pg_conftool "$PG_VERSION" main set listen_addresses '*'
    HBA_LINE="host    telemetry       telemetry       $LAN_CIDR       scram-sha-256"
    grep -qxF "$HBA_LINE" "$CONF_DIR/pg_hba.conf" || echo "$HBA_LINE" >> "$CONF_DIR/pg_hba.conf"
    info "Accepting telemetry logins from $LAN_CIDR"
fi

systemctl enable "postgresql@$PG_VERSION-main"
systemctl restart "postgresql@$PG_VERSION-main"

# ── Role, database and extension ─────────────────────────────────────────────
PASSWORD="${TELEMETRY_DB_PASSWORD:-}"
if [[ -z "$PASSWORD" && -f "$ENV_FILE" ]]; then
    PASSWORD=$(sed -n 's|^DATABASE_URL=postgres://telemetry:\([^@]*\)@.*|\1|p' "$ENV_FILE")
fi
[[ -n "$PASSWORD" ]] || PASSWORD=$(openssl rand -hex 24)
[[ "$PASSWORD" != telemetry ]] || error "Use a non-default password for the telemetry role."
[[ "$PASSWORD" =~ ^[A-Za-z0-9._~-]+$ ]] || error "TELEMETRY_DB_PASSWORD may only contain A-Z a-z 0-9 . _ ~ - (it goes into a URL)."

psql_su -d postgres -v pw="$PASSWORD" <<'SQL'
SELECT 'CREATE ROLE telemetry' WHERE NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'telemetry')\gexec
ALTER ROLE telemetry WITH LOGIN PASSWORD :'pw';
SELECT 'CREATE DATABASE telemetry OWNER telemetry' WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname = 'telemetry')\gexec
SQL
psql_su -d telemetry -c 'CREATE EXTENSION IF NOT EXISTS timescaledb'

DATABASE_URL="postgres://telemetry:$PASSWORD@localhost:$PG_PORT/telemetry"
install -d -m 755 "$(dirname "$ENV_FILE")"
(umask 077 && echo "DATABASE_URL=$DATABASE_URL" > "$ENV_FILE")

# ── Verify ───────────────────────────────────────────────────────────────────
DATA_DIR=$(psql_su -d telemetry -Atc 'SHOW data_directory')
[[ "$DATA_DIR" == "$CLUSTER_DIR" ]] || error "data_directory is $DATA_DIR, expected $CLUSTER_DIR"

TS_VERSION=$(psql_su -d telemetry -Atc "SELECT extversion FROM pg_extension WHERE extname = 'timescaledb'")
[[ -n "$TS_VERSION" ]] || error "timescaledb extension is missing from the telemetry database."

WORKERS=0
for _ in $(seq 10); do
    WORKERS=$(psql_su -d telemetry -Atc "SELECT count(*) FROM pg_stat_activity WHERE backend_type LIKE 'TimescaleDB%'")
    (( WORKERS > 0 )) && break
    sleep 1
done
(( WORKERS > 0 )) || error "No TimescaleDB background workers running. Check shared_preload_libraries in $CONF_DIR/postgresql.conf."

psql "$DATABASE_URL" -X -Atc 'SELECT 1' >/dev/null || error "telemetry role cannot log in over localhost."

echo ""
info "TimescaleDB $TS_VERSION running on port $PG_PORT with $WORKERS background worker(s)."
info "Data directory: $DATA_DIR ($DATA_SRC)"
info "DATABASE_URL written to $ENV_FILE (root-only). Load it with: set -a; . <(sudo cat $ENV_FILE); set +a"
