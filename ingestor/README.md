# Telemetry Ingestor

Telemtery data stream ingestor.

## Usage

```sh
task db:up
export DATABASE_URL=postgres://telemetry:telemetry@localhost:5433/telemetry
cargo run -p ingestor -- --port /dev/ttyACM0
```

`task db:up` starts TimescaleDB from `compose.yaml` on port 5433. The ingestor applies the
migrations in `migrations/` on startup and writes each frame to `telemetry_frames`.

Without `--port`, the ingestor uses the first serial port whose USB VID/PID matches the firmware
(`1209:0001`). It prints each decoded frame prefixed with its receive time (Unix ms), logs and
skips frames that fail to decode, logs gaps and resets in `seq`, and reconnects if the board is
unplugged.

## Database on the Raspberry Pi

`compose.yaml` is for laptop development only. On the Pi, TimescaleDB runs natively (PostgreSQL 17
from PGDG, TimescaleDB from Timescale's apt repo) on port `5432`. Its data directory lives on a
dedicated SSD partition, so a full database cannot fill the root filesystem.

1. Create an ext4 partition on the SSD for the database and mount it at `/mnt/pgdata` through
   `/etc/fstab`, for example:

   ```sh
   sudo mkfs.ext4 -L pgdata /dev/nvme0n1p3   # the new, empty partition
   sudo mkdir -p /mnt/pgdata
   echo 'LABEL=pgdata /mnt/pgdata ext4 defaults,noatime 0 2' | sudo tee -a /etc/fstab
   sudo systemctl daemon-reload && sudo mount /mnt/pgdata
   ```

2. Run the install script:

   ```sh
   sudo scripts/pi/install-timescaledb.sh
   ```

   It refuses to continue unless `/mnt/pgdata` is a separate, fstab-mounted filesystem. It then
   creates cluster `17/main` in `/mnt/pgdata/postgresql/17/main`, runs `timescaledb-tune`, creates
   the `telemetry` role and database with a random password, enables the `timescaledb` extension, and
   checks that the Timescale background workers are running. Re-running it is safe and keeps the
   existing password.

   Optional environment variables: `TELEMETRY_DB_PASSWORD` to choose the password, `LAN_CIDR` (e.g.
   `192.168.1.0/24`) to accept `telemetry` logins from the LAN, `PG_DATA_MOUNT` to use another mount
   point.

3. The connection string is written to `/etc/telemetry/database.env` (root-only):

   ```sh
   set -a; . <(sudo cat /etc/telemetry/database.env); set +a
   cargo run --release -p ingestor
   ```

## HTTP API

Served on `127.0.0.1:3000` by default (`--listen` to change).

- `GET /health`: `200` with `{"status":"ok","serial_connected":true|false}` while the process is up.
- `GET /frames/latest`: the newest stored frame as JSON, or `404` when none are stored.

```sh
curl localhost:3000/frames/latest
```

## Tests

- `cargo test -p ingestor`: unit tests, no hardware or database.
- `task test:db`: store tests against the compose database.
- `task test:e2e`: all ignored tests; needs the database and a board on its application USB port.

## Serial access on Linux

The firmware enumerates as a USB CDC-ACM device, which the kernel exposes as `/dev/ttyACM*`
(group `dialout`). Add yourself to that group, then log out and back in:

```sh
sudo usermod -aG dialout "$USER"
```

If `/dev/ttyACM*` is missing while `lsusb -d 1209:0001` shows the board, something detached the
kernel `cdc_acm` driver (e.g. a libusb tool). Replug the board to restore it.
