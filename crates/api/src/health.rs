use axum::{extract::Extension, http::StatusCode, response::IntoResponse, Json};
use cache::Cache;
use db::Db;
use serde_json::json;

#[derive(Clone)]
pub struct HealthState {
    pub db: Option<Db>,
    pub cache: Option<Cache>,
}

#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = 200, description = "Service is alive")
    )
)]
pub async fn live() -> impl IntoResponse {
    // Liveness probe: touches no dependencies
    (StatusCode::OK, Json(json!({ "status": "ok" })))
}

#[utoipa::path(
    get,
    path = "/health/ready",
    responses(
        (status = 200, description = "Service is ready"),
        (status = 503, description = "Dependency is unreachable")
    )
)]
pub async fn ready(Extension(state): Extension<HealthState>) -> impl IntoResponse {
    // Readiness probe: verifies db and cache
    let db_ok = match &state.db {
        Some(db) => db.conn().ping().await.is_ok(),
        None => false,
    };

    let cache_ok = state.cache.is_some();

    if db_ok && cache_ok {
        (
            StatusCode::OK,
            Json(json!({
                "status": "ready",
                "database": "ok",
                "cache": "ok"
            })),
        )
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "status": "unhealthy",
                "database": if db_ok { "ok" } else { "down" },
                "cache": if cache_ok { "ok" } else { "down" }
            })),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::get,
        Router,
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_answers_while_database_is_down() {
        let app = Router::new().route("/health", get(live));

        let req = Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn ready_returns_503_when_dependency_down() {
        // State with missing/down database
        let broken_state = HealthState {
            db: None,
            cache: Some(Cache::in_memory("test")),
        };

        let app = Router::new()
            .route("/health/ready", get(ready))
            .layer(Extension(broken_state));

        let req = Request::builder()
            .uri("/health/ready")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
