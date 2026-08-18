# Telemetry Project — Immersion Scope

## Overview

An end-to-end telemetry system built entirely in Rust. An STM32F3Discovery board reads IMU sensor data and streams it over UART to a companion host (laptop → RPi 4), which ingests, enriches, and stores the data in a locally-accessible database. The project is structured in phases to ensure a working system exists at every stage, with performance optimisation reserved for the final phase.

**Hardware:** STM32F3Discovery (Cortex-M4F, onboard IMU, ST-LINK debugger)  
**Firmware framework:** Embassy (async, `no_std`, no RTOS)  
**Host runtime:** Tokio + Axum  
**Protocol:** Postcard serialisation + COBS framing, shared `protocol` crate  

---

## Learning Goals

- `no_std` embedded Rust and the Cortex-M4F hardware model
- Embassy async executor — cooperative multitasking without an OS
- Designing a typed, versioned binary protocol
- Custom serde serialiser/deserialiser implementation
- Async concurrency patterns: channels, backpressure, fan-out
- End-to-end system design with clean layer boundaries
- Performance profiling and optimisation on both embedded and host sides

---

## Architecture

```
┌─────────────────────────────────┐
│         STM32F3Discovery        │
│                                 │
│  [imu_task] ──channel──▶        │
│                    [uart_task]  │
│                         │       │
└─────────────────────────┼───────┘
                          │ UART (COBS + postcard)
                          ▼
┌─────────────────────────────────┐
│     Ingestor (laptop / RPi4)    │
│                                 │
│  [serial] ──mpsc──▶ [pipeline]  │
│                         │       │
└─────────────────────────┼───────┘
                          │ sqlx
                          ▼
┌─────────────────────────────────┐
│      Storage (Timescale DB)     │
└─────────────────────────────────┘
                          │ 
                          ▼
┌─────────────────────────────────┐
│               API               │
└─────────────────────────---─────┘
```

---

## Repo Layout

```
telemetry-project/
├── Cargo.toml                  # workspace root
├── firmware/                   # no_std, runs on STM32
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs
│       └── tasks/
│           ├── imu.rs          # I2C sensor reading task
│           ├── uart.rs         # UART transmit task
│           └── heartbeat.rs    # LED blink, proves executor is alive
├── protocol/                   # no_std compatible, shared crate
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs              # frame definitions, serde impls, COBS
├── ingestor/                   # std, async, runs on laptop/RPi
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs
│       ├── serial.rs           # reads UART, deserialises frames
│       ├── pipeline.rs         # channel fan-out, backpressure
│       └── store.rs            # writes to DB
└── dashboard/                  # Phase 4+, optional
```


---

## Phases

### Phase 0 — Toolchain & Hello World
**Duration:** 1–2 weeks  
**Goal:** Working embedded Rust environment. Confidence with the toolchain before writing any application logic.

Tasks:
- Install and verify `probe-rs`, `cargo-embed`, `flip-link`
- Work through Rust Embedded Discovery Book chapters 1–5
- Embassy blinky running on STM32F3Discovery
- Heartbeat task proves Embassy executor is alive
- Confirm ST-LINK debugger and RTT logging work

Rust concepts: `no_std`, linker scripts, `.cargo/config.toml`, `probe-rs`, Embassy executor basics

> **Scope guard:** Do not proceed until you can flash, run, and observe RTT (debugger) output.

---

### Phase 1 — IMU Sensor Reading
**Duration:** 2–3 weeks  
**Goal:** Async Embassy tasks reading real sensor data from the onboard IMU over I2C.

Tasks:
- Initialise I2C peripheral via `embassy-stm32`
- Read accelerometer, gyroscope, and magnetometer
- Spawn `imu_task` — reads at 10Hz, sends frames to an internal channel
- Spawn `heartbeat_task` — LED blink confirms executor health
- Log raw readings over STM debugged (RTT) for verification

Rust concepts: `embassy-stm32` I2C, `embassy-time` timers, multi-task spawning, `embassy-sync` channels, `no_std` data structures

> **Scope guard:** Sensor data visible in RTT logs. No UART, no protocol yet.

---

### Phase 2 — Protocol Design & UART Transmission
**Duration:** 2–3 weeks  
**Goal:** Define the shared `protocol` crate and stream encoded telemetry frames over UART.

Tasks:
- Design `TelemetryFrame` schema (version, seq, uptime_ms, ImuReading, FrameMeta)
- Implement postcard-based serde impls for `TelemetryFrame` in the `protocol` crate
- COBS framing for message delimiting over raw UART byte stream
- Spawn `uart_task` — receives from channel, encodes, transmits
- Verify frames on laptop with a minimal decode script

