use api::{ApiDoc, HealthState};
use axum::{routing::get, Json, Router};
use cache::Cache;
use db::Db;
use pos_core::Config;
use utoipa::OpenApi;

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub db: Option<Db>,
    pub cache: Option<Cache>,
}

impl AppState {
    #[must_use]
    pub fn new(config: Config, db: Option<Db>, cache: Option<Cache>) -> Self {
        Self { config, db, cache }
    }
}

pub fn create_router(state: AppState) -> Router {
    let health_state = HealthState {
        db: state.db,
        cache: state.cache,
    };

    let openapi_doc = ApiDoc::openapi();

    Router::new()
        .route("/health", get(api::live))
        .route("/health/ready", get(api::ready))
        .route(
            "/openapi.json",
            get(move || async move { Json(openapi_doc.clone()) }),
        )
        .layer(axum::Extension(health_state))
        .layer(tower_http::trace::TraceLayer::new_for_http())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_route_is_mounted() {
        let figment = pos_core::Config::figment()
            .merge((
                "database.url",
                "postgres://pos_owner:devpass@localhost/medicine_pos",
            ))
            .merge((
                "database.app_url",
                "postgres://pos_app:devpass@localhost/medicine_pos",
            ))
            .merge(("server.port", 8080))
            .merge(("server.env", "test"))
            .merge(("redis.url", "redis://localhost:6379"))
            .merge(("auth.jwt_secret", "secret"));

        let config = pos_core::Config::from_figment(&figment).expect("parse test config");
        let state = AppState::new(config, None, Some(Cache::in_memory("test")));
        let app = create_router(state);

        let req = Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req_doc = Request::builder()
            .uri("/openapi.json")
            .body(Body::empty())
            .unwrap();

        let resp_doc = app.oneshot(req_doc).await.unwrap();
        assert_eq!(resp_doc.status(), StatusCode::OK);
    }
}
