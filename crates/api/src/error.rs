use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use pos_core::error::{AppError, ErrorCode};
use std::ops::Deref;

use crate::envelope::{ErrorEnvelope, ResponseMeta};

#[derive(Debug, Clone)]
pub struct ApiError(pub AppError);

impl Deref for ApiError {
    type Target = AppError;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<AppError> for ApiError {
    fn from(err: AppError) -> Self {
        Self(err)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.0.code.http_status())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

        // Security rule AC-S01: internal error details go to log, never to the response body
        let message = match self.0.code {
            ErrorCode::Internal => {
                "An internal server error occurred. Please try again later.".to_string()
            }
            _ => self.0.user_message.clone(),
        };

        let envelope = ErrorEnvelope {
            success: false,
            message,
            errors: Vec::new(),
            meta: ResponseMeta {
                request_id: String::new(),
                code: Some(self.0.code.as_str().to_string()),
                pagination: None,
            },
        };

        (status, Json(envelope)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn every_error_code_has_a_response() {
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
            let app_err = AppError::new(code, "Test error message");
            let api_err = ApiError::from(app_err);
            let resp = api_err.into_response();
            assert_eq!(resp.status().as_u16(), code.http_status());
        }
    }

    #[tokio::test]
    async fn internal_error_hides_detail() {
        let secret = "secret_credentials_leak";
        let app_err = AppError::internal("Database connection failed", secret);
        let api_err = ApiError::from(app_err);
        let resp = api_err.into_response();

        let status = resp.status();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

        let body_bytes = to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("read body");
        let body_str = String::from_utf8(body_bytes.to_vec()).expect("utf8 body");

        assert!(
            !body_str.contains(secret),
            "Internal error detail must never reach the response body"
        );
    }
}
