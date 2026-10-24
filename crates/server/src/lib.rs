use api::{ApiDoc, HealthState};
use axum::{routing::get, Extension, Json, Router};
use cache::Cache;
use db::Db;
use pos_core::ports::AccessModeSource;
use pos_core::Config;
use std::sync::Arc;
use tenancy::rest::{admin_router, pharmacy_router};
use tenancy::TenancyAccessModeSource;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_scalar::{Scalar, Servable};

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

use pos_core::error::AppError;
use pos_core::ids::{TenantId, UserId};
use pos_core::ports::BoxFuture;

#[derive(Clone)]
pub struct IdentityAuthenticator {
    jwt_secret: String,
}

impl IdentityAuthenticator {
    #[must_use]
    pub fn new(jwt_secret: impl Into<String>) -> Self {
        Self {
            jwt_secret: jwt_secret.into(),
        }
    }
}

impl api::Authenticator for IdentityAuthenticator {
    fn authenticate<'a>(
        &'a self,
        token: &'a str,
    ) -> BoxFuture<'a, Result<api::AuthUser, AppError>> {
        Box::pin(async move {
            let claims = identity::tokens::decode_pharmacy_token(token, &self.jwt_secret)?;
            let user_id = claims
                .sub
                .parse::<UserId>()
                .map_err(|e| AppError::unauthorized(format!("Invalid sub in token: {e}")))?;
            let tenant_id = claims
                .tenant_id
                .parse::<TenantId>()
                .map_err(|e| AppError::unauthorized(format!("Invalid tenant_id in token: {e}")))?;
            Ok(api::AuthUser {
                user_id,
                tenant_id,
                permissions: vec![],
                branch_permissions: Default::default(),
            })
        })
    }
}

pub async fn platform_token_middleware(
    jwt_secret: Arc<String>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let auth_header = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    if let Some(h) = auth_header {
        let t = h.strip_prefix("Bearer ").unwrap_or(h).trim();
        if let Ok(claims) = identity::tokens::decode_platform_token(t, &jwt_secret) {
            if let Ok(user_id) = claims.sub.parse::<UserId>() {
                req.extensions_mut().insert(api::PlatformToken { user_id });
            }
        }
    }

    next.run(req).await
}

