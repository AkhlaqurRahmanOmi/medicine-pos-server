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

/// The guide's numbers: `limit` defaults to 50 and is capped at 200.
pub const MAX_LIMIT: u32 = 200;
pub const DEFAULT_LIMIT: u32 = 50;

/// Cursor pagination. There is no `page` and no `offset`.
///
/// Ids are UUIDv7, so they already sort by creation time and make a natural cursor — no
/// `OFFSET` scanning, and no page drifting as rows are inserted underneath a client.
#[derive(Debug, Clone, PartialEq, Eq, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct Pagination {
    /// Opaque to the client: the last id it saw. `None` is the first page.
    pub cursor: Option<String>,
    pub limit: u32,
}

#[derive(Debug, Deserialize)]
struct PaginationParams {
    cursor: Option<String>,
    limit: Option<u32>,
    /// Rejected, not ignored — a client still sending `page` is paginating against a
    /// contract that no longer exists, and silence would hide it until the numbers were wrong.
    page: Option<u32>,
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

        if params.page.is_some() {
            return Err(ApiError::from(
                AppError::new(
                    pos_core::error::ErrorCode::UnknownField,
                    "Lists are cursor-paginated: use cursor and limit, not page.",
                )
                .detail_field("unknown_field", "page"),
            ));
        }

        let limit = params.limit.unwrap_or(DEFAULT_LIMIT);

        if limit == 0 {
            return Err(ApiError::from(AppError::bad_request(
                "limit must be greater than 0",
            )));
        }

        if limit > MAX_LIMIT {
            return Err(ApiError::from(
                AppError::bad_request(format!("limit cannot exceed {MAX_LIMIT}"))
                    .detail_field("limit", MAX_LIMIT.to_string()),
            ));
        }

        Ok(Self {
            cursor: params.cursor,
            limit,
        })
    }
}

use pos_core::ids::BranchId;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: UserId,
    pub tenant_id: TenantId,
    pub permissions: Vec<String>,
    pub branch_permissions: BTreeMap<BranchId, Vec<String>>,
}

impl AuthUser {
    #[must_use]
    pub fn new(
        user_id: UserId,
        tenant_id: TenantId,
        permissions: Vec<String>,
        branch_permissions: BTreeMap<BranchId, Vec<String>>,
    ) -> Self {
        Self {
            user_id,
            tenant_id,
            permissions,
            branch_permissions,
        }
    }

    #[must_use]
    pub fn has_permission(&self, slug: &str) -> bool {
        self.permissions.iter().any(|p| p == slug)
    }

    #[must_use]
    pub fn has_permission_at(&self, slug: &str, branch: BranchId) -> bool {
        self.has_permission(slug)
            || self
                .branch_permissions
                .get(&branch)
                .is_some_and(|perms| perms.iter().any(|p| p == slug))
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

#[derive(Debug, Clone)]
pub struct PlatformToken {
    pub user_id: UserId,
}

impl<S> FromRequestParts<S> for PlatformToken
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Self>()
            .cloned()
            .ok_or_else(|| ApiError::from(AppError::forbidden("Platform authorization required")))
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

    fn pagination_app() -> Router {
        Router::new().route(
            "/items",
            get(|p: Result<Pagination, ApiError>| async move {
                match p {
                    Ok(pag) => (
                        StatusCode::OK,
                        format!("cursor: {:?}, limit: {}", pag.cursor, pag.limit),
                    ),
                    Err(err) => (
                        StatusCode::from_u16(err.0.code.http_status()).unwrap(),
                        err.0.user_message,
                    ),
                }
            }),
        )
    }

    #[tokio::test]
    async fn pagination_rejects_oversized_limit() {
        let app = pagination_app();

        // Within the cap, with a cursor: OK.
        let ok = Request::builder()
            .uri("/items?cursor=019a&limit=200")
            .body(Body::empty())
            .unwrap();
        assert_eq!(app.clone().oneshot(ok).await.unwrap().status(), StatusCode::OK);

        // No parameters at all: the default of 50.
        let bare = Request::builder().uri("/items").body(Body::empty()).unwrap();
        assert_eq!(
            app.clone().oneshot(bare).await.unwrap().status(),
            StatusCode::OK
        );

        // Over the cap of 200: rejected, not clamped. A client asking for 500 rows has
        // misunderstood something, and silently giving it 200 hides that.
        let oversized = Request::builder()
            .uri("/items?limit=500")
            .body(Body::empty())
            .unwrap();
        let resp_oversized = app.oneshot(oversized).await.unwrap();
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
