use crate::types::{
    Company, Generic, IssueReport, IssueReportStatus, Lookups, Monograph, Product, ProductSummary,
    ProductType, QueueItem, RecordKind, RecordStatus, Submission,
};
use api::envelope::ListPage;
use api::extract::Pagination;
use db::Db;
use pos_core::error::AppError;
use pos_core::ids::{CompanyId, GenericId, ProductId, TenantId};

pub async fn search(
    db: &Db,
    tenant: TenantId,
    q: &str,
    page: Pagination,
) -> Result<ListPage<ProductSummary>, AppError> {
    crate::repo::search(db, tenant, q, page).await
}

pub async fn products(
    db: &Db,
    tenant: TenantId,
    company_id: Option<CompanyId>,
    generic_id: Option<GenericId>,
    product_type: Option<ProductType>,
    page: Pagination,
) -> Result<ListPage<ProductSummary>, AppError> {
    crate::repo::products(db, tenant, company_id, generic_id, product_type, page).await
}

pub async fn product(db: &Db, tenant: TenantId, id: ProductId) -> Result<Product, AppError> {
    crate::repo::product(db, tenant, id).await
}

pub async fn generic(db: &Db, tenant: TenantId, id: GenericId) -> Result<Generic, AppError> {
    crate::repo::generic(db, tenant, id).await
}

pub async fn monograph(db: &Db, id: GenericId) -> Result<Option<Monograph>, AppError> {
    crate::repo::monograph(db, id).await
}

pub async fn companies(
    db: &Db,
    tenant: TenantId,
    page: Pagination,
) -> Result<ListPage<Company>, AppError> {
    crate::repo::companies(db, tenant, page).await
}

pub async fn lookups(db: &Db) -> Result<Lookups, AppError> {
    crate::repo::lookups(db).await
}

pub async fn submissions(
    db: &Db,
    tenant: TenantId,
    page: Pagination,
) -> Result<ListPage<Submission>, AppError> {
    crate::repo::submissions(db, tenant, page).await
}

pub async fn queue(
    db: &Db,
    kind: Option<RecordKind>,
    status: Option<RecordStatus>,
    page: Pagination,
) -> Result<ListPage<QueueItem>, AppError> {
    crate::repo::queue(db, kind, status, page).await
}

pub async fn issues(
    db: &Db,
    tenant: TenantId,
    page: Pagination,
) -> Result<ListPage<IssueReport>, AppError> {
    crate::repo::issues(db, tenant, page).await
}

pub async fn admin_issues(
    db: &Db,
    status: Option<IssueReportStatus>,
    page: Pagination,
) -> Result<ListPage<IssueReport>, AppError> {
    crate::repo::admin_issues(db, status, page).await
}
