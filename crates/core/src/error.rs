use serde::{Serialize, Serializer};
use std::collections::BTreeMap;
use std::fmt;
use thiserror::Error;

/// The error codes the API exposes, as the API Design Guide defines them.
///
/// Serialised `lower_snake` — the stable string a client branches on. `message` is for people
/// and may be reworded at any time, so nothing may match on it.
///
/// `Conflict` carries the module's own 409 code: `core` cannot know what a batch is, so
/// `insufficient_stock` and `duplicate_supplier_invoice` arrive as `&'static str` from the
/// module that raises them — the same arrangement permission slugs use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    // 400
    BadRequest,
    ValidationFailed,
    UnknownField,
    // 401
    InvalidToken,
    TokenExpired,
    DeviceRevoked,
    // 403 — read-only and suspended are deliberately different codes: one shows a renewal
    // banner, the other shows nothing else at all.
    PermissionDenied,
    BranchNotAllowed,
    SubscriptionReadOnly,
    TenantSuspended,
    TenantPendingApproval,
    // 404 — also what another pharmacy's row returns. A 403 would confirm the id exists.
    NotFound,
    // 409
    Conflict(&'static str),
    // 413 / 426 / 429 / 500
    FileTooLarge,
    AppTooOld,
    TooManyRequests,
    Internal,
}

impl ErrorCode {
    /// Every variant, for the exhaustiveness tests. `Conflict` stands in with its generic code.
    pub const ALL: &'static [Self] = &[
        Self::BadRequest,
        Self::ValidationFailed,
        Self::UnknownField,
        Self::InvalidToken,
        Self::TokenExpired,
        Self::DeviceRevoked,
        Self::PermissionDenied,
        Self::BranchNotAllowed,
        Self::SubscriptionReadOnly,
        Self::TenantSuspended,
        Self::TenantPendingApproval,
        Self::NotFound,
        Self::Conflict(Self::GENERIC_CONFLICT),
        Self::FileTooLarge,
        Self::AppTooOld,
        Self::TooManyRequests,
        Self::Internal,
    ];

    /// For a conflict with no module-specific code — a unique violation surfaced by `db`,
    /// which cannot know which domain rule the constraint was protecting.
    pub const GENERIC_CONFLICT: &'static str = "conflict";

    #[must_use]
    pub const fn http_status(&self) -> u16 {
        match self {
            Self::BadRequest | Self::ValidationFailed | Self::UnknownField => 400,
            Self::InvalidToken | Self::TokenExpired | Self::DeviceRevoked => 401,
            Self::PermissionDenied
            | Self::BranchNotAllowed
            | Self::SubscriptionReadOnly
            | Self::TenantSuspended
            | Self::TenantPendingApproval => 403,
            Self::NotFound => 404,
            Self::Conflict(_) => 409,
            Self::FileTooLarge => 413,
            Self::AppTooOld => 426,
            Self::TooManyRequests => 429,
            Self::Internal => 500,
        }
    }

    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::BadRequest => "bad_request",
            Self::ValidationFailed => "validation_failed",
            Self::UnknownField => "unknown_field",
            Self::InvalidToken => "invalid_token",
            Self::TokenExpired => "token_expired",
            Self::DeviceRevoked => "device_revoked",
            Self::PermissionDenied => "permission_denied",
            Self::BranchNotAllowed => "branch_not_allowed",
            Self::SubscriptionReadOnly => "subscription_read_only",
            Self::TenantSuspended => "tenant_suspended",
            Self::TenantPendingApproval => "tenant_pending_approval",
            Self::NotFound => "not_found",
            Self::Conflict(code) => code,
            Self::FileTooLarge => "file_too_large",
            Self::AppTooOld => "app_too_old",
            Self::TooManyRequests => "too_many_requests",
            Self::Internal => "internal_error",
        }
    }
}

/// Serialises as the bare code string. There is no `Deserialize`: nothing in this system
/// parses an error code back into a variant — the clients branch on the string.
impl Serialize for ErrorCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Error)]
pub struct AppError {
    pub code: ErrorCode,
    /// English, for logs and support. Each client maps `code` to its own wording, because the
    /// app must show the right sentence while offline.
    pub user_message: String,
    /// Log and Sentry only `[D-85]`. Never reaches a response body.
    pub detail: Option<String>,
    /// The response's `details` object — `required` for a denied permission, a field name for
    /// a validation failure, the limit that was exceeded.
    pub details: BTreeMap<String, String>,
}

impl AppError {
    #[must_use]
    pub fn new(code: ErrorCode, user_message: impl Into<String>) -> Self {
        Self {
            code,
            user_message: user_message.into(),
            detail: None,
            details: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn with_detail(
        code: ErrorCode,
        user_message: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            detail: Some(detail.into()),
            ..Self::new(code, user_message)
        }
    }

    /// Adds one entry to the response's `details` object.
    #[must_use]
    pub fn detail_field(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.details.insert(key.into(), value.into());
        self
    }

    #[must_use]
    pub fn internal(user_message: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::with_detail(ErrorCode::Internal, user_message, detail)
    }

    #[must_use]
    pub fn bad_request(user_message: impl Into<String>) -> Self {
        Self::new(ErrorCode::BadRequest, user_message)
    }

    #[must_use]
    pub fn not_found(user_message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, user_message)
    }

    /// No token, an expired one, or a bad one — 401.
    #[must_use]
    pub fn unauthorized(user_message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidToken, user_message)
    }

    /// Authenticated but not allowed — 403. Name the permission with `detail_field`.
    #[must_use]
    pub fn forbidden(user_message: impl Into<String>) -> Self {
        Self::new(ErrorCode::PermissionDenied, user_message)
    }

    #[must_use]
    pub fn validation_failed(user_message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ValidationFailed, user_message)
    }

    /// A conflict with no module-specific code. A module uses [`Self::conflict_code`].
    #[must_use]
    pub fn conflict(user_message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Conflict(ErrorCode::GENERIC_CONFLICT), user_message)
    }

    /// A conflict carrying the module's own code — `insufficient_stock`, `batch_expired`.
    #[must_use]
    pub fn conflict_code(code: &'static str, user_message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Conflict(code), user_message)
    }

    #[must_use]
    pub fn rate_limited(user_message: impl Into<String>) -> Self {
        Self::new(ErrorCode::TooManyRequests, user_message)
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}]: {}", self.code, self.user_message)?;
        if let Some(detail) = &self.detail {
            write!(f, " ({detail})")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_code_maps_to_a_status() {
        for code in ErrorCode::ALL {
            let status = code.http_status();
            assert!((400..=599).contains(&status), "{code} has status {status}");
            assert!(!code.as_str().is_empty());
        }
    }

    #[test]
    fn codes_are_lower_snake_as_the_api_design_guide_requires() {
        for code in ErrorCode::ALL {
            let s = code.as_str();
            assert!(
                s.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{s} is not lower_snake"
            );
        }
    }

    #[test]
    fn a_module_conflict_carries_its_own_code_at_409() {
        let err = AppError::conflict_code("insufficient_stock", "Not enough stock.");
        assert_eq!(err.code.as_str(), "insufficient_stock");
        assert_eq!(err.code.http_status(), 409);
    }

    #[test]
    fn the_guide_puts_validation_at_400_and_a_lapsed_subscription_at_403() {
        assert_eq!(ErrorCode::ValidationFailed.http_status(), 400);
        assert_eq!(ErrorCode::SubscriptionReadOnly.http_status(), 403);
    }
}
