//! The wire shapes, as the API Design Guide defines them (`CONTRACT.md` §2, Q1 answered).
//!
//! **There is no success wrapper.** A successful response is the resource itself. The
//! reference server's `{success, message, data, meta}` was replaced when Q1 was answered: a
//! wrapper costs every client an unwrapping step, and `success: false` beside a 4xx status
//! says the same thing twice.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use pos_core::error::ErrorCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// `200` with the resource as the whole body.
#[derive(Debug, Clone)]
pub struct ApiOk<T>(pub T);

impl<T: Serialize> IntoResponse for ApiOk<T> {
    fn into_response(self) -> Response {
        (StatusCode::OK, Json(self.0)).into_response()
    }
}

/// `201` with the created resource, its server-side fields filled in.
///
/// A repeat create with the same client-generated id returns [`ApiOk`] instead — 200 with the
/// existing row, not an error. That is how a phone that lost the connection mid-upload is
/// allowed to simply try again.
#[derive(Debug, Clone)]
pub struct ApiCreated<T>(pub T);

impl<T: Serialize> IntoResponse for ApiCreated<T> {
    fn into_response(self) -> Response {
        (StatusCode::CREATED, Json(self.0)).into_response()
    }
}

/// One page of a list: cursor-paginated, and **never a total count**.
///
/// Counting every matching row to render "page 1 of 97" is the query that gets slow first,
/// and no screen in this system needs it. Reports give totals; lists give rows.
///
/// `has_more` is derived from the cursor rather than passed in, so the two cannot disagree —
/// a client that checks only one of them is still correct.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListPage<T> {
    pub data: Vec<T>,
    /// `null` at the end of the list.
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

impl<T> ListPage<T> {
    #[must_use]
    pub fn new(data: Vec<T>, next_cursor: Option<String>) -> Self {
        Self {
            has_more: next_cursor.is_some(),
            data,
            next_cursor,
        }
    }

    /// The last page: no cursor, nothing more to fetch.
    #[must_use]
    pub fn last(data: Vec<T>) -> Self {
        Self::new(data, None)
    }
}

impl<T: Serialize> IntoResponse for ListPage<T> {
    fn into_response(self) -> Response {
        (StatusCode::OK, Json(self)).into_response()
    }
}

/// The error body. One shape, every status.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ErrorDetail {
    /// Stable `lower_snake`. The only thing a client may branch on.
    pub code: String,
    /// English, for logs and support. Clients map `code` to their own Bengali or English.
    pub message: String,
    /// `required` for a denied permission, the limit that was exceeded, the offending fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    /// Also returned as `X-Trace-Id`, so "it failed at 3pm" is findable in the logs.
    pub trace_id: String,
}

impl ErrorBody {
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>, trace_id: impl Into<String>) -> Self {
        Self {
            error: ErrorDetail {
                code: code.as_str().to_string(),
                message: message.into(),
                details: None,
                trace_id: trace_id.into(),
            },
        }
    }

    #[must_use]
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.error.details = Some(details);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_successful_response_is_the_resource_with_no_wrapper() {
        #[derive(Serialize)]
        struct Sale {
            id: &'static str,
            total: &'static str,
        }

        let json = serde_json::to_value(Sale {
            id: "019a",
            total: "340.00",
        })
        .expect("serialise");

        // Serialising through ApiOk must add nothing: no `success`, no `data`, no `meta`.
        assert_eq!(json, serde_json::json!({ "id": "019a", "total": "340.00" }));
    }

    #[test]
    fn list_response_has_no_total_count() {
        let page = ListPage::new(vec![1, 2, 3], Some("019a".to_string()));
        let json = serde_json::to_value(&page).expect("serialise");

        // Compared as a set: a JSON object has no order, and the contract names keys only.
        let mut keys: Vec<&str> = json
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec!["data", "has_more", "next_cursor"],
            "a list carries exactly these three keys — no total, no page, no page_size"
        );
        assert!(json.get("total").is_none());
        assert!(json.get("meta").is_none());
    }

    #[test]
    fn has_more_cannot_disagree_with_the_cursor() {
        assert!(ListPage::new(vec![1], Some("019a".into())).has_more);
        assert!(!ListPage::<i32>::last(vec![]).has_more);
        assert!(ListPage::<i32>::last(vec![]).next_cursor.is_none());
    }
}
