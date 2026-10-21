use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessEnvelope<T> {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub data: T,
    pub meta: ResponseMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorEnvelope {
    pub success: bool,
    pub message: String,
    pub errors: Vec<FieldError>,
    pub meta: ResponseMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResponseMeta {
    #[serde(rename = "requestId")]
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pagination: Option<PaginationMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationMeta {
    pub total: u64,
    pub page: u32,
    #[serde(rename = "pageSize")]
    pub page_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldError {
    pub field: String,
    pub code: String,
    pub detail: String,
}

pub struct ApiOk<T> {
    pub data: T,
    pub message: Option<String>,
    pub request_id: String,
    pub pagination: Option<PaginationMeta>,
}

impl<T> ApiOk<T> {
    pub fn new(data: T, request_id: impl Into<String>) -> Self {
        Self {
            data,
            message: None,
            request_id: request_id.into(),
            pagination: None,
        }
    }

    pub fn with_message(
        data: T,
        message: impl Into<String>,
        request_id: impl Into<String>,
    ) -> Self {
        Self {
            data,
            message: Some(message.into()),
            request_id: request_id.into(),
            pagination: None,
        }
    }
}

impl<T: Serialize> IntoResponse for ApiOk<T> {
    fn into_response(self) -> Response {
        let envelope = SuccessEnvelope {
            success: true,
            message: self.message,
            data: self.data,
            meta: ResponseMeta {
                request_id: self.request_id,
                code: None,
                pagination: self.pagination,
            },
        };
        (StatusCode::OK, Json(envelope)).into_response()
    }
}
