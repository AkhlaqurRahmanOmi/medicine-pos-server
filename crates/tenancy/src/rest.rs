use api::{
    access_gate_middleware, require_platform_middleware, ApiCreated, ApiError, ApiOk, ErrorBody,
    ListPage, Pagination, PlatformToken, TenantTx,
};
use axum::{
    extract::{Extension, Json, Path, Query},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
};
use cache::Cache;
use db::{Db, Tx};
use pos_core::context::TenantContext;
use pos_core::error::{AppError, ErrorCode};
use pos_core::ids::{BranchId, TenantId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::perms;
use crate::types::{
    Branch, BranchPatch, NewBranch, NewPayment, NewPlanRequest, NewSubscription, Payment, Plan,
    PlanLimit, SetPlanLimitRequest, SettingsPatch, Subscription, Tenant, TenantDetail,
    TenantExport, TenantPatch, TenantSettings, TenantSummary, UpdatePlanRequest, UploadSlipRequest,
};
use crate::{access, commands, platform, queries};

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct SuspendRequest {
    pub reason: String,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RejectPaymentRequest {
    pub note: String,
}

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct ListTenantsQuery {
    pub status: Option<String>,
}

// -----------------------------------------------------------------------------
// Pharmacy routes
// -----------------------------------------------------------------------------

/// Get the current tenant's profile, settings, and access mode summary.
#[utoipa::path(
    get, path = "/tenant", tag = "tenancy", security(("bearer" = [])),
    responses(
        (status = 200, description = "The pharmacy, with its derived access mode", body = Tenant),
        (status = 403, description = "`tenant_suspended` or `tenant_pending_approval`", body = ErrorBody),
    )
)]
pub async fn get_tenant(
    Extension(ctx): Extension<TenantContext>,
    Extension(db): Extension<Db>,
) -> Result<ApiOk<Tenant>, ApiError> {
    let tenant = queries::tenant(&db, ctx.tenant_id)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(tenant))
}

/// Update tenant details partially.
#[utoipa::path(
    patch, path = "/tenant", tag = "tenancy", security(("bearer" = [])),
    request_body = TenantPatch,
    responses(
        (status = 200, description = "The updated pharmacy", body = Tenant),
        (status = 400, description = "`validation_failed`", body = ErrorBody),
    )
)]
pub async fn patch_tenant(
    tx: TenantTx,
    Json(patch): Json<TenantPatch>,
) -> Result<ApiOk<Tenant>, ApiError> {
    let tenant = commands::update_tenant(&tx.0, patch)
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(tenant))
}

/// Get tenant settings for the authenticated tenant.
#[utoipa::path(
    get, path = "/tenant/settings", tag = "tenancy", security(("bearer" = [])),
    responses(
        (status = 200, description = "Settings the phone caches and reads offline", body = TenantSettings),
        (status = 403, description = "Blocked tenant", body = ErrorBody),
    )
)]
pub async fn get_settings(
    Extension(ctx): Extension<TenantContext>,
    Extension(db): Extension<Db>,
) -> Result<ApiOk<TenantSettings>, ApiError> {
    let settings = queries::settings(&db, ctx.tenant_id)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(settings))
}

/// Update tenant settings partially.
#[utoipa::path(
    patch, path = "/tenant/settings", tag = "tenancy", security(("bearer" = [])),
    request_body = SettingsPatch,
    responses(
        (status = 200, description = "The settings after the patch", body = TenantSettings),
        (status = 403, description = "`subscription_read_only` once the grace period has passed", body = ErrorBody),
    )
)]
pub async fn update_settings(
    tx: TenantTx,
    Json(patch): Json<SettingsPatch>,
) -> Result<ApiOk<TenantSettings>, ApiError> {
    let settings = commands::update_settings(&tx.0, patch)
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(settings))
}

