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
