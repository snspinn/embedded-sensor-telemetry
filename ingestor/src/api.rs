use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::store::Store;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub serial_connected: Arc<AtomicBool>,
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    serial_connected: bool,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/frames/latest", get(latest_frame))
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> Json<Health> {
    Json(Health {
        status: "ok",
        serial_connected: state.serial_connected.load(Ordering::Relaxed),
    })
}

async fn latest_frame(State(state): State<AppState>) -> Response {
    match state.store.latest().await {
        Ok(Some(frame)) => Json(frame).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            eprintln!("Failed to load latest frame: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use protocol::{ImuFusion, TelemetryFrame};
    use serde_json::Value;
    use sqlx::PgPool;
    use tower::ServiceExt;

    use super::*;
    use crate::serial::Received;

    fn app(pool: PgPool, connected: bool) -> Router {
        router(AppState {
            store: Store::new(pool),
            serial_connected: Arc::new(AtomicBool::new(connected)),
        })
    }

    async fn get(app: Router, uri: &str) -> (StatusCode, Value) {
        let response = app
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    #[tokio::test]
    async fn health_reports_serial_state() {
        let pool = PgPool::connect_lazy("postgres://unused").unwrap();
        for connected in [true, false] {
            let (status, body) = get(app(pool.clone(), connected), "/health").await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body["serial_connected"], connected);
        }
    }

    #[sqlx::test]
    #[ignore = "needs Postgres at DATABASE_URL"]
    async fn latest_frame_is_404_when_empty_then_newest(pool: PgPool) {
        let (status, _) = get(app(pool.clone(), true), "/frames/latest").await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let store = Store::new(pool.clone());
        let received_at = SystemTime::now();
        for seq in [1, 2] {
            let frame = TelemetryFrame::new(
                seq,
                seq as u64 * 100,
                ImuFusion {
                    roll: 0.1,
                    pitch: 0.2,
                    yaw: 0.3,
                },
            );
            store
                .insert(&Received { received_at, frame })
                .await
                .unwrap();
        }

        let (status, body) = get(app(pool, true), "/frames/latest").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["seq"], 2);
        assert_eq!(body["uptime_ms"], 200);
    }
}