/// List branches of the authenticated tenant with cursor pagination.
#[utoipa::path(
    get, path = "/branches", tag = "tenancy", security(("bearer" = [])),
    params(Pagination),
    responses(
        (status = 200, description = "One page of branches — cursor, and no total count", body = ListPage<Branch>),
        (status = 403, description = "Blocked tenant", body = ErrorBody),
    )
)]
pub async fn list_branches(
    Extension(ctx): Extension<TenantContext>,
    Extension(db): Extension<Db>,
    pagination: Pagination,
) -> Result<ApiOk<ListPage<Branch>>, ApiError> {
    let branches = queries::branches(&db, ctx.tenant_id, pagination)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(branches))
}

/// Create a new branch for the authenticated tenant.
#[utoipa::path(
    post, path = "/branches", tag = "tenancy", security(("bearer" = [])),
    request_body = NewBranch,
    responses(
        (status = 201, description = "The branch, its server-side fields filled in", body = Branch),
        (status = 409, description = "`duplicate_branch_name` or `plan_limit_reached`", body = ErrorBody),
        (status = 403, description = "`subscription_read_only` or a blocked tenant", body = ErrorBody),
    )
)]
pub async fn create_branch(
    tx: TenantTx,
    Json(input): Json<NewBranch>,
) -> Result<ApiCreated<Branch>, ApiError> {
    let branch = commands::create_branch(&tx.0, input)
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiCreated(branch))
}

/// Get a specific branch by ID.
#[utoipa::path(
    get, path = "/branches/{id}", tag = "tenancy", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Branch ID")),
    responses(
        (status = 200, description = "The branch", body = Branch),
        (status = 404, description = "`not_found`", body = ErrorBody),
    )
)]
pub async fn get_branch(
    Extension(ctx): Extension<TenantContext>,
    Extension(db): Extension<Db>,
    Path(id): Path<Uuid>,
) -> Result<ApiOk<Branch>, ApiError> {
    let branch = queries::branch(&db, ctx.tenant_id, BranchId::from_uuid(id))
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(branch))
}

/// Update a branch partially.
#[utoipa::path(
    patch, path = "/branches/{id}", tag = "tenancy", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Branch ID")),
    request_body = BranchPatch,
    responses(
        (status = 200, description = "The updated branch", body = Branch),
        (status = 409, description = "`duplicate_branch_name`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
    )
)]
pub async fn patch_branch(
    tx: TenantTx,
    Path(id): Path<Uuid>,
    Json(patch): Json<BranchPatch>,
) -> Result<ApiOk<Branch>, ApiError> {
    let branch = commands::update_branch(&tx.0, BranchId::from_uuid(id), patch)
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(branch))
}

/// Deactivate an active branch.
#[utoipa::path(
    post, path = "/branches/{id}/deactivate", tag = "tenancy", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Branch to retire")),
    responses(
        (status = 200, description = "The branch, now inactive — never deleted, because stock and sales point at it forever", body = Branch),
        (status = 409, description = "`last_active_branch` — the only active branch, or the default one", body = ErrorBody),
    )
)]
pub async fn deactivate_branch(
    tx: TenantTx,
    Path(id): Path<Uuid>,
) -> Result<ApiOk<Branch>, ApiError> {
    let branch = commands::deactivate_branch(&tx.0, BranchId::from_uuid(id))
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(branch))
}

/// List plans, prices and limits for all authenticated callers.
#[utoipa::path(
    get, path = "/plans", tag = "tenancy", security(("bearer" = [])),
    responses(
        (status = 200, description = "The plan catalogue — platform-owned, readable by every authenticated caller", body = ListPage<Plan>),
    )
)]
pub async fn list_plans(Extension(db): Extension<Db>) -> Result<ApiOk<ListPage<Plan>>, ApiError> {
    let plans = queries::plans(&db).await.map_err(ApiError::from)?;
    Ok(ApiOk(ListPage::new(plans, None)))
}

/// List subscriptions (billing history) for the authenticated tenant.
#[utoipa::path(
    get, path = "/subscriptions", tag = "tenancy", security(("bearer" = [])),
    params(Pagination),
    responses(
        (status = 200, description = "Billing history page", body = ListPage<Subscription>),
    )
)]
pub async fn list_subscriptions(
    Extension(ctx): Extension<TenantContext>,
    Extension(db): Extension<Db>,
    pagination: Pagination,
) -> Result<ApiOk<ListPage<Subscription>>, ApiError> {
    let subs = queries::subscriptions(&db, ctx.tenant_id, pagination)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(subs))
}

