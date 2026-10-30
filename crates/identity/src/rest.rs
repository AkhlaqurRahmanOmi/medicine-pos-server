use api::{ApiCreated, ApiError, ApiOk, ErrorBody, TenantTx};
use axum::extract::{Extension, Json, Path};
use cache::Cache;
use db::Db;
use pos_core::context::TenantContext;
use pos_core::ids::{DeviceId, RoleId, SessionId, TenantId, UserId};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::types::{
    AssignRoleRequest, ChangePasswordRequest, CreatePlatformUserRequest, CreateRoleRequest,
    CreateUserRequest, Device, ImpersonationSessionItem, LoginRequest, LoginResponse, LogoutRequest,
    Me, OpenImpersonationRequest, PasswordResetConfirmRequest, PasswordResetRequest, Permission,
    PlatformLoginResponse, PlatformLoginStepOneRequest, PlatformLoginStepOneResponse,
    PlatformLoginStepTwoRequest, PlatformRole, PlatformUser, RefreshTokenRequest,
    RefreshTokenResponse, RegisterDeviceRequest, Role, Session, SetPinRequest,
    SetRolePermissionsRequest, UpdateRoleRequest, UpdateUserRequest, User,
};
use crate::settings::TokenSettings;
use crate::{commands, login, platform, queries};

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateImpersonationSessionBody {
    #[serde(alias = "tenant_id")]
    pub target_tenant: Option<Uuid>,
    pub reason: String,
}

// -----------------------------------------------------------------------------
// Auth Routes (Public)
// -----------------------------------------------------------------------------

#[utoipa::path(
    post, path = "/auth/login", tag = "auth",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Login successful", body = LoginResponse),
        (status = 401, description = "Invalid credentials", body = ErrorBody),
        (status = 429, description = "Rate limited", body = ErrorBody),
    )
)]
pub async fn login_handler(
    Extension(db): Extension<Db>,
    Extension(cache): Extension<Cache>,
    Extension(settings): Extension<TokenSettings>,
    Json(body): Json<LoginRequest>,
) -> Result<ApiOk<LoginResponse>, ApiError> {
    let res = login::login(db.conn(), &cache, body, &settings, None, None)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(res))
}

#[utoipa::path(
    post, path = "/auth/refresh", tag = "auth",
    request_body = RefreshTokenRequest,
    responses(
        (status = 200, description = "Token refreshed", body = RefreshTokenResponse),
        (status = 401, description = "Invalid or revoked refresh token", body = ErrorBody),
    )
)]
pub async fn refresh_handler(
    Extension(db): Extension<Db>,
    Extension(cache): Extension<Cache>,
    Extension(settings): Extension<TokenSettings>,
    Json(body): Json<RefreshTokenRequest>,
) -> Result<ApiOk<RefreshTokenResponse>, ApiError> {
    let res = login::refresh_token(db.conn(), &cache, body, &settings)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(res))
}

#[utoipa::path(
    post, path = "/auth/logout", tag = "auth",
    request_body = LogoutRequest,
    responses(
        (status = 200, description = "Logout successful", body = serde_json::Value),
    )
)]
pub async fn logout_handler(
    Extension(db): Extension<Db>,
    Extension(cache): Extension<Cache>,
    Json(body): Json<LogoutRequest>,
) -> Result<ApiOk<serde_json::Value>, ApiError> {
    login::logout(db.conn(), &cache, body)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(serde_json::json!({ "status": "logged_out" })))
}

#[utoipa::path(
    post, path = "/auth/password-reset", tag = "auth",
    request_body = PasswordResetRequest,
    responses(
        (status = 202, description = "Password reset accepted", body = serde_json::Value),
        (status = 429, description = "Rate limited", body = ErrorBody),
    )
)]
pub async fn request_password_reset_handler(
    Extension(db): Extension<Db>,
    Extension(cache): Extension<Cache>,
    Json(body): Json<PasswordResetRequest>,
) -> Result<ApiOk<serde_json::Value>, ApiError> {
    let _ = login::request_password_reset(db.conn(), &cache, &body.email)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(serde_json::json!({ "status": "accepted" })))
}

#[utoipa::path(
    post, path = "/auth/password-reset/confirm", tag = "auth",
    request_body = PasswordResetConfirmRequest,
    responses(
        (status = 200, description = "Password reset successful", body = serde_json::Value),
        (status = 400, description = "Invalid or expired reset token", body = ErrorBody),
    )
)]
pub async fn confirm_password_reset_handler(
    Extension(db): Extension<Db>,
    Json(body): Json<PasswordResetConfirmRequest>,
) -> Result<ApiOk<serde_json::Value>, ApiError> {
    login::confirm_password_reset(db.conn(), body)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(serde_json::json!({ "status": "confirmed" })))
}

