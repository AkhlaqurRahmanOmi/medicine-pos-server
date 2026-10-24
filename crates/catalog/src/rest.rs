use api::{
    ApiCreated, ApiError, ApiOk, ErrorBody, ListPage, Pagination, PlatformToken, TenantTx,
};
use axum::{
    extract::{Extension, Json, Path, Query},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
};
use db::{Db, Tx};
use pos_core::context::TenantContext;
use pos_core::error::AppError;
use pos_core::ids::{CompanyId, GenericId, IssueReportId, ProductId, TenantId, UserId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::perms;
use crate::types::{
    Company, Generic, IssueReport, IssueReportStatus, Lookups, Monograph, MonographInput,
    NewCompany, NewGeneric, NewIssueReport, NewProduct, Product, ProductSummary, ProductType,
    QueueItem, RecordKind, RecordStatus, Reviewed, Submission,
};
use crate::{commands, platform, queries};

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RejectSubmissionRequest {
    pub note: String,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct MergeSubmissionRequest {
    pub into: Uuid,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ResolveIssueRequest {
    pub note: String,
}

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct SearchQuery {
    pub q: String,
}

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct ProductsFilter {
    pub company_id: Option<Uuid>,
    pub generic_id: Option<Uuid>,
    pub product_type: Option<ProductType>,
}

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct QueueFilter {
    pub kind: Option<RecordKind>,
    pub status: Option<RecordStatus>,
}

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct AdminIssuesFilter {
    pub status: Option<IssueReportStatus>,
}

// =============================================================================
// Pharmacy Handlers
// =============================================================================

/// Search products by brand or generic name using trigram similarity.
#[utoipa::path(
    get, path = "/catalog/search", tag = "catalog", security(("bearer" = [])),
    params(SearchQuery, Pagination),
    responses(
        (status = 200, description = "Matching products", body = ListPage<ProductSummary>),
        (status = 400, description = "Empty query", body = ErrorBody),
    )
)]
pub async fn search_products(
    Extension(ctx): Extension<TenantContext>,
    Extension(db): Extension<Db>,
    Query(search): Query<SearchQuery>,
    pagination: Pagination,
) -> Result<ApiOk<ListPage<ProductSummary>>, ApiError> {
    if search.q.trim().is_empty() {
        return Err(ApiError::from(AppError::bad_request("Query cannot be empty")));
    }
    let res = queries::search(&db, ctx.tenant_id, &search.q, pagination)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(res))
}

/// List products with optional filtering.
#[utoipa::path(
    get, path = "/catalog/products", tag = "catalog", security(("bearer" = [])),
    params(ProductsFilter, Pagination),
    responses(
        (status = 200, description = "Page of products", body = ListPage<ProductSummary>),
    )
)]
pub async fn list_products(
    Extension(ctx): Extension<TenantContext>,
    Extension(db): Extension<Db>,
    Query(filter): Query<ProductsFilter>,
    pagination: Pagination,
) -> Result<ApiOk<ListPage<ProductSummary>>, ApiError> {
    let res = queries::products(
        &db,
        ctx.tenant_id,
        filter.company_id.map(CompanyId::from_uuid),
        filter.generic_id.map(GenericId::from_uuid),
        filter.product_type,
        pagination,
    )
    .await
    .map_err(ApiError::from)?;
    Ok(ApiOk(res))
}

/// Get a product by ID.
#[utoipa::path(
    get, path = "/catalog/products/{id}", tag = "catalog", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Product ID")),
    responses(
        (status = 200, description = "Product details", body = Product),
        (status = 404, description = "Product not found", body = ErrorBody),
    )
)]
pub async fn get_product(
    Extension(ctx): Extension<TenantContext>,
    Extension(db): Extension<Db>,
    Path(id): Path<Uuid>,
) -> Result<ApiOk<Product>, ApiError> {
    let res = queries::product(&db, ctx.tenant_id, ProductId::from_uuid(id))
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(res))
}

/// Get generic details by ID.
#[utoipa::path(
    get, path = "/catalog/generics/{id}", tag = "catalog", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Generic ID")),
    responses(
        (status = 200, description = "Generic details", body = Generic),
        (status = 404, description = "Generic not found", body = ErrorBody),
    )
)]
pub async fn get_generic(
    Extension(ctx): Extension<TenantContext>,
    Extension(db): Extension<Db>,
    Path(id): Path<Uuid>,
) -> Result<ApiOk<Generic>, ApiError> {
    let res = queries::generic(&db, ctx.tenant_id, GenericId::from_uuid(id))
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(res))
}

