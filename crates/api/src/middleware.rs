use axum::{
    extract::Request,
    http::{HeaderName, HeaderValue, Method},
    middleware::Next,
    response::{IntoResponse, Response},
};
use pos_core::context::TenantContext;
use pos_core::ids::RequestId;
use pos_core::ports::AccessModeSource;
use std::sync::Arc;

use crate::error::ApiError;
use crate::extract::AuthUser;

pub static REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

pub async fn request_id_middleware(mut req: Request, next: Next) -> Response {
    let request_id = match req
        .headers()
        .get(&REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
    {
        Some(s) if !s.trim().is_empty() => RequestId::new(s.trim()),
        _ => RequestId::new_v7(),
    };

    req.extensions_mut().insert(request_id.clone());

    let id_str = request_id.as_str().to_string();
    let mut resp = next.run(req).await;

    if let Ok(val) = HeaderValue::from_str(&id_str) {
        resp.headers_mut().insert(REQUEST_ID_HEADER.clone(), val);
    }

    resp
}

pub async fn access_gate_middleware(
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let tenant_id = req
        .extensions()
        .get::<TenantContext>()
        .map(|c| c.tenant_id)
        .or_else(|| req.extensions().get::<AuthUser>().map(|u| u.tenant_id));

    if let Some(tenant_id) = tenant_id {
        let Some(source) = req.extensions().get::<Arc<dyn AccessModeSource>>() else {
            return Ok((
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "Access mode source not configured",
            )
                .into_response());
        };
        let is_write = matches!(
            *req.method(),
            Method::POST | Method::PUT | Method::PATCH | Method::DELETE
        );
        source
            .check_access(tenant_id, is_write)
            .await
            .map_err(ApiError::from)?;
    }

    Ok(next.run(req).await)
}

pub async fn require_platform_middleware(
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if req.extensions().get::<crate::extract::PlatformToken>().is_some() {
        Ok(next.run(req).await)
    } else {
        Err(ApiError::from(pos_core::error::AppError::forbidden(
            "Platform authorization required",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        middleware,
        routing::get,
        Router,
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn request_id_generated_when_absent() {
        let app = Router::new()
            .route("/test", get(|| async { StatusCode::OK }))
            .layer(middleware::from_fn(request_id_middleware));

        // 1. When absent: generated
        let req_absent = Request::builder().uri("/test").body(Body::empty()).unwrap();

        let resp_absent = app.clone().oneshot(req_absent).await.unwrap();
        let header_val = resp_absent.headers().get(&REQUEST_ID_HEADER);
        assert!(header_val.is_some(), "Header x-request-id must be present");
        let generated_id = header_val.unwrap().to_str().unwrap();
        assert!(!generated_id.is_empty());

        // 2. When given: preserved
        let given_id = "custom-client-id-456";
        let req_given = Request::builder()
            .uri("/test")
            .header("x-request-id", given_id)
            .body(Body::empty())
            .unwrap();

        let resp_given = app.oneshot(req_given).await.unwrap();
        let preserved_val = resp_given.headers().get(&REQUEST_ID_HEADER);
        assert_eq!(preserved_val.unwrap().to_str().unwrap(), given_id);
    }

    #[tokio::test]
    async fn the_gate_refuses_when_no_access_source_is_wired() {
        let app = Router::new()
            .route("/test", get(|| async { StatusCode::OK }))
            .layer(middleware::from_fn(access_gate_middleware));

        let mut req = Request::builder().uri("/test").body(Body::empty()).unwrap();
        req.extensions_mut().insert(AuthUser {
            user_id: pos_core::ids::UserId::new_v7(),
            tenant_id: pos_core::ids::TenantId::new_v7(),
            permissions: vec![],
            branch_permissions: std::collections::BTreeMap::new(),
        });

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