// -----------------------------------------------------------------------------
// Current User (/me)
// -----------------------------------------------------------------------------

#[utoipa::path(
    get, path = "/me", tag = "identity", security(("bearer" = [])),
    responses(
        (status = 200, description = "Current user profile, permissions, and branch list", body = Me),
        (status = 404, description = "User not found", body = ErrorBody),
    )
)]
pub async fn get_me_handler(
    Extension(ctx): Extension<TenantContext>,
    Extension(db): Extension<Db>,
) -> Result<ApiOk<Me>, ApiError> {
    let tx = db::Tx::begin(db.conn(), &ctx)
        .await
        .map_err(ApiError::from)?;
    let me = queries::get_me(tx.transaction().map_err(ApiError::from)?, ctx.tenant_id, ctx.user_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::from(pos_core::error::AppError::not_found("User not found")))?;
    Ok(ApiOk(me))
}

#[utoipa::path(
    post, path = "/me/password", tag = "identity", security(("bearer" = [])),
    request_body = ChangePasswordRequest,
    responses(
        (status = 200, description = "Password changed successfully", body = serde_json::Value),
        (status = 401, description = "Current password does not match", body = ErrorBody),
    )
)]
pub async fn change_password_handler(
    tx: TenantTx,
    Extension(ctx): Extension<TenantContext>,
    Json(body): Json<ChangePasswordRequest>,
) -> Result<ApiOk<serde_json::Value>, ApiError> {
    commands::change_password(&tx.0, ctx.user_id, body.current_password.as_deref(), &body.new_password)
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(serde_json::json!({ "status": "password_changed" })))
}

#[utoipa::path(
    post, path = "/me/pin", tag = "identity", security(("bearer" = [])),
    request_body = SetPinRequest,
    responses(
        (status = 200, description = "PIN configured", body = serde_json::Value),
    )
)]
pub async fn set_my_pin_handler(
    tx: TenantTx,
    Extension(ctx): Extension<TenantContext>,
    Json(body): Json<SetPinRequest>,
) -> Result<ApiOk<serde_json::Value>, ApiError> {
    commands::set_user_pin(&tx.0, ctx.user_id, &body.pin)
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(serde_json::json!({ "status": "pin_set" })))
}

#[utoipa::path(
    post, path = "/users/pin", tag = "identity", security(("bearer" = [])),
    request_body = SetPinRequest,
    responses(
        (status = 200, description = "PIN configured", body = serde_json::Value),
    )
)]
pub async fn set_pin_handler(
    tx: TenantTx,
    Extension(ctx): Extension<TenantContext>,
    Json(body): Json<SetPinRequest>,
) -> Result<ApiOk<serde_json::Value>, ApiError> {
    set_my_pin_handler(tx, Extension(ctx), Json(body)).await
}

// -----------------------------------------------------------------------------
// User Management
// -----------------------------------------------------------------------------

#[utoipa::path(
    get, path = "/users", tag = "identity", security(("bearer" = [])),
    responses(
        (status = 200, description = "List staff users", body = Vec<User>),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn list_users_handler(
    Extension(ctx): Extension<TenantContext>,
    Extension(db): Extension<Db>,
) -> Result<ApiOk<Vec<User>>, ApiError> {
    let tx = db::Tx::begin(db.conn(), &ctx)
        .await
        .map_err(ApiError::from)?;
    let users = queries::list_users(tx.transaction().map_err(ApiError::from)?, ctx.tenant_id)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(users))
}

#[utoipa::path(
    post, path = "/users", tag = "identity", security(("bearer" = [])),
    request_body = CreateUserRequest,
    responses(
        (status = 201, description = "User created", body = serde_json::Value),
        (status = 409, description = "Email already registered", body = ErrorBody),
    )
)]
pub async fn create_user_handler(
    tx: TenantTx,
    Json(body): Json<CreateUserRequest>,
) -> Result<ApiCreated<serde_json::Value>, ApiError> {
    let user_id = commands::create_user(
        &tx.0,
        &body.name,
        &body.email,
        body.phone.as_deref(),
        body.password.as_deref(),
    )
    .await
    .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiCreated(serde_json::json!({ "id": user_id })))
}

#[utoipa::path(
    get, path = "/users/{id}", tag = "identity", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "User id")),
    responses(
        (status = 200, description = "User profile", body = User),
        (status = 404, description = "User not found", body = ErrorBody),
    )
)]
pub async fn get_user_handler(
    Extension(ctx): Extension<TenantContext>,
    Extension(db): Extension<Db>,
    Path(id): Path<Uuid>,
) -> Result<ApiOk<User>, ApiError> {
    let tx = db::Tx::begin(db.conn(), &ctx)
        .await
        .map_err(ApiError::from)?;
    let user = queries::get_user(tx.transaction().map_err(ApiError::from)?, ctx.tenant_id, UserId::from(id))
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::from(pos_core::error::AppError::not_found("User not found")))?;
    Ok(ApiOk(user))
}

