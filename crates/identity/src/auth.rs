//! The middleware's body: a bearer token in, an `AuthUser` out.
//!
//! `api` holds the gate (`require`, `require_at`) and the layer, and keeps no dependency on
//! this crate — `crates/server` passes this function in, the way it passes
//! `AccessModeSource` for 02 Tenancy. See `CONTRACT.md` §3, "What this module needs from
//! `00-foundation`".

use api::AuthUser;
use chrono::Utc;
use db::Tx;
use pos_core::context::TenantContext;
use pos_core::error::AppError;
use pos_core::ids::{SessionId, TenantId, UserId};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};

use crate::{queries, tokens};

/// Authenticate a pharmacy access token and resolve what the holder may do.
///
/// The permissions are resolved **inside a tenant transaction**: every table involved is
/// tenant-owned with forced RLS, so a resolution on a bare connection silently returns an
/// empty set — an authenticated request that may do nothing at all.
pub async fn authenticate(
    db: &DatabaseConnection,
    jwt_secret: &str,
    token: &str,
) -> Result<AuthUser, AppError> {
    let claims = tokens::decode_pharmacy_token(token, jwt_secret)?;

    let user_id: UserId = claims
        .sub
        .parse()
        .map_err(|e| AppError::unauthorized(format!("Invalid sub in token: {e}")))?;
    let tenant_id: TenantId = claims
        .tenant_id
        .parse()
        .map_err(|e| AppError::unauthorized(format!("Invalid tenant_id in token: {e}")))?;
    let session_id: SessionId = claims
        .session_id
        .parse()
        .map_err(|e| AppError::unauthorized(format!("Invalid session_id in token: {e}")))?;

    // The branch list is what we are about to resolve, so the context starts without one.
    // Nothing read below is branch-scoped at the policy level.
    let ctx = TenantContext::new(tenant_id, user_id, vec![]);
    let tx = Tx::begin(db, &ctx).await?;
    let txn = tx.transaction()?;

    // Check that the session exists, is not revoked, and has not expired
    let check_sess = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT revoked_at, expires_at FROM sessions WHERE id = $1 AND tenant_id = $2",
        [session_id.as_uuid().into(), tenant_id.as_uuid().into()],
    );
    let sess_row = txn
        .query_one_raw(check_sess)
        .await
        .map_err(|e| AppError::internal("DB error checking session", e.to_string()))?;

    match sess_row {
        Some(row) => {
            let revoked_at: Option<chrono::DateTime<Utc>> = row
                .try_get_by_index(0)
                .map_err(|e| AppError::internal("DB error reading revoked_at", e.to_string()))?;
            let expires_at: chrono::DateTime<Utc> = row
                .try_get_by_index(1)
                .map_err(|e| AppError::internal("DB error reading expires_at", e.to_string()))?;

            if revoked_at.is_some() {
                return Err(AppError::unauthorized("Session has been revoked"));
            }
            if expires_at < Utc::now() {
                return Err(AppError::unauthorized("Session has expired"));
            }
        }
        None => {
            return Err(AppError::unauthorized("Session has been revoked"));
        }
    }

    let access = queries::resolve_access(txn, tenant_id, user_id).await?;
    tx.commit().await?;

    Ok(AuthUser::new(
        user_id,
        tenant_id,
        access.wide,
        access.branch_permissions,
    ))
}

/// Authenticate a platform staff access token.
pub async fn authenticate_platform(
    jwt_secret: &str,
    token: &str,
) -> Result<api::PlatformToken, AppError> {
    let claims = tokens::decode_platform_token(token, jwt_secret)?;
    let user_id = claims
        .sub
        .parse::<UserId>()
        .map_err(|e| AppError::unauthorized(format!("Invalid sub in platform token: {e}")))?;
    Ok(api::PlatformToken { user_id })
}
