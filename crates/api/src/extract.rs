use axum::{
    extract::{FromRequestParts, Query},
    http::request::Parts,
};
use db::{Db, Tx};
use pos_core::context::TenantContext;
use pos_core::error::AppError;
use pos_core::ids::{TenantId, UserId};
use serde::Deserialize;

use crate::error::ApiError;

pub const MAX_PAGE_SIZE: u32 = 100;
pub const DEFAULT_PAGE_SIZE: u32 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pagination {
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Deserialize)]
struct PaginationParams {
    page: Option<u32>,
    page_size: Option<u32>,
}

impl<S> FromRequestParts<S> for Pagination
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Query(params) = Query::<PaginationParams>::from_request_parts(parts, state)
            .await
            .map_err(|e| {
                ApiError::from(AppError::bad_request(format!(
                    "Invalid pagination query parameters: {e}"
                )))
            })?;

        let page = params.page.unwrap_or(1);
        let page_size = params.page_size.unwrap_or(DEFAULT_PAGE_SIZE);

        if page == 0 {
            return Err(ApiError::from(AppError::bad_request(
                "page must be greater than 0",
            )));
        }

        if page_size > MAX_PAGE_SIZE {
            return Err(ApiError::from(AppError::bad_request(format!(
                "page_size cannot exceed maximum allowed limit of {MAX_PAGE_SIZE}"
            ))));
        }

        Ok(Self { page, page_size })
    }
}

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: UserId,
    pub tenant_id: TenantId,
    pub permissions: Vec<String>,
}

impl AuthUser {
    #[must_use]
    pub fn has_permission(&self, slug: &str) -> bool {
        self.permissions.iter().any(|p| p == slug)
    }
}

impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Self>()
            .cloned()
            .ok_or_else(|| ApiError::from(AppError::unauthorized("Authentication required")))
    }
}

pub struct TenantTx(pub Tx);

impl<S> FromRequestParts<S> for TenantTx
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let ctx = parts
            .extensions
            .get::<TenantContext>()
            .cloned()
            .ok_or_else(|| ApiError::from(AppError::unauthorized("Tenant context required")))?;

        let db = parts.extensions.get::<Db>().ok_or_else(|| {
            ApiError::from(AppError::internal(
                "Database connection missing in extensions",
                "Db not in extensions",
            ))
        })?;

        let tx = Tx::begin(db.conn(), &ctx).await.map_err(ApiError::from)?;
        Ok(Self(tx))
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
    use pos_core::ids::BranchId;
    use sea_orm::{DbBackend, MockDatabase, MockExecResult};
    use tower::ServiceExt;

    #[tokio::test]
    async fn pagination_rejects_oversized_page() {
        let app = Router::new().route(
            "/items",
            get(|p: Result<Pagination, ApiError>| async move {
                match p {
                    Ok(pag) => (
                        StatusCode::OK,
                        format!("page: {}, size: {}", pag.page, pag.page_size),
                    ),
                    Err(err) => (StatusCode::BAD_REQUEST, err.0.user_message),
                }
            }),
        );

        // 1. Within limit: OK
        let req_valid = Request::builder()
            .uri("/items?page=1&page_size=50")
            .body(Body::empty())
            .unwrap();
        let resp_valid = app.clone().oneshot(req_valid).await.unwrap();
        assert_eq!(resp_valid.status(), StatusCode::OK);

        // 2. Over cap (>100): Bad Request
        let req_oversized = Request::builder()
            .uri("/items?page=1&page_size=500")
            .body(Body::empty())
            .unwrap();
        let resp_oversized = app.oneshot(req_oversized).await.unwrap();
        assert_eq!(resp_oversized.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn tenant_tx_extractor_opens_and_commits() {
        let tenant_id = TenantId::new_v7();
        let user_id = UserId::new_v7();
        let ctx = TenantContext::new(tenant_id, user_id, vec![BranchId::new_v7()]);

        let mock_db = MockDatabase::new(DbBackend::Postgres)
            .append_exec_results([MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();

        let db = Db::from_connection(mock_db, "postgres://pos_app@localhost/db");

        let app = Router::new()
            .route(
                "/tx-action",
                get(|TenantTx(tx): TenantTx| async move {
                    tx.commit().await.unwrap();
                    StatusCode::OK
                }),
            )
            .layer(axum::Extension(ctx))
            .layer(axum::Extension(db));

        let req = Request::builder()
            .uri("/tx-action")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