#[utoipa::path(
    patch, path = "/users/{id}", tag = "identity", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "User id")),
    request_body = UpdateUserRequest,
    responses(
        (status = 200, description = "User updated", body = serde_json::Value),
    )
)]
pub async fn update_user_handler(
    tx: TenantTx,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateUserRequest>,
) -> Result<ApiOk<serde_json::Value>, ApiError> {
    commands::update_user(&tx.0, UserId::from(id), body.name.as_deref(), body.phone.as_deref())
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(serde_json::json!({ "status": "updated" })))
}

#[utoipa::path(
    post, path = "/users/{id}/disable", tag = "identity", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "User id")),
    responses(
        (status = 200, description = "User disabled and sessions revoked", body = serde_json::Value),
        (status = 409, description = "Cannot disable last active owner", body = ErrorBody),
    )
)]
pub async fn disable_user_handler(
    tx: TenantTx,
    Extension(cache): Extension<Cache>,
    Path(id): Path<Uuid>,
) -> Result<ApiOk<serde_json::Value>, ApiError> {
    commands::disable_user(&tx.0, &cache, UserId::from(id))
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(serde_json::json!({ "status": "disabled" })))
}

#[utoipa::path(
    post, path = "/users/{id}/reset-password", tag = "identity", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "User id")),
    responses(
        (status = 200, description = "Password reset with temporary password", body = serde_json::Value),
        (status = 404, description = "User not found", body = ErrorBody),
    )
)]
pub async fn reset_user_password_handler(
    tx: TenantTx,
    Path(id): Path<Uuid>,
) -> Result<ApiOk<serde_json::Value>, ApiError> {
    let temp_pwd = commands::reset_user_password(&tx.0, UserId::from(id))
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(serde_json::json!({ "temporary_password": temp_pwd })))
}

// -----------------------------------------------------------------------------
// Role Management
// -----------------------------------------------------------------------------

#[utoipa::path(
    get, path = "/roles", tag = "identity", security(("bearer" = [])),
    responses(
        (status = 200, description = "List tenant roles", body = Vec<Role>),
    )
)]
pub async fn list_roles_handler(
    Extension(ctx): Extension<TenantContext>,
    Extension(db): Extension<Db>,
) -> Result<ApiOk<Vec<Role>>, ApiError> {
    let tx = db::Tx::begin(db.conn(), &ctx)
        .await
        .map_err(ApiError::from)?;
    let roles = queries::list_roles(tx.transaction().map_err(ApiError::from)?, ctx.tenant_id)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(roles))
}

#[utoipa::path(
    get, path = "/roles/permissions", tag = "identity", security(("bearer" = [])),
    responses(
        (status = 200, description = "List assignable permissions", body = Vec<Permission>),
    )
)]
pub async fn list_permissions_handler(
    Extension(db): Extension<Db>,
) -> Result<ApiOk<Vec<Permission>>, ApiError> {
    let perms = queries::assignable_permissions(db.conn())
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(perms))
}

#[utoipa::path(
    get, path = "/permissions", tag = "identity", security(("bearer" = [])),
    responses(
        (status = 200, description = "List assignable permissions", body = Vec<Permission>),
    )
)]
pub async fn list_permissions_contract_handler(
    Extension(db): Extension<Db>,
) -> Result<ApiOk<Vec<Permission>>, ApiError> {
    list_permissions_handler(Extension(db)).await
}

#[utoipa::path(
    post, path = "/roles", tag = "identity", security(("bearer" = [])),
    request_body = CreateRoleRequest,
    responses(
        (status = 201, description = "Role created", body = serde_json::Value),
        (status = 403, description = "Non-assignable permission rejected", body = ErrorBody),
    )
)]
pub async fn create_role_handler(
    tx: TenantTx,
    Json(body): Json<CreateRoleRequest>,
) -> Result<ApiCreated<serde_json::Value>, ApiError> {
    let role_id = commands::create_role(&tx.0, &body.name, &body.permissions)
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiCreated(serde_json::json!({ "id": role_id })))
}

#[utoipa::path(
    get, path = "/roles/{id}", tag = "identity", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Role id")),
    responses(
        (status = 200, description = "Role details", body = Role),
        (status = 404, description = "Role not found", body = ErrorBody),
    )
)]
pub async fn get_role_handler(
    Extension(ctx): Extension<TenantContext>,
    Extension(db): Extension<Db>,
    Path(id): Path<Uuid>,
) -> Result<ApiOk<Role>, ApiError> {
    let tx = db::Tx::begin(db.conn(), &ctx)
        .await
        .map_err(ApiError::from)?;
    let role = queries::get_role(tx.transaction().map_err(ApiError::from)?, ctx.tenant_id, RoleId::from(id))
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::from(pos_core::error::AppError::not_found("Role not found")))?;
    Ok(ApiOk(role))
}