/// Get the current covering subscription or 204 if none.
#[utoipa::path(
    get, path = "/subscriptions/current", tag = "tenancy", security(("bearer" = [])),
    responses(
        (status = 200, description = "The covering subscription", body = Subscription),
        (status = 204, description = "No covering subscription"),
    )
)]
pub async fn get_current_subscription(
    Extension(ctx): Extension<TenantContext>,
    Extension(db): Extension<Db>,
) -> Result<Response, ApiError> {
    let sub = queries::current_subscription(&db, ctx.tenant_id)
        .await
        .map_err(ApiError::from)?;
    match sub {
        Some(s) => Ok(ApiOk(s).into_response()),
        None => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}

/// Submit a subscription payment for platform review.
#[utoipa::path(
    post, path = "/subscription-payments", tag = "tenancy", security(("bearer" = [])),
    request_body = NewPayment,
    responses(
        (status = 201, description = "The payment, always `submitted` — a pharmacy cannot confirm its own", body = Payment),
        (status = 409, description = "`duplicate_payment_reference` — one bank slip cannot be used twice", body = ErrorBody),
    )
)]
pub async fn submit_payment(
    tx: TenantTx,
    Json(input): Json<NewPayment>,
) -> Result<ApiCreated<Payment>, ApiError> {
    let payment = commands::submit_payment(&tx.0, input)
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiCreated(payment))
}

/// Upload bank transfer or bKash payment proof slip.
#[utoipa::path(
    post, path = "/subscription-payments/{id}/slip", tag = "tenancy", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Payment ID")),
    request_body = UploadSlipRequest,
    responses(
        (status = 200, description = "The payment with proof_file_key recorded", body = Payment),
        (status = 413, description = "`file_too_large` — exceeds 5MB limit", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
    )
)]
pub async fn upload_slip(
    tx: TenantTx,
    Path(id): Path<Uuid>,
    Json(body): Json<UploadSlipRequest>,
) -> Result<ApiOk<Payment>, ApiError> {
    if body.file_size_bytes.is_some_and(|sz| sz > 5 * 1024 * 1024) {
        return Err(ApiError::from(AppError::new(
            ErrorCode::FileTooLarge,
            "Slip file size exceeds 5MB limit",
        )));
    }
    let payment = commands::upload_payment_slip(&tx.0, id, body.proof_file_key)
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(payment))
}

/// Request a data export for the pharmacy.
#[utoipa::path(
    post, path = "/exports", tag = "tenancy", security(("bearer" = [])),
    responses(
        (status = 202, description = "Export accepted", body = TenantExport),
        (status = 409, description = "`export_already_running`", body = ErrorBody),
    )
)]
pub async fn request_export(
    tx: TenantTx,
    Extension(ctx): Extension<TenantContext>,
) -> Result<(StatusCode, Json<TenantExport>), ApiError> {
    let exp = commands::request_export(&tx.0, ctx.user_id)
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok((StatusCode::ACCEPTED, Json(exp)))
}

/// Get export status and download URL if ready.
#[utoipa::path(
    get, path = "/exports/{id}", tag = "tenancy", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Export ID")),
    responses(
        (status = 200, description = "Export status and URL", body = TenantExport),
        (status = 404, description = "`not_found`", body = ErrorBody),
    )
)]
pub async fn get_export(
    Extension(ctx): Extension<TenantContext>,
    Extension(db): Extension<Db>,
    Path(id): Path<Uuid>,
) -> Result<ApiOk<TenantExport>, ApiError> {
    let exp = queries::export(&db, ctx.tenant_id, id)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(exp))
}

