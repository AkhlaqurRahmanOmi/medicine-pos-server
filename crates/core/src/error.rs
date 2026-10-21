use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    BadRequest,
    ValidationFailed,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    RateLimited,
    TenantSuspended,
    SubscriptionLapsed,
    AppVersionUnsupported,
    Internal,
}

impl ErrorCode {
    #[must_use]
    pub const fn http_status(&self) -> u16 {
        match self {
            Self::BadRequest => 400,
            Self::ValidationFailed => 422,
            Self::Unauthorized => 401,
            Self::Forbidden => 403,
            Self::NotFound => 404,
            Self::Conflict => 409,
            Self::RateLimited => 429,
            Self::TenantSuspended => 403,
            Self::SubscriptionLapsed => 402,
            Self::AppVersionUnsupported => 426,
            Self::Internal => 500,
        }
    }

    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::BadRequest => "BAD_REQUEST",
            Self::ValidationFailed => "VALIDATION_FAILED",
            Self::Unauthorized => "UNAUTHORIZED",
            Self::Forbidden => "FORBIDDEN",
            Self::NotFound => "NOT_FOUND",
            Self::Conflict => "CONFLICT",
            Self::RateLimited => "RATE_LIMITED",
            Self::TenantSuspended => "TENANT_SUSPENDED",
            Self::SubscriptionLapsed => "SUBSCRIPTION_LAPSED",
            Self::AppVersionUnsupported => "APP_VERSION_UNSUPPORTED",
            Self::Internal => "INTERNAL",
        }
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
    pub user_message: String,
    pub detail: Option<String>,
}

impl AppError {
    #[must_use]
    pub fn new(code: ErrorCode, user_message: impl Into<String>) -> Self {
        Self {
            code,
            user_message: user_message.into(),
            detail: None,
        }
    }

    #[must_use]
    pub fn with_detail(
        code: ErrorCode,
        user_message: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            code,
            user_message: user_message.into(),
            detail: Some(detail.into()),
        }
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

    #[must_use]
    pub fn unauthorized(user_message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Unauthorized, user_message)
    }

    #[must_use]
    pub fn forbidden(user_message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Forbidden, user_message)
    }

    #[must_use]
    pub fn validation_failed(user_message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ValidationFailed, user_message)
    }

    #[must_use]
    pub fn conflict(user_message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Conflict, user_message)
    }

    #[must_use]
    pub fn rate_limited(user_message: impl Into<String>) -> Self {
        Self::new(ErrorCode::RateLimited, user_message)
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
        let codes = [
            ErrorCode::BadRequest,
            ErrorCode::ValidationFailed,
            ErrorCode::Unauthorized,
            ErrorCode::Forbidden,
            ErrorCode::NotFound,
            ErrorCode::Conflict,
            ErrorCode::RateLimited,
            ErrorCode::TenantSuspended,
            ErrorCode::SubscriptionLapsed,
            ErrorCode::AppVersionUnsupported,
            ErrorCode::Internal,
        ];

        for code in codes {
            let status = code.http_status();
            assert!((400..=599).contains(&status));
            assert!(!code.as_str().is_empty());
        }
    }
}
