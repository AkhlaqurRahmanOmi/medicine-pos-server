use axum::{
    extract::Request,
    middleware::Next,
    response::{IntoResponse, Response},
};
use pos_core::error::AppError;
use pos_core::ids::BranchId;
use std::future::Future;
use std::pin::Pin;

use crate::error::ApiError;
use crate::extract::AuthUser;

pub trait BranchResolver: Send + Sync + 'static {
    fn resolve(&self, req: &Request) -> Option<BranchId>;
}

impl BranchResolver for BranchId {
    fn resolve(&self, _req: &Request) -> Option<BranchId> {
        Some(*self)
    }
}

impl<F> BranchResolver for F
where
    F: Fn(&Request) -> Option<BranchId> + Send + Sync + 'static,
{
    fn resolve(&self, req: &Request) -> Option<BranchId> {
        (self)(req)
    }
}

pub fn branch_from_path(req: &Request) -> Option<BranchId> {
    let path = req.uri().path();
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    for i in 0..segments.len() {
        if segments[i] == "branches" && i + 1 < segments.len() {
            if let Ok(id) = segments[i + 1].parse::<BranchId>() {
                return Some(id);
            }
        }
    }
    for segment in &segments {
        if let Ok(id) = segment.parse::<BranchId>() {
            return Some(id);
        }
    }
    req.extensions().get::<BranchId>().copied()
}

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

pub async fn check_permission_at<B: BranchResolver>(
    slug: &'static str,
    resolver: &B,
    req: Request,
    next: Next,
) -> Response {
    let branch = resolver.resolve(&req);
    let user = req.extensions().get::<AuthUser>();

    let has_perm = match (user, branch) {
        (Some(u), Some(b)) => u.has_permission_at(slug, b),
        (Some(u), None) => u.has_permission(slug),
        (None, _) => false,
    };

    if has_perm {
        next.run(req).await
    } else {
        ApiError::from(AppError::forbidden(format!(
            "Forbidden: missing required permission '{slug}'"
        )))
        .into_response()
    }
}

pub fn require_at<B: BranchResolver + Clone>(
    slug: &'static str,
    resolver: B,
) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send>> + Clone + Send + 'static {
    move |req, next| {
        let resolver = resolver.clone();
        Box::pin(async move { check_permission_at(slug, &resolver, req, next).await })
    }
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
            branch_permissions: Default::default(),
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
            branch_permissions: Default::default(),
        };

        let mut req_allowed = Request::builder()
            .uri("/sales")
            .body(Body::empty())
            .unwrap();
        req_allowed.extensions_mut().insert(authorized_user);

        let resp_allowed = app.oneshot(req_allowed).await.unwrap();
        assert_eq!(resp_allowed.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn require_at_distinguishes_two_branches() {
        let branch_1 = BranchId::new_v7();
        let branch_2 = BranchId::new_v7();

        let mut branch_permissions = std::collections::BTreeMap::new();
        branch_permissions.insert(branch_1, vec!["inventory.count".to_string()]);

        let user = AuthUser {
            user_id: UserId::new_v7(),
            tenant_id: TenantId::new_v7(),
            permissions: vec![],
            branch_permissions,
        };

        let app = Router::new()
            .route(
                "/branches/{branch_id}/count",
                get(|| async { StatusCode::OK }),
            )
            .route_layer(middleware::from_fn(require_at(
                "inventory.count",
                branch_from_path,
            )));

        // Request for branch_1 -> 200 OK
        let req_1 = Request::builder()
            .uri(format!("/branches/{branch_1}/count"))
            .body(Body::empty())
            .unwrap();
        let mut req_1 = req_1;
        req_1.extensions_mut().insert(user.clone());

        let resp_1 = app.clone().oneshot(req_1).await.unwrap();
        assert_eq!(resp_1.status(), StatusCode::OK);

        // Request for branch_2 -> 403 FORBIDDEN
        let req_2 = Request::builder()
            .uri(format!("/branches/{branch_2}/count"))
            .body(Body::empty())
            .unwrap();
        let mut req_2 = req_2;
        req_2.extensions_mut().insert(user);

        let resp_2 = app.oneshot(req_2).await.unwrap();
        assert_eq!(resp_2.status(), StatusCode::FORBIDDEN);
    }
}
