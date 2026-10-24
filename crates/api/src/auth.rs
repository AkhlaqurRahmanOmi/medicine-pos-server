use axum::{
    extract::Request,
    response::{IntoResponse, Response},
};
use pos_core::error::AppError;
use pos_core::ports::BoxFuture;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tower::{Layer, Service};

use crate::error::ApiError;
use crate::extract::AuthUser;

pub trait Authenticator: Send + Sync + 'static {
    fn authenticate<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<AuthUser, AppError>>;
}

impl<T: Authenticator> Authenticator for Arc<T> {
    fn authenticate<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<AuthUser, AppError>> {
        (**self).authenticate(token)
    }
}

impl Authenticator for Arc<dyn Authenticator> {
    fn authenticate<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<AuthUser, AppError>> {
        (**self).authenticate(token)
    }
}

impl<F, Fut> Authenticator for F
where
    F: Fn(String) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<AuthUser, AppError>> + Send + 'static,
{
    fn authenticate<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<AuthUser, AppError>> {
        let fut = (self)(token.to_string());
        Box::pin(fut)
    }
}

pub struct AuthLayer<A> {
    authenticator: Arc<A>,
}

impl<A> Clone for AuthLayer<A> {
    fn clone(&self) -> Self {
        Self {
            authenticator: Arc::clone(&self.authenticator),
        }
    }
}

pub fn auth_layer<A: Authenticator>(authenticator: A) -> AuthLayer<A> {
    AuthLayer {
        authenticator: Arc::new(authenticator),
    }
}

impl<S, A> Layer<S> for AuthLayer<A> {
    type Service = AuthService<S, A>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthService {
            inner,
            authenticator: Arc::clone(&self.authenticator),
        }
    }
}

pub struct AuthService<S, A> {
    inner: S,
    authenticator: Arc<A>,
}

impl<S: Clone, A> Clone for AuthService<S, A> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            authenticator: Arc::clone(&self.authenticator),
        }
    }
}

pub async fn auth_middleware<A: Authenticator>(
    authenticator: Arc<A>,
    mut req: Request,
    next: axum::middleware::Next,
) -> Response {
    let auth_header = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let token = match auth_header {
        Some(h) => {
            let t = h.strip_prefix("Bearer ").unwrap_or(h).trim();
            if t.is_empty() {
                return ApiError::from(AppError::unauthorized("Missing authorization token"))
                    .into_response();
            }
            t
        }
        None => {
            return ApiError::from(AppError::unauthorized("Authorization header missing"))
                .into_response();
        }
    };

    match authenticator.authenticate(token).await {
        Ok(user) => {
            let branch_ids = user.branch_permissions.keys().copied().collect();
            let ctx = pos_core::context::TenantContext::new(
                user.tenant_id,
                user.user_id,
                branch_ids,
            );
            req.extensions_mut().insert(ctx);
            req.extensions_mut().insert(user);
            next.run(req).await
        }
        Err(err) => ApiError::from(err).into_response(),
    }
}

impl<S, A> Service<Request> for AuthService<S, A>
where
    S: Service<Request, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
    A: Authenticator,
{
    type Response = Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request) -> Self::Future {
        let auth = Arc::clone(&self.authenticator);
        let mut inner = self.inner.clone();

        Box::pin(async move {
            let auth_header = req
                .headers()
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok());

            let token = match auth_header {
                Some(h) => {
                    let t = h.strip_prefix("Bearer ").unwrap_or(h).trim();
                    if t.is_empty() {
                        return Ok(
                            ApiError::from(AppError::unauthorized("Missing authorization token"))
                                .into_response(),
                        );
                    }
                    t
                }
                None => {
                    return Ok(
                        ApiError::from(AppError::unauthorized("Authorization header missing"))
                            .into_response(),
                    );
                }
            };

            match auth.authenticate(token).await {
                Ok(user) => {
                    let branch_ids = user.branch_permissions.keys().copied().collect();
                    let ctx = pos_core::context::TenantContext::new(
                        user.tenant_id,
                        user.user_id,
                        branch_ids,
                    );
                    req.extensions_mut().insert(ctx);
                    req.extensions_mut().insert(user);
                    inner.call(req).await
                }
                Err(err) => Ok(ApiError::from(err).into_response()),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::get,
        Router,
    };
    use pos_core::ids::{TenantId, UserId};
    use std::sync::atomic::{AtomicBool, Ordering};
    use tower::ServiceExt;

    struct DummyAuth(AuthUser);

    impl Authenticator for DummyAuth {
        fn authenticate<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<AuthUser, AppError>> {
            if token == "valid-token" {
                let u = self.0.clone();
                Box::pin(async move { Ok(u) })
            } else {
                Box::pin(async move { Err(AppError::unauthorized("Invalid token")) })
            }
        }
    }

    #[tokio::test]
    async fn auth_layer_inserts_the_user_it_was_given() {
        let expected_user = AuthUser {
            user_id: UserId::new_v7(),
            tenant_id: TenantId::new_v7(),
            permissions: vec!["role.view".into()],
            branch_permissions: Default::default(),
        };

        let dummy_auth = DummyAuth(expected_user.clone());

        let user_to_check = expected_user.clone();
        let app = Router::new()
            .route(
                "/me",
                get(move |user: AuthUser| {
                    let expected = user_to_check.clone();
                    async move {
                        assert_eq!(user.user_id, expected.user_id);
                        assert_eq!(user.tenant_id, expected.tenant_id);
                        assert_eq!(user.permissions, expected.permissions);
                        StatusCode::OK
                    }
                }),
            )
            .layer(auth_layer(dummy_auth));

        let req = Request::builder()
            .uri("/me")
            .header("Authorization", "Bearer valid-token")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn a_request_without_a_token_is_refused_by_the_layer() {
        static HANDLER_CALLED: AtomicBool = AtomicBool::new(false);

        let dummy_user = AuthUser {
            user_id: UserId::new_v7(),
            tenant_id: TenantId::new_v7(),
            permissions: vec![],
            branch_permissions: Default::default(),
        };

        let dummy_auth = DummyAuth(dummy_user);

        let app = Router::new()
            .route(
                "/guarded",
                get(|| async {
                    HANDLER_CALLED.store(true, Ordering::SeqCst);
                    StatusCode::OK
                }),
            )
            .layer(auth_layer(dummy_auth));

        // 1. Request with no Authorization header
        let req_no_header = Request::builder()
            .uri("/guarded")
            .body(Body::empty())
            .unwrap();

        let resp_no_header = app.clone().oneshot(req_no_header).await.unwrap();
        assert_eq!(resp_no_header.status(), StatusCode::UNAUTHORIZED);
        assert!(!HANDLER_CALLED.load(Ordering::SeqCst));

        // 2. Request with empty Bearer token
        let req_empty_token = Request::builder()
            .uri("/guarded")
            .header("Authorization", "Bearer   ")
            .body(Body::empty())
            .unwrap();

        let resp_empty_token = app.oneshot(req_empty_token).await.unwrap();
        assert_eq!(resp_empty_token.status(), StatusCode::UNAUTHORIZED);
        assert!(!HANDLER_CALLED.load(Ordering::SeqCst));
    }
}