/// Get monograph text for a generic, or 204 if none.
#[utoipa::path(
    get, path = "/catalog/generics/{id}/monograph", tag = "catalog", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Generic ID")),
    responses(
        (status = 200, description = "Monograph text", body = Monograph),
        (status = 204, description = "Generic has no monograph text"),
        (status = 404, description = "Generic not found", body = ErrorBody),
    )
)]
pub async fn get_monograph(
    Extension(db): Extension<Db>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    match queries::monograph(&db, GenericId::from_uuid(id)).await.map_err(ApiError::from)? {
        Some(m) => Ok((StatusCode::OK, Json(m)).into_response()),
        None => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}

/// List approved and pending companies.
#[utoipa::path(
    get, path = "/catalog/companies", tag = "catalog", security(("bearer" = [])),
    params(Pagination),
    responses(
        (status = 200, description = "Page of companies", body = ListPage<Company>),
    )
)]
pub async fn list_companies(
    Extension(ctx): Extension<TenantContext>,
    Extension(db): Extension<Db>,
    pagination: Pagination,
) -> Result<ApiOk<ListPage<Company>>, ApiError> {
    let res = queries::companies(&db, ctx.tenant_id, pagination)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(res))
}

/// Get static lookup tables (dosage forms, pack units, therapeutic classes).
#[utoipa::path(
    get, path = "/catalog/lookups", tag = "catalog", security(("bearer" = [])),
    responses(
        (status = 200, description = "All lookup tables", body = Lookups),
    )
)]
pub async fn get_lookups(
    Extension(db): Extension<Db>,
) -> Result<ApiOk<Lookups>, ApiError> {
    let res = queries::lookups(&db).await.map_err(ApiError::from)?;
    Ok(ApiOk(res))
}

/// List submissions filed by the authenticated tenant.
#[utoipa::path(
    get, path = "/catalog/submissions", tag = "catalog", security(("bearer" = [])),
    params(Pagination),
    responses(
        (status = 200, description = "Submissions list", body = ListPage<Submission>),
    )
)]
pub async fn list_submissions(
    Extension(ctx): Extension<TenantContext>,
    Extension(db): Extension<Db>,
    pagination: Pagination,
) -> Result<ApiOk<ListPage<Submission>>, ApiError> {
    let res = queries::submissions(&db, ctx.tenant_id, pagination)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(res))
}

/// List issue reports filed by the authenticated tenant.
#[utoipa::path(
    get, path = "/catalog/issues", tag = "catalog", security(("bearer" = [])),
    params(Pagination),
    responses(
        (status = 200, description = "Issue reports list", body = ListPage<IssueReport>),
    )
)]
pub async fn list_issues(
    Extension(ctx): Extension<TenantContext>,
    Extension(db): Extension<Db>,
    pagination: Pagination,
) -> Result<ApiOk<ListPage<IssueReport>>, ApiError> {
    let res = queries::issues(&db, ctx.tenant_id, pagination)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(res))
}

/// Submit a new company.
#[utoipa::path(
    post, path = "/catalog/companies", tag = "catalog", security(("bearer" = [])),
    request_body = NewCompany,
    responses(
        (status = 201, description = "Company submitted", body = Company),
        (status = 409, description = "Duplicate company", body = ErrorBody),
    )
)]
pub async fn submit_company(
    tx: TenantTx,
    Json(input): Json<NewCompany>,
) -> Result<ApiCreated<Company>, ApiError> {
    let res = commands::submit_company(&tx.0, input)
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiCreated(res))
}

/// Submit a new generic with its ingredients and therapeutic classes.
#[utoipa::path(
    post, path = "/catalog/generics", tag = "catalog", security(("bearer" = [])),
    request_body = NewGeneric,
    responses(
        (status = 201, description = "Generic submitted", body = Generic),
        (status = 409, description = "Duplicate generic", body = ErrorBody),
    )
)]
pub async fn submit_generic(
    tx: TenantTx,
    Json(input): Json<NewGeneric>,
) -> Result<ApiCreated<Generic>, ApiError> {
    let res = commands::submit_generic(&tx.0, input)
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiCreated(res))
}

/// Submit a new product.
#[utoipa::path(
    post, path = "/catalog/products", tag = "catalog", security(("bearer" = [])),
    request_body = NewProduct,
    responses(
        (status = 201, description = "Product submitted", body = Product),
        (status = 409, description = "Duplicate product", body = ErrorBody),
    )
)]
pub async fn submit_product(
    tx: TenantTx,
    Json(input): Json<NewProduct>,
) -> Result<ApiCreated<Product>, ApiError> {
    let res = commands::submit_product(&tx.0, input)
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiCreated(res))
}

