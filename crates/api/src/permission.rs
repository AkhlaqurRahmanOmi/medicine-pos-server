use axum::{
    extract::Request,
    middleware::Next,
    response::{IntoResponse, Response},
};
use pos_core::error::AppError;
use std::future::Future;
use std::pin::Pin;

use crate::error::ApiError;
use crate::extract::AuthUser;

pub async fn check_permission(slug: &'static str, req: Request, next: Next) -> Response {
    let has_perm = req
        .extensions()
        .get::<AuthUser>()
        .is_some_and(|u| u.has_permission(slug));

    if has_perm {
        next.run(req).await
    } else {
        ApiError::from(AppError::forbidden(format!(
            "Forbidden: missing required permission '{slug}'"
        )))
        .into_response()
    }
}

pub fn require(
    slug: &'static str,
) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send>> + Clone + Send + 'static
{
    move |req, next| Box::pin(check_permission(slug, req, next))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        middleware,
        routing::get,
        Router,
    };
    use pos_core::ids::{TenantId, UserId};
    use tower::ServiceExt;

    #[tokio::test]
    async fn require_returns_forbidden_without_permission() {
        let app = Router::new()
            .route("/sales", get(|| async { StatusCode::OK }))
            .route_layer(middleware::from_fn(require("sales.create")));

        // 1. User without required permission: 403 Forbidden
        let unauthorized_user = AuthUser {
            user_id: UserId::new_v7(),
            tenant_id: TenantId::new_v7(),
            permissions: vec!["inventory.read".into()],
        };

        let mut req_forbidden = Request::builder()
            .uri("/sales")
            .body(Body::empty())
            .unwrap();
        req_forbidden.extensions_mut().insert(unauthorized_user);

        let resp_forbidden = app.clone().oneshot(req_forbidden).await.unwrap();
        assert_eq!(resp_forbidden.status(), StatusCode::FORBIDDEN);

        // 2. User with required permission: 200 OK
        let authorized_user = AuthUser {
            user_id: UserId::new_v7(),
            tenant_id: TenantId::new_v7(),
            permissions: vec!["sales.create".into()],
        };

        let mut req_allowed = Request::builder()
            .uri("/sales")
            .body(Body::empty())
            .unwrap();
        req_allowed.extensions_mut().insert(authorized_user);

        let resp_allowed = app.oneshot(req_allowed).await.unwrap();
        assert_eq!(resp_allowed.status(), StatusCode::OK);
    }
}
