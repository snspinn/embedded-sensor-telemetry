use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{FromRow, PgPool};

use crate::serial::Received;

#[derive(Debug, Serialize, FromRow)]
pub struct StoredFrame {
    pub seq: i64,
    pub uptime_ms: i64,
    pub received_at: DateTime<Utc>,
    pub roll: f32,
    pub pitch: f32,
    pub yaw: f32,
}

#[derive(Clone)]
pub struct Store {
    pool: PgPool,
}

impl Store {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn connect(url: &str) -> Result<Self, sqlx::Error> {
        let pool = PgPool::connect(url).await?;
        sqlx::migrate!().run(&pool).await?;
        Ok(Self::new(pool))
    }

    pub async fn latest(&self) -> Result<Option<StoredFrame>, sqlx::Error> {
        // Frames decoded from one serial read share `received_at`, so `seq` breaks the tie.
        sqlx::query_as(
            "SELECT seq, uptime_ms, received_at, roll, pitch, yaw FROM telemetry_frames
             ORDER BY received_at DESC, seq DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn insert(&self, received: &Received) -> Result<(), sqlx::Error> {
        let frame = &received.frame;
        sqlx::query(
            "INSERT INTO telemetry_frames (seq, uptime_ms, received_at, roll, pitch, yaw)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(i64::from(frame.seq.0))
        .bind(frame.uptime_ms as i64)
        .bind(DateTime::<Utc>::from(received.received_at))
        .bind(frame.imu.roll)
        .bind(frame.imu.pitch)
        .bind(frame.imu.yaw)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use protocol::{ImuFusion, TelemetryFrame};

    use super::*;

    #[sqlx::test]
    #[ignore = "needs Postgres at DATABASE_URL"]
    async fn inserts_frame(pool: PgPool) {
        let store = Store { pool: pool.clone() };
        let received = Received {
            received_at: SystemTime::now(),
            frame: TelemetryFrame::new(
                7,
                700,
                ImuFusion {
                    roll: 0.1,
                    pitch: 0.2,
                    yaw: 0.3,
                },
            ),
        };

        store.insert(&received).await.expect("insert");

        let (seq, uptime_ms, yaw): (i64, i64, f32) =
            sqlx::query_as("SELECT seq, uptime_ms, yaw FROM telemetry_frames")
                .fetch_one(&pool)
                .await
                .expect("select");
        assert_eq!((seq, uptime_ms, yaw), (7, 700, 0.3));
    }
}