/// Report an issue against a catalog row.
#[utoipa::path(
    post, path = "/catalog/issues", tag = "catalog", security(("bearer" = [])),
    request_body = NewIssueReport,
    responses(
        (status = 201, description = "Issue reported", body = IssueReport),
        (status = 400, description = "Invalid issue target", body = ErrorBody),
    )
)]
pub async fn report_issue(
    tx: TenantTx,
    Json(input): Json<NewIssueReport>,
) -> Result<ApiCreated<IssueReport>, ApiError> {
    let res = commands::report_issue(&tx.0, input)
        .await
        .map_err(ApiError::from)?;
    tx.0.commit().await.map_err(ApiError::from)?;
    Ok(ApiCreated(res))
}

// =============================================================================
// Platform Admin Handlers
// =============================================================================

/// List pending catalog review queue items with near-duplicate suggestions.
#[utoipa::path(
    get, path = "/catalog/queue", tag = "catalog-admin", security(("bearer" = [])),
    params(QueueFilter, Pagination),
    responses(
        (status = 200, description = "Review queue", body = ListPage<QueueItem>),
    )
)]
pub async fn list_admin_queue(
    Extension(db): Extension<Db>,
    Query(filter): Query<QueueFilter>,
    pagination: Pagination,
) -> Result<ApiOk<ListPage<QueueItem>>, ApiError> {
    let res = queries::queue(&db, filter.kind, filter.status, pagination)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(res))
}

/// List all issue reports across all pharmacies.
#[utoipa::path(
    get, path = "/catalog/issues", tag = "catalog-admin", security(("bearer" = [])),
    params(AdminIssuesFilter, Pagination),
    responses(
        (status = 200, description = "All issue reports", body = ListPage<IssueReport>),
    )
)]
pub async fn list_admin_issues(
    Extension(db): Extension<Db>,
    Query(filter): Query<AdminIssuesFilter>,
    pagination: Pagination,
) -> Result<ApiOk<ListPage<IssueReport>>, ApiError> {
    let res = queries::admin_issues(&db, filter.status, pagination)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiOk(res))
}

// Specific review handlers for each record kind
pub async fn approve_company_handler(tx: &Tx, id: Uuid, by: UserId) -> Result<Reviewed, AppError> {
    platform::approve(tx, RecordKind::Company, id, by).await
}

pub async fn approve_generic_handler(tx: &Tx, id: Uuid, by: UserId) -> Result<Reviewed, AppError> {
    platform::approve(tx, RecordKind::Generic, id, by).await
}

pub async fn approve_product_handler(tx: &Tx, id: Uuid, by: UserId) -> Result<Reviewed, AppError> {
    platform::approve(tx, RecordKind::Product, id, by).await
}

pub async fn reject_company_handler(tx: &Tx, id: Uuid, by: UserId, note: String) -> Result<Reviewed, AppError> {
    platform::reject(tx, RecordKind::Company, id, by, note).await
}

pub async fn reject_generic_handler(tx: &Tx, id: Uuid, by: UserId, note: String) -> Result<Reviewed, AppError> {
    platform::reject(tx, RecordKind::Generic, id, by, note).await
}

pub async fn reject_product_handler(tx: &Tx, id: Uuid, by: UserId, note: String) -> Result<Reviewed, AppError> {
    platform::reject(tx, RecordKind::Product, id, by, note).await
}

pub async fn merge_company_handler(tx: &Tx, id: Uuid, into: Uuid, by: UserId) -> Result<Reviewed, AppError> {
    platform::merge(tx, RecordKind::Company, id, into, by).await
}

pub async fn merge_generic_handler(tx: &Tx, id: Uuid, into: Uuid, by: UserId) -> Result<Reviewed, AppError> {
    platform::merge(tx, RecordKind::Generic, id, into, by).await
}

pub async fn merge_product_handler(tx: &Tx, id: Uuid, into: Uuid, by: UserId) -> Result<Reviewed, AppError> {
    platform::merge(tx, RecordKind::Product, id, into, by).await
}