pub fn create_router(state: AppState) -> Router {
    let health_state = HealthState {
        db: state.db.clone(),
        cache: state.cache.clone(),
    };

    let access_source: Arc<dyn AccessModeSource> =
        if let (Some(db), Some(cache)) = (&state.db, &state.cache) {
            Arc::new(TenancyAccessModeSource::new(db.clone(), cache.clone()))
        } else {
            Arc::new(tenancy::access::DummyAccessModeSource)
        };

    let authenticator = IdentityAuthenticator::new(state.config.auth.jwt_secret.clone());
    let jwt_secret = Arc::new(state.config.auth.jwt_secret.clone());
    // One source for the secret and the three lifetimes: no handler holds a constant.
    // Module 03 Q1/Q3, answered 2026-10-24.
    let token_settings = identity::TokenSettings::from(&state.config.auth);

    let pharmacy = pharmacy_router()
        .merge(identity::rest::pharmacy_router())
        .merge(catalog::rest::pharmacy_router())
        .layer(api::auth_layer(authenticator));

    let admin = admin_router()
        .merge(identity::rest::platform_router())
        .merge(catalog::rest::admin_router())
        .layer(axum::middleware::from_fn(move |req, next| {
            let secret = Arc::clone(&jwt_secret);
            platform_token_middleware(secret, req, next)
        }));

    // Assemble the document and the routes together. `ApiDoc` is the root — identity, tags
    // and the bearer scheme — and each module's `OpenApiRouter` contributes the paths its
    // `routes!` collected, so what `/docs` renders is the router that is actually mounted.
    let (api_routes, openapi_doc) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .merge(identity::rest::auth_router())
        .nest("/api/v1", pharmacy)
        .nest("/api/v1/admin", admin)
        .layer(Extension(token_settings))
        .split_for_parts();

    // Fill in the failures every operation shares. Derived from the document rather than
    // written per handler, so a module added later is covered without being told.
    let openapi_doc = api::openapi::with_standard_errors(openapi_doc);
    let served_doc = openapi_doc.clone();

    let mut router = Router::new()
        .route("/health", get(api::live))
        .route("/health/ready", get(api::ready))
        .route(
            "/openapi.json",
            get(move || async move { Json(served_doc.clone()) }),
        )
        .merge(api_routes)
        // Scalar renders the document at `/docs`, inlined into the page rather than fetched.
        .merge(Scalar::with_url("/docs", openapi_doc))
        .layer(Extension(access_source))
        .layer(Extension(health_state))
        .layer(tower_http::trace::TraceLayer::new_for_http());

    if let Some(db) = state.db {
        router = router.layer(Extension(db));
    }
    if let Some(cache) = state.cache {
        router = router.layer(Extension(cache));
    }

    router
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

    fn test_app() -> Router {
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
        create_router(AppState::new(
            config,
            None,
            Some(Cache::in_memory("test")),
        ))
    }

    async fn body_text(app: Router, uri: &str) -> (StatusCode, String) {
        let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read body");
        (status, String::from_utf8(bytes.to_vec()).expect("utf-8 body"))
    }

    #[tokio::test]
    async fn scalar_is_served_at_docs_with_the_document_inlined() {
        let (status, html) = body_text(test_app(), "/docs").await;

        assert_eq!(status, StatusCode::OK, "Scalar must answer at /docs");
        assert!(
            html.contains("id=\"api-reference\""),
            "/docs must be the Scalar page, not an arbitrary 200"
        );
        // The spec is inlined into the page, so the renderer needs no second request to
        // /openapi.json — and a path only reaches it by being registered with `routes!`.
        assert!(
            html.contains("\"openapi\"") && html.contains("/api/v1/tenant"),
            "the document must be embedded in the page Scalar renders"
        );
    }

    #[tokio::test]
    async fn the_document_carries_every_mounted_module_path() {
        let (status, json) = body_text(test_app(), "/openapi.json").await;
        assert_eq!(status, StatusCode::OK);

        let doc: serde_json::Value = serde_json::from_str(&json).expect("valid openapi json");
        let paths = doc["paths"].as_object().expect("paths object");

        // Nesting composes the prefix, so these are the paths as actually mounted.
        for expected in [
            "/health",
            "/health/ready",
            "/api/v1/tenant",
            "/api/v1/tenant/settings",
            "/api/v1/tenant/closure",
            "/api/v1/branches",
            "/api/v1/branches/{id}/deactivate",
            "/api/v1/plans",
            "/api/v1/subscription-payments",
            "/api/v1/admin/tenants/{id}/approve",
            "/api/v1/admin/tenants/{id}/suspend",
            "/api/v1/admin/tenants/{id}/reinstate",
            "/api/v1/admin/tenants/{id}/subscriptions",
            "/api/v1/admin/subscription-payments",
            "/api/v1/admin/subscription-payments/{id}/confirm",
            "/api/v1/admin/subscription-payments/{id}/reject",
        ] {
            assert!(
                paths.contains_key(expected),
                "{expected} is mounted but missing from the document"
            );
        }
    }

    #[tokio::test]
    async fn catalog_paths_are_in_the_openapi_document() {
        let (status, json) = body_text(test_app(), "/openapi.json").await;
        assert_eq!(status, StatusCode::OK);

        let doc: serde_json::Value = serde_json::from_str(&json).expect("valid openapi json");
        let paths = doc["paths"].as_object().expect("paths object");

        for expected in [
            "/api/v1/catalog/search",
            "/api/v1/catalog/products",
            "/api/v1/catalog/products/{id}",
            "/api/v1/catalog/generics/{id}",
            "/api/v1/catalog/generics/{id}/monograph",
            "/api/v1/catalog/companies",
            "/api/v1/catalog/lookups",
            "/api/v1/catalog/submissions",
            "/api/v1/catalog/issues",
            "/api/v1/admin/catalog/queue",
            "/api/v1/admin/catalog/issues",
            "/api/v1/admin/catalog/{kind}/{id}/approve",
            "/api/v1/admin/catalog/{kind}/{id}/reject",
            "/api/v1/admin/catalog/{kind}/{id}/merge",
            "/api/v1/admin/catalog/generics/{id}/monograph",
            "/api/v1/admin/catalog/issues/{id}/resolve",
            "/api/v1/admin/catalog/issues/{id}/dismiss",
        ] {
            assert!(
                paths.contains_key(expected),
                "{expected} is mounted but missing from the document"
            );
        }
    }

    #[tokio::test]
    async fn the_document_offers_a_bearer_token_and_names_its_tags() {
        let (_, json) = body_text(test_app(), "/openapi.json").await;
        let doc: serde_json::Value = serde_json::from_str(&json).expect("valid openapi json");

        assert_eq!(
            doc["components"]["securitySchemes"]["bearer"]["scheme"], "bearer",
            "without the scheme Scalar renders no token field, and every tenant route needs one"
        );
        assert_eq!(
            doc["paths"]["/api/v1/tenant"]["get"]["security"][0]["bearer"],
            serde_json::json!([]),
            "a tenant-scoped route must declare that it needs the token"
        );
        assert_eq!(
            doc["paths"]["/api/v1/admin/tenants/{id}/approve"]["post"]["tags"][0],
            "platform",
            "platform-only routes are tagged apart from the pharmacy surface"
        );
    }

    /// The point of deriving the error responses is that nobody has to remember them. This
    /// asserts it for **every** operation the document holds, so a module added later that
    /// somehow escapes the pass fails here rather than shipping a document that under-promises.
    #[tokio::test]
    async fn every_secured_operation_documents_how_it_can_fail() {
        let (_, json) = body_text(test_app(), "/openapi.json").await;
        let doc: serde_json::Value = serde_json::from_str(&json).expect("valid openapi json");

        let mut checked = 0;
        for (path, item) in doc["paths"].as_object().expect("paths") {
            for (method, op) in item.as_object().expect("path item") {
                let Some(responses) = op["responses"].as_object() else {
                    continue;
                };
                let where_ = format!("{} {path}", method.to_uppercase());

                if op["security"].is_null() {
                    // `/health` must answer while everything else is down, and claiming it
                    // can 401 would be a lie.
                    if path.starts_with("/health") {
                        assert!(
                            !responses.contains_key("401"),
                            "{where_} needs no token, so it cannot return 401"
                        );
                    }
                    continue;
                }

                for status in ["401", "403", "429", "500"] {
                    assert!(
                        responses.contains_key(status),
                        "{where_} is secured but does not document {status}"
                    );
                }
                if op.get("requestBody").is_some() {
                    assert!(
                        responses.contains_key("400"),
                        "{where_} takes a body but does not document 400"
                    );
                }
                if path.contains('{') {
                    assert!(
                        responses.contains_key("404"),
                        "{where_} has a path parameter but does not document 404"
                    );
                }

                // One shape at every status (CONTRACT.md §2).
                for status in ["401", "403", "500"] {
                    assert_eq!(
                        responses[status]["content"]["application/json"]["schema"]["$ref"],
                        "#/components/schemas/ErrorBody",
                        "{where_} {status} must be the one error shape"
                    );
                }
                checked += 1;
            }
        }
        assert!(checked >= 16, "expected the module routes, checked {checked}");
    }

    #[tokio::test]
    async fn a_handwritten_response_is_not_overwritten() {
        let (_, json) = body_text(test_app(), "/openapi.json").await;
        let doc: serde_json::Value = serde_json::from_str(&json).expect("valid openapi json");

        let conflict = &doc["paths"]["/api/v1/branches"]["post"]["responses"]["409"];
        assert!(
            conflict["description"]
                .as_str()
                .expect("409 description")
                .contains("duplicate_branch_name"),
            "the pass fills gaps; it must not flatten an endpoint's own 409 into a generic one"
        );

        let forbidden = &doc["paths"]["/api/v1/tenant/settings"]["patch"]["responses"]["403"];
        assert!(
            forbidden["description"]
                .as_str()
                .expect("403 description")
                .contains("subscription_read_only"),
            "a handler that named its own 403 keeps it"
        );
    }

    #[tokio::test]
    async fn tenancy_routes_are_mounted() {
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

        // 1. Pharmacy router route /api/v1/plans is mounted
        let req_pharmacy = Request::builder()
            .uri("/api/v1/plans")
            .body(Body::empty())
            .unwrap();
        let resp_pharmacy = app.clone().oneshot(req_pharmacy).await.unwrap();
        assert_ne!(
            resp_pharmacy.status(),
            StatusCode::NOT_FOUND,
            "Pharmacy route /api/v1/plans must be mounted"
        );

        // 2. Admin router route /api/v1/admin/subscription-payments is mounted and gated
        let req_admin = Request::builder()
            .uri("/api/v1/admin/subscription-payments")
            .body(Body::empty())
            .unwrap();
        let resp_admin = app.oneshot(req_admin).await.unwrap();
        assert_eq!(
            resp_admin.status(),
            StatusCode::FORBIDDEN,
            "Admin route /api/v1/admin/subscription-payments must be mounted and protected"
        );
    }

    #[tokio::test]
    async fn identity_routes_are_in_the_document() {
        let (status, json) = body_text(test_app(), "/openapi.json").await;
        assert_eq!(status, StatusCode::OK);

        let doc: serde_json::Value = serde_json::from_str(&json).expect("valid openapi json");
        let paths = doc["paths"].as_object().expect("paths object");

        for expected in [
            "/auth/login",
            "/auth/refresh",
            "/auth/logout",
            "/api/v1/users",
            "/api/v1/users/{id}",
            "/api/v1/users/{id}/disable",
            "/api/v1/users/pin",
            "/api/v1/roles",
            "/api/v1/roles/permissions",
            "/api/v1/roles/{id}",
            "/api/v1/roles/{id}/permissions",
            "/api/v1/users/{id}/roles",
            "/api/v1/user-roles/{id}",
            "/api/v1/devices",
            "/api/v1/devices/{id}/revoke",
            "/api/v1/impersonations",
            "/api/v1/admin/platform/auth/step-one",
            "/api/v1/admin/platform/auth/step-two",
            "/api/v1/admin/platform/impersonate/{tenant_id}",
            "/api/v1/admin/platform/impersonate/{id}/enable-writes",
        ] {
            assert!(
                paths.contains_key(expected),
                "{expected} is mounted but missing from the document"
            );
        }
    }

    #[tokio::test]
    async fn a_signed_token_reaches_a_tenancy_route() {
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

        let user_id = pos_core::ids::UserId::new_v7();
        let tenant_id = pos_core::ids::TenantId::new_v7();
        let token = identity::tokens::encode_pharmacy_token(user_id, tenant_id, "secret", 900)
            .expect("encode token");

        // 1. Unauthenticated request to /api/v1/plans returns 401
        let req_unauth = Request::builder()
            .uri("/api/v1/plans")
            .body(Body::empty())
            .unwrap();
        let resp_unauth = app.clone().oneshot(req_unauth).await.unwrap();
        assert_eq!(
            resp_unauth.status(),
            StatusCode::UNAUTHORIZED,
            "Request without token must be refused with 401"
        );

        // 2. Request with real signed token reaches the route (not 401, not 403)
        let req_auth = Request::builder()
            .uri("/api/v1/plans")
            .header("Authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp_auth = app.oneshot(req_auth).await.unwrap();
        assert_ne!(
            resp_auth.status(),
            StatusCode::UNAUTHORIZED,
            "Real signed token must not be rejected with 401"
        );
        assert_ne!(
            resp_auth.status(),
            StatusCode::FORBIDDEN,
            "Real signed token must not be rejected with 403"
        );
    }
}