/// Request closure of the pharmacy. Purge date will be set to 30 days later.
#[utoipa::path(
    post, path = "/tenant/closure", tag = "tenancy", security(("bearer" = [])),
    responses(
        (status = 200, description = "The pharmacy, now `closing`, with the purge date 30 days out", body = Tenant),
    )
)]
pub async fn request_closure(
    tx: TenantTx,
    Extension(ctx): Extension<TenantContext>,
) -> Result<ApiOk<Tenant>, ApiError> {
    let tenant = commands::request_closure(&tx.0, ctx.user_id)
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(tenant))
}

/// Constructs the router for pharmacy-facing routes.
/// General routes are gated by `access_gate_middleware`, while payment renewal
/// and closure paths are ungated so lapsed/read-only tenants can submit renewals.
/// Every route enforces the permission specified in CONTRACT.md §2.
pub fn pharmacy_router() -> OpenApiRouter {
    let ungated = OpenApiRouter::new()
        .routes(routes!(submit_payment))
        .routes(routes!(upload_slip))
        .layer(middleware::from_fn(api::permission::require(perms::SUBSCRIPTION_PAY)))
        .merge(
            OpenApiRouter::new()
                .routes(routes!(request_closure))
                .layer(middleware::from_fn(api::permission::require(perms::CLOSE))),
        );

    let view_routes = OpenApiRouter::new()
        .routes(routes!(get_tenant))
        .routes(routes!(get_settings))
        .layer(middleware::from_fn(api::permission::require(perms::VIEW)));

    let tenant_update_routes = OpenApiRouter::new()
        .routes(routes!(patch_tenant))
        .layer(middleware::from_fn(api::permission::require(perms::UPDATE)));

    let settings_update_routes = OpenApiRouter::new()
        .routes(routes!(update_settings))
        .layer(middleware::from_fn(api::permission::require(perms::SETTINGS_UPDATE)));

    let branch_view_routes = OpenApiRouter::new()
        .routes(routes!(list_branches))
        .routes(routes!(get_branch))
        .layer(middleware::from_fn(api::permission::require(perms::BRANCH_VIEW)));

    let branch_create_routes = OpenApiRouter::new()
        .routes(routes!(create_branch))
        .layer(middleware::from_fn(api::permission::require(perms::BRANCH_CREATE)));

    let branch_update_routes = OpenApiRouter::new()
        .routes(routes!(patch_branch))
        .routes(routes!(deactivate_branch))
        .layer(middleware::from_fn(api::permission::require(perms::BRANCH_UPDATE)));

    let sub_view_routes = OpenApiRouter::new()
        .routes(routes!(list_subscriptions))
        .routes(routes!(get_current_subscription))
        .layer(middleware::from_fn(api::permission::require(perms::SUBSCRIPTION_VIEW)));

    let export_routes = OpenApiRouter::new()
        .routes(routes!(request_export))
        .routes(routes!(get_export))
        .layer(middleware::from_fn(api::permission::require(perms::EXPORT)));

    let plans_routes = OpenApiRouter::new().routes(routes!(list_plans));

    let gated = OpenApiRouter::new()
        .merge(view_routes)
        .merge(tenant_update_routes)
        .merge(settings_update_routes)
        .merge(branch_view_routes)
        .merge(branch_create_routes)
        .merge(branch_update_routes)
        .merge(sub_view_routes)
        .merge(export_routes)
        .merge(plans_routes)
        .layer(middleware::from_fn(access_gate_middleware));

    ungated.merge(gated)
}

// -----------------------------------------------------------------------------
// Platform (Admin) routes
// -----------------------------------------------------------------------------

/// List pharmacies with optional status filter.
#[utoipa::path(
    get, path = "/tenants", tag = "platform", security(("bearer" = [])),
    params(Pagination, ListTenantsQuery),
    responses(
        (status = 200, description = "List of pharmacies", body = ListPage<TenantSummary>),
    )
)]
pub async fn list_tenants(
    Extension(db): Extension<Db>,
    pagination: Pagination,
    Query(query): Query<ListTenantsQuery>,
) -> Result<ApiOk<ListPage<TenantSummary>>, ApiError> {
    let list = queries::tenants(&db, query.status, pagination)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(list))
}