#[utoipa::path(
    patch, path = "/roles/{id}", tag = "identity", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Role id")),
    request_body = UpdateRoleRequest,
    responses(
        (status = 200, description = "Role updated", body = serde_json::Value),
        (status = 409, description = "Cannot rename system role", body = ErrorBody),
    )
)]
pub async fn update_role_handler(
    tx: TenantTx,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateRoleRequest>,
) -> Result<ApiOk<serde_json::Value>, ApiError> {
    commands::update_role(&tx.0, RoleId::from(id), &body.name)
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(serde_json::json!({ "status": "updated" })))
}

#[utoipa::path(
    delete, path = "/roles/{id}", tag = "identity", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Role id")),
    responses(
        (status = 200, description = "Role deleted", body = serde_json::Value),
        (status = 409, description = "Cannot delete system role", body = ErrorBody),
    )
)]
pub async fn delete_role_handler(
    tx: TenantTx,
    Path(id): Path<Uuid>,
) -> Result<ApiOk<serde_json::Value>, ApiError> {
    commands::delete_role(&tx.0, RoleId::from(id))
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(serde_json::json!({ "status": "deleted" })))
}

#[utoipa::path(
    put, path = "/roles/{id}/permissions", tag = "identity", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Role id")),
    request_body = SetRolePermissionsRequest,
    responses(
        (status = 200, description = "Permissions updated", body = serde_json::Value),
        (status = 409, description = "Cannot modify system role permissions", body = ErrorBody),
    )
)]
pub async fn set_role_permissions_handler(
    tx: TenantTx,
    Extension(cache): Extension<Cache>,
    Path(id): Path<Uuid>,
    Json(body): Json<SetRolePermissionsRequest>,
) -> Result<ApiOk<serde_json::Value>, ApiError> {
    commands::set_role_permissions(&tx.0, &cache, RoleId::from(id), &body.permissions)
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(serde_json::json!({ "status": "permissions_set" })))
}

#[utoipa::path(
    post, path = "/roles/{id}/permissions", tag = "identity", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Role id")),
    request_body = SetRolePermissionsRequest,
    responses(
        (status = 200, description = "Permissions updated", body = serde_json::Value),
        (status = 409, description = "Cannot modify system role permissions", body = ErrorBody),
    )
)]
pub async fn set_role_permissions_post_handler(
    tx: TenantTx,
    Extension(cache): Extension<Cache>,
    Path(id): Path<Uuid>,
    Json(body): Json<SetRolePermissionsRequest>,
) -> Result<ApiOk<serde_json::Value>, ApiError> {
    set_role_permissions_handler(tx, Extension(cache), Path(id), Json(body)).await
}

#[utoipa::path(
    post, path = "/users/{id}/roles", tag = "identity", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "User id")),
    request_body = AssignRoleRequest,
    responses(
        (status = 201, description = "Role granted", body = serde_json::Value),
    )
)]
pub async fn assign_role_handler(
    tx: TenantTx,
    Extension(cache): Extension<Cache>,
    Path(id): Path<Uuid>,
    Json(body): Json<AssignRoleRequest>,
) -> Result<ApiCreated<serde_json::Value>, ApiError> {
    let grant_id = commands::grant_role(&tx.0, &cache, UserId::from(id), body.role_id, body.branch_id)
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiCreated(serde_json::json!({ "grant_id": grant_id })))
}

#[utoipa::path(
    delete, path = "/user-roles/{id}", tag = "identity", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Grant id")),
    responses(
        (status = 200, description = "Role grant revoked", body = serde_json::Value),
        (status = 409, description = "Cannot strip last active owner", body = ErrorBody),
    )
)]
pub async fn revoke_role_handler(
    tx: TenantTx,
    Extension(cache): Extension<Cache>,
    Path(id): Path<Uuid>,
) -> Result<ApiOk<serde_json::Value>, ApiError> {
    commands::revoke_grant(&tx.0, &cache, id)
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(serde_json::json!({ "status": "revoked" })))
}

#[utoipa::path(
    delete, path = "/users/{id}/roles/{grant_id}", tag = "identity", security(("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "User id"),
        ("grant_id" = Uuid, Path, description = "Role grant id"),
    ),
    responses(
        (status = 200, description = "Role grant revoked", body = serde_json::Value),
        (status = 409, description = "Cannot strip last active owner", body = ErrorBody),
    )
)]
pub async fn revoke_user_role_handler(
    tx: TenantTx,
    Extension(cache): Extension<Cache>,
    Path((_user_id, grant_id)): Path<(Uuid, Uuid)>,
) -> Result<ApiOk<serde_json::Value>, ApiError> {
    revoke_role_handler(tx, Extension(cache), Path(grant_id)).await
}

