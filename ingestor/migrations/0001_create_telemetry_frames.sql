CREATE TABLE telemetry_frames (
    seq         BIGINT      NOT NULL,
    uptime_ms   BIGINT      NOT NULL,
    received_at TIMESTAMPTZ NOT NULL,
    roll        REAL        NOT NULL,
    pitch       REAL        NOT NULL,
    yaw         REAL        NOT NULL
);

CREATE INDEX telemetry_frames_received_at_idx ON telemetry_frames (received_at DESC);
