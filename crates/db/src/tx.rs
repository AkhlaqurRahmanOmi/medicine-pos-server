use pos_core::context::TenantContext;
use pos_core::error::{AppError, ErrorCode};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, Statement,
    TransactionTrait, Value,
};

pub struct Tx {
    inner: Option<DatabaseTransaction>,
}

impl Tx {
    pub async fn begin(db: &DatabaseConnection, ctx: &TenantContext) -> Result<Self, AppError> {
        let tx = db.begin().await.map_err(|e| {
            AppError::with_detail(
                ErrorCode::Internal,
                "Failed to begin transaction",
                e.to_string(),
            )
        })?;

        let mut wrapped = Self { inner: Some(tx) };
        if let Err(e) = wrapped.apply_tenant_context(ctx).await {
            let _ = wrapped.rollback().await;
            return Err(e);
        }
        Ok(wrapped)
    }

    pub async fn apply_tenant_context(&mut self, ctx: &TenantContext) -> Result<(), AppError> {
        let stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT set_config('app.tenant_id', $1, true), set_config('app.user_id', $2, true)",
            [
                Value::from(ctx.tenant_id.as_uuid().to_string()),
                Value::from(ctx.user_id.as_uuid().to_string()),
            ],
        );

        if let Some(ref tx) = self.inner {
            tx.execute_raw(stmt).await.map_err(|e| {
                AppError::with_detail(
                    ErrorCode::Internal,
                    "Failed to set tenant context in transaction",
                    e.to_string(),
                )
            })?;
            Ok(())
        } else {
            Err(AppError::internal(
                "Transaction already closed",
                "Transaction inner is None",
            ))
        }
    }

    pub fn transaction(&self) -> Result<&DatabaseTransaction, AppError> {
        self.inner
            .as_ref()
            .ok_or_else(|| AppError::internal("Transaction already consumed", "inner is None"))
    }

    pub async fn commit(mut self) -> Result<(), AppError> {
        if let Some(tx) = self.inner.take() {
            tx.commit().await.map_err(|e| {
                AppError::with_detail(
                    ErrorCode::Internal,
                    "Failed to commit transaction",
                    e.to_string(),
                )
            })?;
        }
        Ok(())
    }

    pub async fn rollback(mut self) -> Result<(), AppError> {
        if let Some(tx) = self.inner.take() {
            tx.rollback().await.map_err(|e| {
                AppError::with_detail(
                    ErrorCode::Internal,
                    "Failed to roll back transaction",
                    e.to_string(),
                )
            })?;
        }
        Ok(())
    }

    pub async fn run<F, Fut, T>(
        db: &DatabaseConnection,
        ctx: &TenantContext,
        f: F,
    ) -> Result<T, AppError>
    where
        F: FnOnce(&Tx) -> Fut,
        Fut: std::future::Future<Output = Result<T, AppError>>,
    {
        let tx = Self::begin(db, ctx).await?;
        match f(&tx).await {
            Ok(val) => {
                tx.commit().await?;
                Ok(val)
            }
            Err(err) => {
                let _ = tx.rollback().await;
                Err(err)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pos_core::ids::{BranchId, TenantId, UserId};
    use sea_orm::{DbBackend, MockDatabase, MockExecResult};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn tx_sets_tenant_id_from_bound_parameter() {
        let tenant_id = TenantId::new_v7();
        let user_id = UserId::new_v7();
        let branch_id = BranchId::new_v7();
        let ctx = TenantContext::new(tenant_id, user_id, vec![branch_id]);

        let db = MockDatabase::new(DbBackend::Postgres)
            .append_exec_results([MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();

        let tx = Tx::begin(&db, &ctx).await.expect("begin should succeed");
        tx.commit().await.expect("commit should succeed");

        let log = db.into_transaction_log();
        let was_called = log.iter().any(|item| {
            format!("{item:?}").contains("app.tenant_id")
                && format!("{item:?}").contains(&tenant_id.as_uuid().to_string())
        });
        assert!(
            was_called,
            "Transaction must set app.tenant_id with bound parameter"
        );
    }

    #[tokio::test]
    async fn tx_setting_is_transaction_local() {
        // Assert that the SQL text uses true as the third argument to make it transaction-local
        let src = include_str!("tx.rs");
        assert!(
            src.contains("set_config('app.tenant_id', $1, true)"),
            "set_config must specify true for transaction-local lifetime"
        );
    }

    #[tokio::test]
    async fn tx_aborts_when_setting_fails() {
        let tenant_id = TenantId::new_v7();
        let user_id = UserId::new_v7();
        let ctx = TenantContext::new(tenant_id, user_id, vec![]);

        // Mock database with empty exec results so execute fails
        let db = MockDatabase::new(DbBackend::Postgres).into_connection();

        let tx_result = Tx::begin(&db, &ctx).await;
        assert!(
            tx_result.is_err(),
            "Transaction must abort when setting fails"
        );
    }

    #[tokio::test]
    async fn tx_commits_on_ok_and_rolls_back_on_err() {
        let tenant_id = TenantId::new_v7();
        let user_id = UserId::new_v7();
        let ctx = TenantContext::new(tenant_id, user_id, vec![]);

        // 1. Success case: commits on Ok
        let db_ok = MockDatabase::new(DbBackend::Postgres)
            .append_exec_results([MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();

        let executed_ok = Arc::new(AtomicBool::new(false));
        let exec_clone = Arc::clone(&executed_ok);
        let res_ok = Tx::run(&db_ok, &ctx, |_tx| async move {
            exec_clone.store(true, Ordering::SeqCst);
            Ok::<_, AppError>(42)
        })
        .await;

        assert_eq!(res_ok.unwrap(), 42);
        assert!(executed_ok.load(Ordering::SeqCst));

        // 2. Error case: rolls back on Err
        let db_err = MockDatabase::new(DbBackend::Postgres)
            .append_exec_results([MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();

        let res_err = Tx::run(&db_err, &ctx, |_tx| async move {
            Err::<(), AppError>(AppError::bad_request("Rollback me"))
        })
        .await;

        assert!(res_err.is_err());
    }
}
