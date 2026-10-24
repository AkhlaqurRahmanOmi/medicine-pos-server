use api::{ApiCreated, ApiError, ApiOk, ErrorBody, TenantTx};
use axum::extract::{Extension, Json, Path};
use cache::Cache;
use db::Db;
use pos_core::context::TenantContext;
use pos_core::ids::{DeviceId, RoleId, TenantId, UserId};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::types::{
    AssignRoleRequest, CreateRoleRequest, CreateUserRequest, Device, ImpersonationSessionItem,
    LoginRequest, LoginResponse, LogoutRequest, OpenImpersonationRequest,
    Permission, PlatformLoginResponse, PlatformLoginStepOneRequest, PlatformLoginStepOneResponse,
    PlatformLoginStepTwoRequest, RefreshTokenRequest, RefreshTokenResponse, RegisterDeviceRequest,
    Role, SetPinRequest, SetRolePermissionsRequest, UpdateRoleRequest, UpdateUserRequest, User,
};
use crate::settings::TokenSettings;
use crate::{commands, login, platform, queries};

// -----------------------------------------------------------------------------
// Auth Routes
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
    let users = queries::list_users(db.conn(), ctx.tenant_id)
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
    let user = queries::get_user(db.conn(), ctx.tenant_id, UserId::from(id))
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
    commands::set_user_pin(&tx.0, ctx.user_id, &body.pin)
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(serde_json::json!({ "status": "pin_set" })))
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
    let roles = queries::list_roles(db.conn(), ctx.tenant_id)
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
    let role = queries::get_role(db.conn(), ctx.tenant_id, RoleId::from(id))
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
    post, path = "/roles/{id}/permissions", tag = "identity", security(("bearer" = [])),
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
    let devices = queries::list_devices(db.conn(), ctx.tenant_id)
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

// -----------------------------------------------------------------------------
// Routers
// -----------------------------------------------------------------------------

pub fn auth_router() -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(login_handler))
        .routes(routes!(refresh_handler))
        .routes(routes!(logout_handler))
}

pub fn pharmacy_router() -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_users_handler, create_user_handler))
        .routes(routes!(get_user_handler, update_user_handler))
        .routes(routes!(disable_user_handler))
        .routes(routes!(set_pin_handler))
        .routes(routes!(list_roles_handler, create_role_handler))
        .routes(routes!(list_permissions_handler))
        .routes(routes!(get_role_handler, update_role_handler, delete_role_handler))
        .routes(routes!(set_role_permissions_handler))
        .routes(routes!(assign_role_handler))
        .routes(routes!(revoke_role_handler))
        .routes(routes!(list_devices_handler, register_device_handler))
        .routes(routes!(revoke_device_handler))
        .routes(routes!(list_impersonations_handler))
}

pub fn platform_router() -> OpenApiRouter {
    let authed = OpenApiRouter::new()
        .routes(routes!(open_impersonation_handler))
        .routes(routes!(enable_impersonation_writes_handler))
        .layer(axum::middleware::from_fn(api::require_platform_middleware));

    OpenApiRouter::new()
        .routes(routes!(platform_step_one_handler))
        .routes(routes!(platform_step_two_handler))
        .merge(authed)
}

