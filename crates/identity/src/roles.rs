use cache::Cache;
use pos_core::error::AppError;
use pos_core::ids::{BranchId, TenantId, UserId};
use pos_core::ports::{BoxFuture, PermissionSource};
use db::Tx;
use pos_core::context::TenantContext;
use sea_orm::DatabaseConnection;

use crate::queries;

#[derive(Clone)]
pub struct IdentityPermissionSource {
    db: DatabaseConnection,
    cache: Cache,
    tenant_id: TenantId,
}

impl IdentityPermissionSource {
    pub fn new(db: DatabaseConnection, cache: Cache, tenant_id: TenantId) -> Self {
        Self {
            db,
            cache,
            tenant_id,
        }
    }
}

impl IdentityPermissionSource {
    /// A read transaction carrying this source's tenant context.
    ///
    /// Every table behind this port is tenant-owned with forced RLS, so a read on the bare
    /// connection returns an empty set instead of an error — the quiet failure that left
    /// `AuthUser` with no permissions at all.
    async fn read_tx(&self, user: UserId) -> Result<Tx, AppError> {
        let ctx = TenantContext::new(self.tenant_id, user, vec![]);
        Tx::begin(&self.db, &ctx).await
    }
}

impl PermissionSource for IdentityPermissionSource {
    fn permissions_at<'a>(
        &'a self,
        user: UserId,
        branch: Option<BranchId>,
    ) -> BoxFuture<'a, Result<Vec<String>, AppError>> {
        Box::pin(async move {
            let tx = self.read_tx(user).await?;
            let conn = tx.transaction()?;
            let perms = match branch {
                Some(b) => queries::permissions_at(conn, self.tenant_id, user, b).await?,
                // No branch named: the pharmacy-wide grant, which is what `resolve_access`
                // calls `wide` — one definition of it, not two.
                None => {
                    queries::resolve_access(conn, self.tenant_id, user)
                        .await?
                        .wide
                }
            };
            tx.commit().await?;
            Ok(perms)
        })
    }

    fn branch_access<'a>(
        &'a self,
        user: UserId,
    ) -> BoxFuture<'a, Result<Vec<BranchId>, AppError>> {
        Box::pin(async move {
            queries::branch_access_cached(&self.db, &self.cache, self.tenant_id, user).await
        })
    }
}