/// Get detailed information of a pharmacy including settings, branches, and subscriptions.
#[utoipa::path(
    get, path = "/tenants/{id}", tag = "platform", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Tenant ID")),
    responses(
        (status = 200, description = "Pharmacy details", body = TenantDetail),
        (status = 404, description = "`not_found`", body = ErrorBody),
    )
)]
pub async fn get_tenant_detail(
    Extension(db): Extension<Db>,
    Extension(platform): Extension<PlatformToken>,
    Path(id): Path<Uuid>,
) -> Result<ApiOk<TenantDetail>, ApiError> {
    let tenant_id = TenantId::from_uuid(id);
    let ctx = TenantContext::new(tenant_id, platform.user_id, vec![]);
    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .map_err(ApiError::from)?;
    let detail = platform::get_tenant_detail(&tx, tenant_id)
        .await
        .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(detail))
}

/// Platform staff approves a pharmacy. Sets status to active and records approver and timestamp.
#[utoipa::path(
    post, path = "/tenants/{id}/approve", tag = "platform", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Pharmacy awaiting approval")),
    responses(
        (status = 200, description = "The pharmacy, now active, with the approver recorded", body = Tenant),
        (status = 403, description = "`permission_denied` — platform token required", body = ErrorBody),
    )
)]
pub async fn approve_tenant(
    Extension(db): Extension<Db>,
    Extension(platform): Extension<PlatformToken>,
    Extension(cache): Extension<Cache>,
    Path(id): Path<Uuid>,
) -> Result<ApiOk<Tenant>, ApiError> {
    let tenant_id = TenantId::from_uuid(id);
    let ctx = TenantContext::new(tenant_id, platform.user_id, vec![]);
    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .map_err(ApiError::from)?;
    let tenant = platform::approve(&tx, tenant_id, platform.user_id)
        .await
        .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    let _ = access::invalidate_access_mode(&cache, tenant_id).await;
    Ok(ApiOk(tenant))
}

/// Platform staff suspends a pharmacy. Requires a reason and immediately bites.
#[utoipa::path(
    post, path = "/tenants/{id}/suspend", tag = "platform", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Pharmacy to suspend")),
    request_body = SuspendRequest,
    responses(
        (status = 200, description = "The pharmacy, now suspended — fully blocked, not merely flagged", body = Tenant),
        (status = 403, description = "`permission_denied` — platform token required", body = ErrorBody),
    )
)]
pub async fn suspend_tenant(
    Extension(db): Extension<Db>,
    Extension(platform): Extension<PlatformToken>,
    Extension(cache): Extension<Cache>,
    Path(id): Path<Uuid>,
    Json(body): Json<SuspendRequest>,
) -> Result<ApiOk<Tenant>, ApiError> {
    let tenant_id = TenantId::from_uuid(id);
    let ctx = TenantContext::new(tenant_id, platform.user_id, vec![]);
    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .map_err(ApiError::from)?;
    let tenant = platform::suspend(&tx, tenant_id, body.reason)
        .await
        .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    let _ = access::invalidate_access_mode(&cache, tenant_id).await;
    Ok(ApiOk(tenant))
}

/// Platform staff reinstates a suspended pharmacy to active status.
#[utoipa::path(
    post, path = "/tenants/{id}/reinstate", tag = "platform", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Suspended pharmacy to reinstate")),
    responses(
        (status = 200, description = "The pharmacy, active again", body = Tenant),
        (status = 403, description = "`permission_denied` — platform token required", body = ErrorBody),
    )
)]
pub async fn reinstate_tenant(
    Extension(db): Extension<Db>,
    Extension(platform): Extension<PlatformToken>,
    Extension(cache): Extension<Cache>,
    Path(id): Path<Uuid>,
) -> Result<ApiOk<Tenant>, ApiError> {
    let tenant_id = TenantId::from_uuid(id);
    let ctx = TenantContext::new(tenant_id, platform.user_id, vec![]);
    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .map_err(ApiError::from)?;
    let tenant = platform::reinstate(&tx, tenant_id)
        .await
        .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    let _ = access::invalidate_access_mode(&cache, tenant_id).await;
    Ok(ApiOk(tenant))
}