Key design decisions:
- `seq: u32` — monotonic sequence number, primary ordering and drop detection mechanism
- `uptime_ms: u64` — device boot-relative time via `embassy-time`
- `FrameMeta { wall_time_ms: Option<u64> }` — `None` in Phase 1, `Some(ts)` in Phase 3+ without breaking the wire format

Rust concepts: IO encoding (`postcard`/`protocol`), `no_std`, channel backpressure between tasks

> **Scope guard:** Frames decodable on the laptop. Protocol crate has unit tests covering encode/decode round-trips.

---

### Phase 3 — Host Ingestor Service
**Duration:** 3–4 weeks  
**Goal:** A production-quality async Rust service on the laptop that reads UART frames and persists them.

Tasks:
- `serial.rs` — async UART reader, COBS frame accumulation, postcard decode
- Companion-side wall-clock timestamping on frame receipt
- Sequence gap detection and drop logging
- `tokio::mpsc` channel between serial reader and pipeline
- `pipeline.rs` — enrichment, validation, fan-out
- `store.rs` — PostgreSQL via `sqlx` (schema: seq, uptime_ms, wall_ts, accel, gyro, mag)
- Basic `axum` HTTP endpoint: `GET /frames/latest` and `GET /health`

Rust concepts: `tokio`, `axum`, `sqlx`, `mpsc` channels, backpressure, structured error handling with `errorstack`, `chrono`

> **Scope guard:** Full pipeline running. Frames visible in Postgres. HTTP endpoint returns live data.

---

### Phase 4 — End-to-End Integration & Time Sync
**Duration:** 2 weeks  
**Goal:** Harden the full system. Add RTC time sync. Swap laptop for RPi 4/5 as companion.

Tasks:
- Implement clock drift correction algo on ingestor
- Migrate ingestor from laptop to RPi 4/5 — firmware unchanged, validates protocol abstraction
- Error handling audit across all boundaries (UART errors, decode failures, DB errors)
- Reconnection logic in ingestor if UART disconnects

Rust concepts: STM32 RTC peripheral, protocol versioning, resilience patterns, `sqlx` migrations

> **Scope guard:** System runs unattended on RPi 4/5. Frames carry real wall timestamps. No firmware changes required for companion swap.

---

### Phase 5 — Performance & Profiling
**Duration:** 2–3 weeks  
**Goal:** Measure before optimising. Produce concrete before/after numbers.

**Embedded side:**
- Cycle-count the IMU read + encode path using DWT (Data Watchpoint and Trace)
- Profile FPU usage — ensure hardware float is being used, not software fallback
- Audit static memory usage: stack, `.bss`, `.data`
- Tighten Embassy task stack sizes based on measured usage
- Explore sensor fusion (complementary filter for roll/pitch) as an FPU workload

**Host side:**
- Benchmark `postcard` decode throughput vs JSON equivalent
- Profile `tokio` task scheduling under simulated high-frequency input
- Measure and visualise end-to-end latency: sample time → DB write
- Identify and resolve any backpressure bottlenecks in the pipeline

Rust concepts: DWT cycle counting, `#[inline]`, `cargo-flamegraph`, `criterion`, `tokio-console`, allocation auditing

> **Scope guard:** At least one concrete optimisation with measured before/after on both embedded and host sides.

---

## Future Work (Post-Immersion or Option add-ons)

These are intentionally out of scope but represent natural next steps:

1. Add a dashboard (Rust-native TUI)
2. Replace RPi 4/5 with Pico
3. Add a GNSS timesource to RPi
4. Add wireless connectivity like cellular or lora and..
  a. move ingestor & storage migrate storage to cloud,
  b. integrate wireless modules
  c. implement the wireless protocols (COAP for cell, LoRaWAN for lora).

---

## Scope Guards (Global)

- No new sensors beyond the onboard IMU — the project is about mastering Embedded Rust and data streaming, not sensor breadth
- No frontend work during the immersion — `GET /frames/latest` JSON is sufficient
- Phase 5 does not begin until Phase 4 is end-to-end working

---

## Key Crates

| Crate | Layer | Purpose |
|---|---|---|
| `embassy-executor` | Firmware | Async task executor |
| `embassy-stm32` | Firmware | STM32 peripheral HAL |
| `embassy-time` | Firmware | Timers, uptime counter |
| `embassy-sync` | Firmware | Async channels, mutexes |
| `postcard` | Protocol | `no_std` serde binary format |
| `tokio` | Ingestor | Async runtime |
| `axum` | Ingestor | HTTP server |
| `sqlx` | Ingestor | Async DB (SQLite → Postgres) |
| `tokio-serial` | Ingestor | Async UART reader |
| `anyhow` / `thiserror` | Ingestor | Error handling |
| `chrono` | Ingestor | Wall-clock timestamping |
| `criterion` | Phase 5 | Benchmarking |
| `tokio-console` | Phase 5 | Async task profiling |