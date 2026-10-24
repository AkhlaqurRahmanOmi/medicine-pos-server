pub mod auth;
pub mod envelope;
pub mod error;
pub mod extract;
pub mod health;
pub mod middleware;
pub mod openapi;
pub mod permission;

pub use auth::{auth_layer, auth_middleware, Authenticator};
pub use envelope::{ApiCreated, ApiOk, ErrorBody, ErrorDetail, ListPage};
pub use error::{ApiError, TRACE_ID_HEADER};
pub use extract::{AuthUser, Pagination, PlatformToken, TenantTx, DEFAULT_LIMIT, MAX_LIMIT};
pub use health::{live, ready, HealthState};
pub use middleware::{
    access_gate_middleware, require_platform_middleware, request_id_middleware, REQUEST_ID_HEADER,
};
pub use openapi::ApiDoc;
pub use permission::{branch_from_path, require, require_at, BranchResolver};