/// Platform staff creates a subscription for a pharmacy.
#[utoipa::path(
    post, path = "/tenants/{id}/subscriptions", tag = "platform", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Pharmacy the subscription belongs to")),
    request_body = NewSubscription,
    responses(
        (status = 201, description = "The subscription, its price copied at purchase", body = Subscription),
        (status = 409, description = "Overlaps an existing subscription — refused by a database exclusion constraint", body = ErrorBody),
    )
)]
pub async fn create_subscription(
    Extension(db): Extension<Db>,
    Extension(platform): Extension<PlatformToken>,
    Extension(cache): Extension<Cache>,
    Path(id): Path<Uuid>,
    Json(input): Json<NewSubscription>,
) -> Result<ApiCreated<Subscription>, ApiError> {
    let tenant_id = TenantId::from_uuid(id);
    let ctx = TenantContext::new(tenant_id, platform.user_id, vec![]);
    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .map_err(ApiError::from)?;
    let sub = platform::create_subscription(&tx, tenant_id, input)
        .await
        .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    let _ = access::invalidate_access_mode(&cache, tenant_id).await;
    Ok(ApiCreated(sub))
}

/// Platform staff confirms a submitted payment and activates its subscription in one transaction.
#[utoipa::path(
    post, path = "/subscription-payments/{id}/confirm", tag = "platform", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Submitted payment to confirm")),
    responses(
        (status = 200, description = "`{payment, subscription}` — the payment and the subscription it activated, in one transaction", body = serde_json::Value),
        (status = 403, description = "`permission_denied` — platform token required", body = ErrorBody),
    )
)]
pub async fn confirm_payment(
    Extension(db): Extension<Db>,
    Extension(platform): Extension<PlatformToken>,
    Extension(cache): Extension<Cache>,
    Path(id): Path<Uuid>,
) -> Result<ApiOk<serde_json::Value>, ApiError> {
    let tenant_id = TenantId::from_uuid(Uuid::nil());
    let ctx = TenantContext::new(tenant_id, platform.user_id, vec![]);
    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .map_err(ApiError::from)?;
    let (payment, sub) = platform::confirm_payment(&tx, id)
        .await
        .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    let _ = access::invalidate_access_mode(&cache, sub.tenant_id).await;
    Ok(ApiOk(serde_json::json!({
        "payment": payment,
        "subscription": sub
    })))
}

/// Platform staff rejects a submitted payment with a review note.
#[utoipa::path(
    post, path = "/subscription-payments/{id}/reject", tag = "platform", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Submitted payment to reject")),
    request_body = RejectPaymentRequest,
    responses(
        (status = 200, description = "The payment, rejected, with the review note stored", body = Payment),
        (status = 403, description = "`permission_denied` — platform token required", body = ErrorBody),
    )
)]
pub async fn reject_payment(
    Extension(db): Extension<Db>,
    Extension(platform): Extension<PlatformToken>,
    Path(id): Path<Uuid>,
    Json(body): Json<RejectPaymentRequest>,
) -> Result<ApiOk<Payment>, ApiError> {
    let tenant_id = TenantId::from_uuid(Uuid::nil());
    let ctx = TenantContext::new(tenant_id, platform.user_id, vec![]);
    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .map_err(ApiError::from)?;
    let payment = platform::reject_payment(&tx, id, body.note)
        .await
        .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(payment))
}

/// List submitted payments for platform review.
#[utoipa::path(
    get, path = "/subscription-payments", tag = "platform", security(("bearer" = [])),
    params(Pagination),
    responses(
        (status = 200, description = "One page of submitted payments awaiting review", body = ListPage<Payment>),
        (status = 403, description = "`permission_denied` — platform token required", body = ErrorBody),
    )
)]
pub async fn list_payments_for_review(
    Extension(db): Extension<Db>,
    pagination: Pagination,
) -> Result<ApiOk<ListPage<Payment>>, ApiError> {
    let page = queries::payments_for_review(&db, "submitted", pagination)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(page))
}

