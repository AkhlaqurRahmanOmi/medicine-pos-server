use chrono::{DateTime, Utc};
use pos_core::ids::{BranchId, DeviceId, RoleId, SessionId, UserId};
use serde::{Deserialize, Serialize};

/// Opaque wrapper for sensitive secrets (hashes, tokens, keys).
/// It intentionally implements no `Serialize` and its `Debug` prints `[redacted]`.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret<T>(T);

impl<T> Secret<T> {
    pub fn new(val: T) -> Self {
        Self(val)
    }

    pub fn expose(&self) -> &T {
        &self.0
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> std::fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[redacted]")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UserStatus {
    Active,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeviceStatus {
    Active,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DevicePlatform {
    Android,
    Ios,
    Web,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlatformRole {
    SuperAdmin,
    Reviewer,
    Support,
    Billing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthTokenPurpose {
    PasswordReset,
    PhoneVerify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TokenAudience {
    Pharmacy,
    Platform,
}

// ----------------------------------------------------------------------------
// Domain Resource Shapes (Responses)
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct User {
    #[schema(value_type = String)]
    pub id: UserId,
    pub name: String,
    pub email: String,
    pub phone: Option<String>,
    pub status: UserStatus,
    pub must_change_password: bool,
    pub last_login_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Me {
    pub user: User,
    pub permissions: Vec<String>,
    #[schema(value_type = Vec<String>)]
    pub branch_ids: Vec<BranchId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Role {
    #[schema(value_type = String)]
    pub id: RoleId,
    pub name: String,
    pub is_system: bool,
    pub is_owner: bool,
    pub template_code: Option<String>,
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Permission {
    pub code: String,
    pub module: String,
    pub description: String,
    pub is_branch_scoped: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Device {
    #[schema(value_type = String)]
    pub id: DeviceId,
    #[schema(value_type = String)]
    pub branch_id: BranchId,
    pub name: String,
    pub invoice_prefix: String,
    pub status: DeviceStatus,
    pub app_version: Option<String>,
    pub last_seen_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Session {
    #[schema(value_type = String)]
    pub id: SessionId,
    #[schema(value_type = String)]
    pub user_id: UserId,
    #[schema(value_type = Option<String>)]
    pub device_id: Option<DeviceId>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ImpersonationSessionItem {
    pub id: uuid::Uuid,
    pub platform_user_id: uuid::Uuid,
    pub reason: String,
    pub writes_enabled_at: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ResetPasswordResponse {
    pub temporary_password: String,
}

// ----------------------------------------------------------------------------
// Request / Input Shapes
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
    #[schema(value_type = Option<String>)]
    pub device_id: Option<DeviceId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct LoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
    pub user: User,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RefreshTokenRequest {
    pub refresh_token: String,
    #[schema(value_type = Option<String>)]
    pub device_id: Option<DeviceId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RefreshTokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct LogoutRequest {
    pub refresh_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PasswordResetRequest {
    pub email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PasswordResetConfirmRequest {
    pub token: String,
    pub new_password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ChangePasswordRequest {
    pub current_password: Option<String>,
    pub new_password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SetPinRequest {
    pub pin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateUserRequest {
    pub email: String,
    pub name: String,
    pub phone: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct UpdateUserRequest {
    pub name: Option<String>,
    pub phone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateRoleRequest {
    pub name: String,
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct UpdateRoleRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SetRolePermissionsRequest {
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AssignRoleRequest {
    #[schema(value_type = String)]
    pub role_id: RoleId,
    #[schema(value_type = Option<String>)]
    pub branch_id: Option<BranchId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RegisterDeviceRequest {
    #[schema(value_type = String)]
    pub branch_id: BranchId,
    pub name: String,
    pub invoice_prefix: String,
    pub platform: Option<DevicePlatform>,
    pub app_version: Option<String>,
    pub push_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct UnlockSessionRequest {
    #[schema(value_type = String)]
    pub device_id: DeviceId,
    #[schema(value_type = String)]
    pub user_id: UserId,
    pub pin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ManagerOverrideRequest {
    #[schema(value_type = String)]
    pub manager_user_id: UserId,
    pub pin: String,
    #[schema(value_type = Option<String>)]
    pub branch_id: Option<BranchId>,
    pub required_permission: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PlatformLoginStepOneRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PlatformLoginStepOneResponse {
    pub temp_token: String,
    pub requires_totp: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PlatformLoginStepTwoRequest {
    pub temp_token: String,
    pub totp_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PlatformLoginResponse {
    pub access_token: String,
    pub role: String,
    pub expires_in: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct OpenImpersonationRequest {
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreatePlatformUserRequest {
    pub email: String,
    pub role: PlatformRole,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PlatformUser {
    pub id: uuid::Uuid,
    pub email: String,
    pub role: String,
    pub status: String,
    pub last_login_at: Option<DateTime<Utc>>,
}