// -----------------------------------------------------------------------------
// Device Management
// -----------------------------------------------------------------------------

#[utoipa::path(
    get, path = "/devices", tag = "identity", security(("bearer" = [])),
    responses(
        (status = 200, description = "List registered POS devices", body = Vec<Device>),
    )
)]
pub async fn list_devices_handler(
    Extension(ctx): Extension<TenantContext>,
    Extension(db): Extension<Db>,
) -> Result<ApiOk<Vec<Device>>, ApiError> {
    let tx = db::Tx::begin(db.conn(), &ctx)
        .await
        .map_err(ApiError::from)?;
    let devices = queries::list_devices(tx.transaction().map_err(ApiError::from)?, ctx.tenant_id)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(devices))
}

#[utoipa::path(
    post, path = "/devices", tag = "identity", security(("bearer" = [])),
    request_body = RegisterDeviceRequest,
    responses(
        (status = 201, description = "Device registered", body = serde_json::Value),
        (status = 409, description = "Prefix already in use in this pharmacy", body = ErrorBody),
    )
)]
pub async fn register_device_handler(
    tx: TenantTx,
    Json(body): Json<RegisterDeviceRequest>,
) -> Result<ApiCreated<serde_json::Value>, ApiError> {
    let device_id = commands::register_device(
        &tx.0,
        body.branch_id,
        &body.name,
        &body.invoice_prefix,
        None,
        body.app_version.as_deref(),
    )
    .await
    .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiCreated(serde_json::json!({ "id": device_id })))
}

#[utoipa::path(
    post, path = "/devices/{id}/revoke", tag = "identity", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Device id")),
    responses(
        (status = 200, description = "Device revoked and sessions invalidated", body = serde_json::Value),
    )
)]
pub async fn revoke_device_handler(
    tx: TenantTx,
    Extension(cache): Extension<Cache>,
    Path(id): Path<Uuid>,
) -> Result<ApiOk<serde_json::Value>, ApiError> {
    commands::revoke_device(&tx.0, &cache, DeviceId::from(id))
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(serde_json::json!({ "status": "revoked" })))
}

// -----------------------------------------------------------------------------
// Sessions Management
// -----------------------------------------------------------------------------

#[utoipa::path(
    get, path = "/sessions", tag = "identity", security(("bearer" = [])),
    responses(
        (status = 200, description = "List active sessions", body = Vec<Session>),
    )
)]
pub async fn list_sessions_handler(
    Extension(ctx): Extension<TenantContext>,
    Extension(db): Extension<Db>,
) -> Result<ApiOk<Vec<Session>>, ApiError> {
    let tx = db::Tx::begin(db.conn(), &ctx)
        .await
        .map_err(ApiError::from)?;
    let sessions = queries::list_sessions(tx.transaction().map_err(ApiError::from)?, ctx.tenant_id)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(sessions))
}

#[utoipa::path(
    delete, path = "/sessions/{id}", tag = "identity", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Session id")),
    responses(
        (status = 200, description = "Session revoked", body = serde_json::Value),
        (status = 404, description = "Session not found", body = ErrorBody),
    )
)]
pub async fn revoke_session_handler(
    tx: TenantTx,
    Extension(cache): Extension<Cache>,
    Path(id): Path<Uuid>,
) -> Result<ApiOk<serde_json::Value>, ApiError> {
    commands::revoke_session(&tx.0, &cache, SessionId::from(id))
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(serde_json::json!({ "status": "revoked" })))
}

// -----------------------------------------------------------------------------
// Impersonation (Pharmacy Owner audit view)
// -----------------------------------------------------------------------------

#[utoipa::path(
    get, path = "/impersonations", tag = "identity", security(("bearer" = [])),
    responses(
        (status = 200, description = "Impersonation sessions opened against this pharmacy", body = Vec<ImpersonationSessionItem>),
    )
)]
pub async fn list_impersonations_handler(
    tx: TenantTx,
) -> Result<ApiOk<Vec<ImpersonationSessionItem>>, ApiError> {
    let sessions = platform::list_impersonation_sessions_for_tenant(&tx.0)
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(sessions))
}

#[utoipa::path(
    get, path = "/impersonation-sessions", tag = "identity", security(("bearer" = [])),
    responses(
        (status = 200, description = "Impersonation sessions opened against this pharmacy", body = Vec<ImpersonationSessionItem>),
    )
)]
pub async fn list_impersonation_sessions_handler(
    tx: TenantTx,
) -> Result<ApiOk<Vec<ImpersonationSessionItem>>, ApiError> {
    list_impersonations_handler(tx).await
}

// -----------------------------------------------------------------------------
// Platform Staff Endpoints (Admin)
// -----------------------------------------------------------------------------

