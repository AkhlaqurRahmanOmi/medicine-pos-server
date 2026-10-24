use axum::{
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use pos_core::error::{AppError, ErrorCode};
use std::ops::Deref;

use crate::envelope::ErrorBody;

/// The response header carrying the trace id. Always set, on every error response.
pub const TRACE_ID_HEADER: &str = "x-trace-id";

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

impl ApiError {
    /// The trace id to put in the body and the header.
    ///
    /// The middleware generates one per request; until it is threaded through the error path
    /// this falls back to a fresh id, so the field is never empty and never absent.
    fn trace_id(&self) -> String {
        self.0
            .details
            .get("trace_id")
            .cloned()
            .unwrap_or_else(|| pos_core::ids::RequestId::new_v7().as_str().to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.0.code.http_status())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

        // AC-S01: an internal error's detail goes to the log, never to the body. The cause is
        // kept — a database outage is a 500 with its reason logged, never a 400.
        let message = match self.0.code {
            ErrorCode::Internal => {
                "An internal server error occurred. Please try again later.".to_string()
            }
            _ => self.0.user_message.clone(),
        };

        let trace_id = self.trace_id();
        let mut body = ErrorBody::new(self.0.code, message, &trace_id);

        // `details` carries what the client needs to act: the permission it lacked, the limit
        // it exceeded. `trace_id` is a transport key, not a detail, so it is not repeated.
        let details: serde_json::Map<String, serde_json::Value> = self
            .0
            .details
            .iter()
            .filter(|(k, _)| k.as_str() != "trace_id")
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect();
        if !details.is_empty() {
            body = body.with_details(serde_json::Value::Object(details));
        }

        let mut response = (status, Json(body)).into_response();
        if let Ok(value) = HeaderValue::from_str(&trace_id) {
            response.headers_mut().insert(TRACE_ID_HEADER, value);
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use crate::envelope::ErrorBody as Body;

    async fn body_of(err: ApiError) -> (StatusCode, serde_json::Value) {
        let resp = err.into_response();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
        (status, serde_json::from_slice(&bytes).expect("json body"))
    }

    #[tokio::test]
    async fn every_error_code_has_a_response() {
        for code in ErrorCode::ALL {
            let resp = ApiError::from(AppError::new(*code, "Test error message")).into_response();
            assert_eq!(resp.status().as_u16(), code.http_status(), "{code}");
            assert!(
                resp.headers().contains_key(TRACE_ID_HEADER),
                "{code} response carries no trace id header"
            );
        }
    }

    #[tokio::test]
    async fn error_body_is_a_single_error_object() {
        let err = AppError::conflict_code("over_credit_limit", "Past their credit limit.")
            .detail_field("credit_limit", "5000.00");
        let (status, json) = body_of(ApiError::from(err)).await;

        assert_eq!(status.as_u16(), 409);

        // Exactly one top-level key, and it is `error`. No `success`, no `data`, no `meta`.
        let object = json.as_object().expect("an object");
        assert_eq!(object.keys().collect::<Vec<_>>(), vec!["error"]);

        let error = &json["error"];
        assert_eq!(error["code"], "over_credit_limit");
        assert_eq!(error["message"], "Past their credit limit.");
        assert_eq!(error["details"]["credit_limit"], "5000.00");
        assert!(
            error["trace_id"].as_str().is_some_and(|s| !s.is_empty()),
            "every error carries a trace id"
        );
    }

    #[tokio::test]
    async fn a_denied_permission_names_what_it_needed() {
        let err = AppError::forbidden("You cannot void a sale.").detail_field("required", "sales.void");
        let (status, json) = body_of(ApiError::from(err)).await;

        assert_eq!(status.as_u16(), 403);
        assert_eq!(json["error"]["code"], "permission_denied");
        assert_eq!(json["error"]["details"]["required"], "sales.void");
    }

    #[tokio::test]
    async fn internal_error_hides_detail() {
        let secret = "secret_credentials_leak";
        let (status, json) = body_of(ApiError::from(AppError::internal(
            "Database connection failed",
            secret,
        )))
        .await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(json["error"]["code"], "internal_error");

        let rendered = serde_json::to_string(&json).expect("render");
        assert!(
            !rendered.contains(secret),
            "an internal error's detail must never reach the response body"
        );
        // The user-facing message is replaced, not passed through.
        assert!(!rendered.contains("Database connection failed"));
    }

    #[tokio::test]
    async fn the_body_round_trips_into_the_published_type() {
        let (_, json) = body_of(ApiError::from(AppError::not_found("No such sale."))).await;
        let parsed: Body = serde_json::from_value(json).expect("parses as ErrorBody");
        assert_eq!(parsed.error.code, "not_found");
        assert!(parsed.error.details.is_none(), "details is absent, not null-filled");
    }
}