/// List all plans with prices and limits for platform admin.
#[utoipa::path(
    get, path = "/plans", tag = "platform", security(("bearer" = [])),
    responses(
        (status = 200, description = "All plans", body = ListPage<Plan>),
    )
)]
pub async fn list_admin_plans(
    Extension(db): Extension<Db>,
) -> Result<ApiOk<ListPage<Plan>>, ApiError> {
    let plans = queries::plans(&db).await.map_err(ApiError::from)?;
    Ok(ApiOk(ListPage::new(plans, None)))
}

/// Create a new plan in the catalogue.
#[utoipa::path(
    post, path = "/plans", tag = "platform", security(("bearer" = [])),
    request_body = NewPlanRequest,
    responses(
        (status = 201, description = "Created plan", body = Plan),
        (status = 409, description = "`duplicate_plan_code`", body = ErrorBody),
    )
)]
pub async fn create_plan(
    Extension(db): Extension<Db>,
    Extension(platform): Extension<PlatformToken>,
    Json(input): Json<NewPlanRequest>,
) -> Result<ApiCreated<Plan>, ApiError> {
    let tenant_id = TenantId::from_uuid(Uuid::nil());
    let ctx = TenantContext::new(tenant_id, platform.user_id, vec![]);
    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .map_err(ApiError::from)?;
    let p = platform::create_plan(&tx, input)
        .await
        .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(ApiCreated(p))
}

/// Update an existing plan.
#[utoipa::path(
    patch, path = "/plans/{id}", tag = "platform", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Plan ID")),
    request_body = UpdatePlanRequest,
    responses(
        (status = 200, description = "Updated plan", body = Plan),
        (status = 404, description = "`not_found`", body = ErrorBody),
    )
)]
pub async fn update_plan(
    Extension(db): Extension<Db>,
    Extension(platform): Extension<PlatformToken>,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdatePlanRequest>,
) -> Result<ApiOk<Plan>, ApiError> {
    let tenant_id = TenantId::from_uuid(Uuid::nil());
    let ctx = TenantContext::new(tenant_id, platform.user_id, vec![]);
    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .map_err(ApiError::from)?;
    let p = platform::update_plan(&tx, id, input)
        .await
        .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(p))
}

/// Set a plan limit value.
#[utoipa::path(
    put, path = "/plans/{id}/limits/{limit_key}", tag = "platform", security(("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Plan ID"),
        ("limit_key" = String, Path, description = "Limit key, e.g. branches, users, devices"),
    ),
    request_body = SetPlanLimitRequest,
    responses(
        (status = 200, description = "Set limit", body = PlanLimit),
        (status = 404, description = "`not_found`", body = ErrorBody),
    )
)]
pub async fn set_plan_limit(
    Extension(db): Extension<Db>,
    Extension(platform): Extension<PlatformToken>,
    Path((id, limit_key)): Path<(Uuid, String)>,
    Json(input): Json<SetPlanLimitRequest>,
) -> Result<ApiOk<PlanLimit>, ApiError> {
    let tenant_id = TenantId::from_uuid(Uuid::nil());
    let ctx = TenantContext::new(tenant_id, platform.user_id, vec![]);
    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .map_err(ApiError::from)?;
    let lim = platform::set_plan_limit(&tx, id, limit_key, input.max_value)
        .await
        .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(lim))
}

/// Constructs the router for platform-facing admin routes.
/// All routes are gated by `require_platform_middleware`.
pub fn admin_router() -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_tenants))
        .routes(routes!(get_tenant_detail))
        .routes(routes!(approve_tenant))
        .routes(routes!(suspend_tenant))
        .routes(routes!(reinstate_tenant))
        .routes(routes!(create_subscription))
        .routes(routes!(list_payments_for_review))
        .routes(routes!(confirm_payment))
        .routes(routes!(reject_payment))
        .routes(routes!(list_admin_plans, create_plan))
        .routes(routes!(update_plan))
        .routes(routes!(set_plan_limit))
        .layer(middleware::from_fn(require_platform_middleware))
}