#[utoipa::path(
    post, path = "/platform/auth/step-one", tag = "platform",
    request_body = PlatformLoginStepOneRequest,
    responses(
        (status = 200, description = "Step one verified, TOTP required", body = PlatformLoginStepOneResponse),
        (status = 401, description = "Invalid credentials", body = ErrorBody),
        (status = 403, description = "TOTP not configured", body = ErrorBody),
    )
)]
pub async fn platform_step_one_handler(
    Extension(db): Extension<Db>,
    Extension(settings): Extension<TokenSettings>,
    Json(body): Json<PlatformLoginStepOneRequest>,
) -> Result<ApiOk<PlatformLoginStepOneResponse>, ApiError> {
    let res = platform::platform_login_step_one(db.conn(), body, &settings.jwt_secret)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(res))
}

#[utoipa::path(
    post, path = "/auth/login", tag = "platform",
    request_body = PlatformLoginStepOneRequest,
    responses(
        (status = 200, description = "Step one verified, TOTP required", body = PlatformLoginStepOneResponse),
        (status = 401, description = "Invalid credentials", body = ErrorBody),
        (status = 403, description = "TOTP not configured", body = ErrorBody),
    )
)]
pub async fn platform_login_handler(
    Extension(db): Extension<Db>,
    Extension(settings): Extension<TokenSettings>,
    Json(body): Json<PlatformLoginStepOneRequest>,
) -> Result<ApiOk<PlatformLoginStepOneResponse>, ApiError> {
    platform_step_one_handler(Extension(db), Extension(settings), Json(body)).await
}

#[utoipa::path(
    post, path = "/platform/auth/step-two", tag = "platform",
    request_body = PlatformLoginStepTwoRequest,
    responses(
        (status = 200, description = "Step two verified, access token issued", body = PlatformLoginResponse),
        (status = 401, description = "Invalid TOTP code", body = ErrorBody),
    )
)]
pub async fn platform_step_two_handler(
    Extension(db): Extension<Db>,
    Extension(settings): Extension<TokenSettings>,
    Json(body): Json<PlatformLoginStepTwoRequest>,
) -> Result<ApiOk<PlatformLoginResponse>, ApiError> {
    let res = platform::platform_login_step_two(db.conn(), body, &settings)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(res))
}

#[utoipa::path(
    post, path = "/auth/login/totp", tag = "platform",
    request_body = PlatformLoginStepTwoRequest,
    responses(
        (status = 200, description = "Step two verified, access token issued", body = PlatformLoginResponse),
        (status = 401, description = "Invalid TOTP code", body = ErrorBody),
    )
)]
pub async fn platform_login_totp_handler(
    Extension(db): Extension<Db>,
    Extension(settings): Extension<TokenSettings>,
    Json(body): Json<PlatformLoginStepTwoRequest>,
) -> Result<ApiOk<PlatformLoginResponse>, ApiError> {
    platform_step_two_handler(Extension(db), Extension(settings), Json(body)).await
}

#[utoipa::path(
    post, path = "/platform/impersonate/{tenant_id}", tag = "platform", security(("bearer" = [])),
    params(("tenant_id" = Uuid, Path, description = "Pharmacy to impersonate")),
    request_body = OpenImpersonationRequest,
    responses(
        (status = 201, description = "Impersonation session opened", body = serde_json::Value),
        (status = 400, description = "Reason required", body = ErrorBody),
    )
)]
pub async fn open_impersonation_handler(
    Extension(db): Extension<Db>,
    Extension(platform_user): Extension<api::PlatformToken>,
    Extension(settings): Extension<TokenSettings>,
    Path(tenant_id): Path<Uuid>,
    Json(body): Json<OpenImpersonationRequest>,
) -> Result<ApiCreated<serde_json::Value>, ApiError> {
    let session_id = platform::open_impersonation_session(
        db.conn(),
        platform_user.user_id.as_uuid(),
        TenantId::from(tenant_id),
        &body.reason,
        settings.impersonation_ttl_secs,
    )
    .await
    .map_err(ApiError::from)?;
    Ok(ApiCreated(serde_json::json!({ "session_id": session_id })))
}

#[utoipa::path(
    post, path = "/impersonation-sessions", tag = "platform", security(("bearer" = [])),
    request_body = CreateImpersonationSessionBody,
    responses(
        (status = 201, description = "Impersonation session opened", body = serde_json::Value),
        (status = 400, description = "Reason required", body = ErrorBody),
    )
)]
pub async fn create_impersonation_session_handler(
    Extension(db): Extension<Db>,
    Extension(platform_user): Extension<api::PlatformToken>,
    Extension(settings): Extension<TokenSettings>,
    Json(body): Json<CreateImpersonationSessionBody>,
) -> Result<ApiCreated<serde_json::Value>, ApiError> {
    let target_id = body.target_tenant.ok_or_else(|| {
        ApiError::from(pos_core::error::AppError::bad_request("tenant_id required"))
    })?;
    let session_id = platform::open_impersonation_session(
        db.conn(),
        platform_user.user_id.as_uuid(),
        TenantId::from(target_id),
        &body.reason,
        settings.impersonation_ttl_secs,
    )
    .await
    .map_err(ApiError::from)?;
    Ok(ApiCreated(serde_json::json!({ "session_id": session_id })))
}