/// Approve a pending submission.
#[utoipa::path(
    post, path = "/catalog/{kind}/{id}/approve", tag = "catalog-admin", security(("bearer" = [])),
    params(
        ("kind" = RecordKind, Path, description = "Record kind"),
        ("id" = Uuid, Path, description = "Record ID"),
    ),
    responses(
        (status = 200, description = "Approved row", body = Reviewed),
        (status = 409, description = "Already reviewed", body = ErrorBody),
    )
)]
pub async fn approve_submission(
    Extension(db): Extension<Db>,
    Extension(platform_token): Extension<PlatformToken>,
    Path((kind, id)): Path<(RecordKind, Uuid)>,
) -> Result<ApiOk<Reviewed>, ApiError> {
    let ctx = TenantContext::new(TenantId::from_uuid(Uuid::nil()), platform_token.user_id, vec![]);
    let tx = Tx::begin_platform(db.conn(), &ctx).await.map_err(ApiError::from)?;
    let reviewed = match kind {
        RecordKind::Company => approve_company_handler(&tx, id, platform_token.user_id).await,
        RecordKind::Generic => approve_generic_handler(&tx, id, platform_token.user_id).await,
        RecordKind::Product => approve_product_handler(&tx, id, platform_token.user_id).await,
    }
    .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(reviewed))
}

/// Reject a pending submission with a review note.
#[utoipa::path(
    post, path = "/catalog/{kind}/{id}/reject", tag = "catalog-admin", security(("bearer" = [])),
    params(
        ("kind" = RecordKind, Path, description = "Record kind"),
        ("id" = Uuid, Path, description = "Record ID"),
    ),
    request_body = RejectSubmissionRequest,
    responses(
        (status = 200, description = "Rejected row", body = Reviewed),
        (status = 400, description = "Missing note", body = ErrorBody),
        (status = 409, description = "Already reviewed", body = ErrorBody),
    )
)]
pub async fn reject_submission(
    Extension(db): Extension<Db>,
    Extension(platform_token): Extension<PlatformToken>,
    Path((kind, id)): Path<(RecordKind, Uuid)>,
    Json(input): Json<RejectSubmissionRequest>,
) -> Result<ApiOk<Reviewed>, ApiError> {
    let ctx = TenantContext::new(TenantId::from_uuid(Uuid::nil()), platform_token.user_id, vec![]);
    let tx = Tx::begin_platform(db.conn(), &ctx).await.map_err(ApiError::from)?;
    let reviewed = match kind {
        RecordKind::Company => reject_company_handler(&tx, id, platform_token.user_id, input.note).await,
        RecordKind::Generic => reject_generic_handler(&tx, id, platform_token.user_id, input.note).await,
        RecordKind::Product => reject_product_handler(&tx, id, platform_token.user_id, input.note).await,
    }
    .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(reviewed))
}

/// Merge a pending submission into an active target.
#[utoipa::path(
    post, path = "/catalog/{kind}/{id}/merge", tag = "catalog-admin", security(("bearer" = [])),
    params(
        ("kind" = RecordKind, Path, description = "Record kind"),
        ("id" = Uuid, Path, description = "Record ID"),
    ),
    request_body = MergeSubmissionRequest,
    responses(
        (status = 200, description = "Merged row", body = Reviewed),
        (status = 409, description = "Invalid merge target", body = ErrorBody),
    )
)]
pub async fn merge_submission(
    Extension(db): Extension<Db>,
    Extension(platform_token): Extension<PlatformToken>,
    Path((kind, id)): Path<(RecordKind, Uuid)>,
    Json(input): Json<MergeSubmissionRequest>,
) -> Result<ApiOk<Reviewed>, ApiError> {
    let ctx = TenantContext::new(TenantId::from_uuid(Uuid::nil()), platform_token.user_id, vec![]);
    let tx = Tx::begin_platform(db.conn(), &ctx).await.map_err(ApiError::from)?;
    let reviewed = match kind {
        RecordKind::Company => merge_company_handler(&tx, id, input.into, platform_token.user_id).await,
        RecordKind::Generic => merge_generic_handler(&tx, id, input.into, platform_token.user_id).await,
        RecordKind::Product => merge_product_handler(&tx, id, input.into, platform_token.user_id).await,
    }
    .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(reviewed))
}

