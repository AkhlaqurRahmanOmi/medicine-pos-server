use axum::{
    extract::Request,
    http::{HeaderName, HeaderValue},
    middleware::Next,
    response::Response,
};
use pos_core::ids::RequestId;

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
}