#[utoipa::path(
    post, path = "/platform/impersonate/{id}/enable-writes", tag = "platform", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Impersonation session id")),
    responses(
        (status = 200, description = "Writes enabled", body = serde_json::Value),
        (status = 404, description = "Active session not found", body = ErrorBody),
    )
)]
pub async fn enable_impersonation_writes_handler(
    Extension(db): Extension<Db>,
    Extension(platform_user): Extension<api::PlatformToken>,
    Path(id): Path<Uuid>,
) -> Result<ApiOk<serde_json::Value>, ApiError> {
    platform::enable_impersonation_writes(db.conn(), id, platform_user.user_id.as_uuid())
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(serde_json::json!({ "writes_enabled": true })))
}

#[utoipa::path(
    post, path = "/impersonation-sessions/{id}/enable-writes", tag = "platform", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Impersonation session id")),
    responses(
        (status = 200, description = "Writes enabled", body = serde_json::Value),
        (status = 404, description = "Active session not found", body = ErrorBody),
    )
)]
pub async fn enable_impersonation_writes_contract_handler(
    Extension(db): Extension<Db>,
    Extension(platform_user): Extension<api::PlatformToken>,
    Path(id): Path<Uuid>,
) -> Result<ApiOk<serde_json::Value>, ApiError> {
    enable_impersonation_writes_handler(Extension(db), Extension(platform_user), Path(id)).await
}

#[utoipa::path(
    post, path = "/impersonation-sessions/{id}/end", tag = "platform", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Impersonation session id")),
    responses(
        (status = 200, description = "Impersonation session ended", body = serde_json::Value),
        (status = 404, description = "Active session not found", body = ErrorBody),
    )
)]
pub async fn end_impersonation_handler(
    Extension(db): Extension<Db>,
    Extension(_platform_user): Extension<api::PlatformToken>,
    Path(id): Path<Uuid>,
) -> Result<ApiOk<serde_json::Value>, ApiError> {
    commands::end_impersonation_session(db.conn(), id)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(serde_json::json!({ "status": "ended" })))
}

#[utoipa::path(
    get, path = "/platform-users", tag = "platform", security(("bearer" = [])),
    responses(
        (status = 200, description = "List platform staff users", body = Vec<PlatformUser>),
    )
)]
pub async fn list_platform_users_handler(
    Extension(db): Extension<Db>,
    Extension(_platform): Extension<api::PlatformToken>,
) -> Result<ApiOk<Vec<PlatformUser>>, ApiError> {
    let users = queries::list_platform_users(db.conn()).await.map_err(ApiError::from)?;
    Ok(ApiOk(users))
}

#[utoipa::path(
    post, path = "/platform-users", tag = "platform", security(("bearer" = [])),
    request_body = CreatePlatformUserRequest,
    responses(
        (status = 201, description = "Platform user created", body = serde_json::Value),
    )
)]
pub async fn create_platform_user_handler(
    Extension(db): Extension<Db>,
    Extension(_platform): Extension<api::PlatformToken>,
    Json(body): Json<CreatePlatformUserRequest>,
) -> Result<ApiCreated<serde_json::Value>, ApiError> {
    let role_str = match body.role {
        PlatformRole::SuperAdmin => "super_admin",
        PlatformRole::Reviewer => "reviewer",
        PlatformRole::Support => "support",
        PlatformRole::Billing => "billing",
    };
    let id = commands::create_platform_user(db.conn(), &body.email, role_str)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiCreated(serde_json::json!({ "id": id })))
}

// -----------------------------------------------------------------------------
// Routers
// -----------------------------------------------------------------------------

pub fn auth_router() -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(login_handler))
        .routes(routes!(refresh_handler))
        .routes(routes!(logout_handler))
        .routes(routes!(request_password_reset_handler))
        .routes(routes!(confirm_password_reset_handler))
}