/// Upsert a monograph for a generic.
#[utoipa::path(
    put, path = "/catalog/generics/{id}/monograph", tag = "catalog-admin", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Generic ID")),
    request_body = MonographInput,
    responses(
        (status = 200, description = "Monograph updated", body = Monograph),
        (status = 404, description = "Generic not found", body = ErrorBody),
    )
)]
pub async fn put_monograph(
    Extension(db): Extension<Db>,
    Extension(platform_token): Extension<PlatformToken>,
    Path(id): Path<Uuid>,
    Json(input): Json<MonographInput>,
) -> Result<ApiOk<Monograph>, ApiError> {
    let ctx = TenantContext::new(TenantId::from_uuid(Uuid::nil()), platform_token.user_id, vec![]);
    let tx = Tx::begin_platform(db.conn(), &ctx).await.map_err(ApiError::from)?;
    let monograph = platform::upsert_monograph(&tx, GenericId::from_uuid(id), input)
        .await
        .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(monograph))
}

/// Resolve an open issue report.
#[utoipa::path(
    post, path = "/catalog/issues/{id}/resolve", tag = "catalog-admin", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Issue Report ID")),
    request_body = ResolveIssueRequest,
    responses(
        (status = 200, description = "Issue resolved", body = IssueReport),
        (status = 409, description = "Already closed", body = ErrorBody),
    )
)]
pub async fn resolve_issue(
    Extension(db): Extension<Db>,
    Extension(platform_token): Extension<PlatformToken>,
    Path(id): Path<Uuid>,
    Json(input): Json<ResolveIssueRequest>,
) -> Result<ApiOk<IssueReport>, ApiError> {
    let ctx = TenantContext::new(TenantId::from_uuid(Uuid::nil()), platform_token.user_id, vec![]);
    let tx = Tx::begin_platform(db.conn(), &ctx).await.map_err(ApiError::from)?;
    let report = platform::resolve_issue(&tx, IssueReportId::from_uuid(id), platform_token.user_id, input.note)
        .await
        .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(report))
}

/// Dismiss an open issue report.
#[utoipa::path(
    post, path = "/catalog/issues/{id}/dismiss", tag = "catalog-admin", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Issue Report ID")),
    request_body = ResolveIssueRequest,
    responses(
        (status = 200, description = "Issue dismissed", body = IssueReport),
        (status = 409, description = "Already closed", body = ErrorBody),
    )
)]
pub async fn dismiss_issue(
    Extension(db): Extension<Db>,
    Extension(platform_token): Extension<PlatformToken>,
    Path(id): Path<Uuid>,
    Json(input): Json<ResolveIssueRequest>,
) -> Result<ApiOk<IssueReport>, ApiError> {
    let ctx = TenantContext::new(TenantId::from_uuid(Uuid::nil()), platform_token.user_id, vec![]);
    let tx = Tx::begin_platform(db.conn(), &ctx).await.map_err(ApiError::from)?;
    let report = platform::dismiss_issue(&tx, IssueReportId::from_uuid(id), platform_token.user_id, input.note)
        .await
        .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(ApiOk(report))
}

// =============================================================================
// Router Assembly
// =============================================================================

pub fn pharmacy_router() -> OpenApiRouter {
    let view_routes = OpenApiRouter::new()
        .routes(routes!(search_products))
        .routes(routes!(list_products))
        .routes(routes!(get_product))
        .routes(routes!(get_generic))
        .routes(routes!(get_monograph))
        .routes(routes!(list_companies))
        .routes(routes!(get_lookups))
        .route_layer(middleware::from_fn(api::permission::require(perms::VIEW)));

    let submit_routes = OpenApiRouter::new()
        .routes(routes!(submit_company))
        .routes(routes!(submit_generic))
        .routes(routes!(submit_product))
        .routes(routes!(list_submissions))
        .route_layer(middleware::from_fn(api::permission::require(perms::SUBMIT)));

    let issue_routes = OpenApiRouter::new()
        .routes(routes!(report_issue))
        .routes(routes!(list_issues))
        .route_layer(middleware::from_fn(api::permission::require(perms::ISSUE_REPORT)));

    OpenApiRouter::new()
        .merge(view_routes)
        .merge(submit_routes)
        .merge(issue_routes)
}

pub fn router() -> OpenApiRouter {
    pharmacy_router()
}

pub fn admin_router() -> OpenApiRouter {
    let review_routes = OpenApiRouter::new()
        .routes(routes!(list_admin_queue))
        .routes(routes!(list_admin_issues))
        .routes(routes!(approve_submission))
        .routes(routes!(reject_submission))
        .routes(routes!(merge_submission))
        .routes(routes!(put_monograph))
        .routes(routes!(resolve_issue))
        .routes(routes!(dismiss_issue))
        .route_layer(middleware::from_fn(api::permission::require(perms::REVIEW)));

    OpenApiRouter::new().merge(review_routes)
}
