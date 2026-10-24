use db::Db;
use pos_core::error::AppError;
use pos_core::ids::{ProductId, TenantId};

pub async fn is_sellable(db: &Db, tenant: TenantId, id: ProductId) -> Result<bool, AppError> {
    crate::repo::is_sellable(db, tenant, id).await
}