pub fn pharmacy_router() -> OpenApiRouter {
    let authed_routes = OpenApiRouter::new()
        .routes(routes!(get_me_handler))
        .routes(routes!(change_password_handler))
        .routes(routes!(set_my_pin_handler))
        .routes(routes!(set_pin_handler));

    let user_view_routes = OpenApiRouter::new()
        .routes(routes!(list_users_handler))
        .routes(routes!(get_user_handler))
        .routes(routes!(list_impersonations_handler))
        .routes(routes!(list_impersonation_sessions_handler))
        .layer(axum::middleware::from_fn(api::permission::require(crate::perms::USER_VIEW)));

    let user_create_routes = OpenApiRouter::new()
        .routes(routes!(create_user_handler))
        .layer(axum::middleware::from_fn(api::permission::require(crate::perms::USER_CREATE)));

    let user_update_routes = OpenApiRouter::new()
        .routes(routes!(update_user_handler))
        .layer(axum::middleware::from_fn(api::permission::require(crate::perms::USER_UPDATE)));

    let user_disable_routes = OpenApiRouter::new()
        .routes(routes!(disable_user_handler))
        .layer(axum::middleware::from_fn(api::permission::require(crate::perms::USER_DISABLE)));

    let user_reset_pwd_routes = OpenApiRouter::new()
        .routes(routes!(reset_user_password_handler))
        .layer(axum::middleware::from_fn(api::permission::require(crate::perms::USER_RESET_PASSWORD)));

    let role_view_routes = OpenApiRouter::new()
        .routes(routes!(list_permissions_handler))
        .routes(routes!(list_permissions_contract_handler))
        .routes(routes!(list_roles_handler))
        .routes(routes!(get_role_handler))
        .layer(axum::middleware::from_fn(api::permission::require(crate::perms::ROLE_VIEW)));

    let role_create_routes = OpenApiRouter::new()
        .routes(routes!(create_role_handler))
        .layer(axum::middleware::from_fn(api::permission::require(crate::perms::ROLE_CREATE)));

    let role_update_routes = OpenApiRouter::new()
        .routes(routes!(update_role_handler))
        .routes(routes!(set_role_permissions_handler))
        .routes(routes!(set_role_permissions_post_handler))
        .layer(axum::middleware::from_fn(api::permission::require(crate::perms::ROLE_UPDATE)));

    let role_delete_routes = OpenApiRouter::new()
        .routes(routes!(delete_role_handler))
        .layer(axum::middleware::from_fn(api::permission::require(crate::perms::ROLE_DELETE)));

    let role_assign_routes = OpenApiRouter::new()
        .routes(routes!(assign_role_handler))
        .routes(routes!(revoke_role_handler))
        .routes(routes!(revoke_user_role_handler))
        .layer(axum::middleware::from_fn(api::permission::require(crate::perms::ROLE_ASSIGN)));

    let device_view_routes = OpenApiRouter::new()
        .routes(routes!(list_devices_handler))
        .layer(axum::middleware::from_fn(api::permission::require(crate::perms::DEVICE_VIEW)));

    let device_reg_routes = OpenApiRouter::new()
        .routes(routes!(register_device_handler))
        .layer(axum::middleware::from_fn(api::permission::require(crate::perms::DEVICE_REGISTER)));

    let device_revoke_routes = OpenApiRouter::new()
        .routes(routes!(revoke_device_handler))
        .layer(axum::middleware::from_fn(api::permission::require(crate::perms::DEVICE_REVOKE)));

    let session_view_routes = OpenApiRouter::new()
        .routes(routes!(list_sessions_handler))
        .layer(axum::middleware::from_fn(api::permission::require(crate::perms::SESSION_VIEW)));

    let session_revoke_routes = OpenApiRouter::new()
        .routes(routes!(revoke_session_handler))
        .layer(axum::middleware::from_fn(api::permission::require(crate::perms::SESSION_REVOKE)));

    OpenApiRouter::new()
        .merge(authed_routes)
        .merge(user_view_routes)
        .merge(user_create_routes)
        .merge(user_update_routes)
        .merge(user_disable_routes)
        .merge(user_reset_pwd_routes)
        .merge(role_view_routes)
        .merge(role_create_routes)
        .merge(role_update_routes)
        .merge(role_delete_routes)
        .merge(role_assign_routes)
        .merge(device_view_routes)
        .merge(device_reg_routes)
        .merge(device_revoke_routes)
        .merge(session_view_routes)
        .merge(session_revoke_routes)
}

pub fn platform_router() -> OpenApiRouter {
    let authed = OpenApiRouter::new()
        .routes(routes!(open_impersonation_handler))
        .routes(routes!(create_impersonation_session_handler))
        .routes(routes!(enable_impersonation_writes_handler))
        .routes(routes!(enable_impersonation_writes_contract_handler))
        .routes(routes!(end_impersonation_handler))
        .routes(routes!(list_platform_users_handler, create_platform_user_handler))
        .layer(axum::middleware::from_fn(api::require_platform_middleware));

    OpenApiRouter::new()
        .routes(routes!(platform_step_one_handler))
        .routes(routes!(platform_login_handler))
        .routes(routes!(platform_step_two_handler))
        .routes(routes!(platform_login_totp_handler))
        .merge(authed)
}
