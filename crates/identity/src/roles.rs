use cache::Cache;
use pos_core::error::AppError;
use pos_core::ids::{BranchId, TenantId, UserId};
use pos_core::ports::{BoxFuture, PermissionSource};
use sea_orm::{ConnectionTrait, DatabaseConnection};

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

impl PermissionSource for IdentityPermissionSource {
    fn permissions_at<'a>(
        &'a self,
        user: UserId,
        branch: Option<BranchId>,
    ) -> BoxFuture<'a, Result<Vec<String>, AppError>> {
        Box::pin(async move {
            match branch {
                Some(b) => queries::permissions_at(&self.db, self.tenant_id, user, b).await,
                None => {
                    // Pharmacy-wide permissions (branch_id IS NULL)
                    let stmt = sea_orm::Statement::from_sql_and_values(
                        sea_orm::DbBackend::Postgres,
                        r#"SELECT DISTINCT rp.permission_code
                           FROM user_branch_roles ubr
                           JOIN role_permissions rp ON ubr.role_id = rp.role_id AND ubr.tenant_id = rp.tenant_id
                           WHERE ubr.tenant_id = $1 AND ubr.user_id = $2 AND ubr.branch_id IS NULL
                           ORDER BY rp.permission_code"#,
                        [self.tenant_id.as_uuid().into(), user.as_uuid().into()],
                    );
                    let rows = self
                        .db
                        .query_all_raw(stmt)
                        .await
                        .map_err(|e| AppError::internal("DB query error", e.to_string()))?;
                    let mut perms = Vec::with_capacity(rows.len());
                    for row in rows {
                        let code: String = row
                            .try_get_by_index(0)
                            .map_err(|e| AppError::internal("DB error", e.to_string()))?;
                        perms.push(code);
                    }
                    Ok(perms)
                }
            }
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
