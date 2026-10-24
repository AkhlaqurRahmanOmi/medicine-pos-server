//! The OpenAPI root other crates attach to.
//!
//! This holds the document's identity — title, tags, the bearer scheme — and the two paths
//! `api` owns itself. **Module paths are not listed here.** Each module crate builds an
//! [`utoipa_axum::router::OpenApiRouter`] and registers its handlers with `routes!`, which
//! ties a route to its `#[utoipa::path]` so a route cannot be registered without its
//! documentation (Architecture & Tech Stack §7). `crates/server` nests those routers onto
//! this root and splits the result into a router and a document.

use utoipa::openapi::path::Operation;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::openapi::{Content, Ref, RefOr, Response, ResponseBuilder};
use utoipa::{Modify, OpenApi};

use crate::envelope::{ErrorBody, ErrorDetail};
use crate::health;

#[derive(OpenApi)]
#[openapi(
    paths(health::live, health::ready,),
    components(schemas(ErrorBody, ErrorDetail)),
    tags(
        (name = "health", description = "Liveness and readiness. No authentication."),
        (name = "tenancy", description = "The pharmacy's own profile, settings, branches, plans and payments."),
        (name = "platform", description = "Platform staff only: approval, suspension, subscriptions and payment review."),
        (name = "auth", description = "Authentication: login, token refresh, logout."),
        (name = "identity", description = "Identity, users, roles, permissions and devices."),
    ),
    modifiers(&BearerAuth),
    info(
        title = "Medicine POS API",
        version = "1.0.0",
        description = "Pharmacy POS Backend API"
    )
)]
pub struct ApiDoc;

/// Declares the `bearer` security scheme the handlers reference as `security(("bearer" = []))`.
///
/// Registered through a [`Modify`] rather than the derive because the scheme is one fact for
/// the whole API, and Scalar's "try it" panel needs it to offer a token field at all.
pub struct BearerAuth;

impl Modify for BearerAuth {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        // `components` is `None` until something adds a schema, and the modifier runs before
        // any module router has attached — so insert rather than assume.
        openapi
            .components
            .get_or_insert_with(Default::default)
            .add_security_scheme(
                "bearer",
                SecurityScheme::Http(
                    HttpBuilder::new()
                        .scheme(HttpAuthScheme::Bearer)
                        .bearer_format("JWT")
                        .build(),
                ),
            );
    }
}

/// Adds the failure modes every operation shares, derived from the assembled document.
///
/// utoipa reads the success type out of the handler's signature, but it cannot see how a
/// request fails: `401` comes from the auth layer, `403` from the access gate, `429` from the
/// rate limiter, `500` from `ApiError`'s catch-all — none of which the handler mentions.
/// Writing them out per handler means remembering to, on every endpoint of every module
/// forever, and the one that is forgotten is invisible.
///
/// So they are inferred from facts already in the document:
///
/// | Added | When |
/// |---|---|
/// | `400` | the operation takes a request body |
/// | `401` · `403` · `429` · `500` | the operation declares `security` |
/// | `404` | that, and the path has a `{parameter}` |
///
/// **An entry the handler wrote itself is never replaced**, so a specific `409
/// duplicate_branch_name` keeps its own description and this only fills the gaps. A module
/// added next year is covered without its author knowing this function exists — the cost is
/// that an endpoint which genuinely cannot 404 still says it might.
#[must_use]
pub fn with_standard_errors(mut doc: utoipa::openapi::OpenApi) -> utoipa::openapi::OpenApi {
    for (path, item) in &mut doc.paths.paths {
        let path_has_parameter = path.contains('{');

        let operations = [
            item.get.as_mut(),
            item.put.as_mut(),
            item.post.as_mut(),
            item.delete.as_mut(),
            item.patch.as_mut(),
            item.head.as_mut(),
            item.options.as_mut(),
            item.trace.as_mut(),
        ];

        for operation in operations.into_iter().flatten() {
            for (status, description) in standard_errors_for(operation, path_has_parameter) {
                operation
                    .responses
                    .responses
                    .entry(status.to_string())
                    .or_insert_with(|| error_response(description));
            }
        }
    }

    doc
}

/// Which failures an operation can have, read off the operation itself.
fn standard_errors_for(
    operation: &Operation,
    path_has_parameter: bool,
) -> Vec<(&'static str, &'static str)> {
    let mut errors = Vec::new();

    if operation.request_body.is_some() {
        errors.push((
            "400",
            "`validation_failed`, `unknown_field` or `bad_request`. Unknown fields are \
             rejected rather than ignored, so a typo fails loudly instead of silently doing \
             nothing. Validation errors list the offending fields in `details.fields`.",
        ));
    }

    // No token required, no token-shaped failures: this is what keeps `/health` honest.
    if operation.security.is_some() {
        errors.push((
            "401",
            "`invalid_token`, `token_expired` or `device_revoked`.",
        ));
        errors.push((
            "403",
            "`permission_denied`, `branch_not_allowed`, `subscription_read_only`, \
             `tenant_suspended` or `tenant_pending_approval`. The last two are never collapsed \
             into one code.",
        ));

        if path_has_parameter {
            errors.push((
                "404",
                "`not_found` — and what another pharmacy's row returns too, never `403`, \
                 because a 403 would confirm the row exists.",
            ));
        }

        errors.push(("429", "`too_many_requests`."));
        errors.push((
            "500",
            "`internal`. The cause is logged against the response's `trace_id` and never put \
             in the body.",
        ));
    }

    errors
}

/// Every error, at every status, is the one `ErrorBody` shape (`CONTRACT.md` §2).
fn error_response(description: &'static str) -> RefOr<Response> {
    RefOr::T(
        ResponseBuilder::new()
            .description(description)
            .content(
                "application/json",
                Content::new(Some(Ref::from_schema_name("ErrorBody"))),
            )
            .build(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_document_builds() {
        let doc = ApiDoc::openapi();
        assert_eq!(doc.info.title, "Medicine POS API");
        assert!(!doc.paths.paths.is_empty());
        let json_str = doc.to_json().expect("serialize openapi doc");
        assert!(json_str.contains("/health"));
    }

    #[test]
    fn the_root_declares_the_bearer_scheme() {
        let doc = ApiDoc::openapi();
        let components = doc
            .components
            .expect("the bearer modifier must create components");
        assert!(
            components.security_schemes.contains_key("bearer"),
            "handlers cite `security((\"bearer\" = []))`; without the scheme Scalar offers no \
             token field and the citation dangles"
        );
    }
}
