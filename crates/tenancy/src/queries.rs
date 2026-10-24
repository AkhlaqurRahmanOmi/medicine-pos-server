use crate::access::AccessMode;
use crate::repo;
use crate::types::*;
use api::envelope::ListPage;
use api::extract::Pagination;
use chrono::{DateTime, Utc};
use db::Db;
use pos_core::error::AppError;
use pos_core::ids::TenantId;

pub async fn access_mode(
    db: &Db,
    tenant: TenantId,
    now: DateTime<Utc>,
) -> Result<AccessMode, AppError> {
    repo::get_access_mode(db, tenant, now).await
}

pub async fn tenant(db: &Db, tenant: TenantId) -> Result<Tenant, AppError> {
    repo::get_tenant(db, tenant).await
}

pub async fn settings(db: &Db, tenant: TenantId) -> Result<TenantSettings, AppError> {
    repo::get_settings(db, tenant).await
}

pub async fn branches(
    db: &Db,
    tenant: TenantId,
    page: Pagination,
) -> Result<ListPage<Branch>, AppError> {
    repo::get_branches(db, tenant, page).await
}

pub async fn limit_remaining(
    db: &Db,
    tenant: TenantId,
    key: &str,
) -> Result<Option<i32>, AppError> {
    repo::get_limit_remaining_query(db, tenant, key).await
}

pub async fn plans(db: &Db) -> Result<Vec<Plan>, AppError> {
    repo::get_plans(db).await
}

pub async fn payments_for_review(
    db: &Db,
    status: &str,
    page: Pagination,
) -> Result<ListPage<Payment>, AppError> {
    repo::payments_for_review(db, status, page).await
}

pub async fn branch(
    db: &Db,
    tenant: TenantId,
    id: pos_core::ids::BranchId,
) -> Result<Branch, AppError> {
    repo::get_branch(db, tenant, id).await
}

pub async fn subscriptions(
    db: &Db,
    tenant: TenantId,
    page: Pagination,
) -> Result<ListPage<Subscription>, AppError> {
    repo::get_subscriptions(db, tenant, page).await
}

pub async fn current_subscription(
    db: &Db,
    tenant: TenantId,
) -> Result<Option<Subscription>, AppError> {
    repo::get_current_subscription(db, tenant).await
}

pub async fn export(
    db: &Db,
    tenant: TenantId,
    id: uuid::Uuid,
) -> Result<TenantExport, AppError> {
    repo::get_export(db, tenant, id).await
}

pub async fn tenants(
    db: &Db,
    status_filter: Option<String>,
    page: Pagination,
) -> Result<ListPage<TenantSummary>, AppError> {
    repo::list_tenants(db, status_filter, page).await
}
