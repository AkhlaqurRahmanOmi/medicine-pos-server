use crate::error::AppError;
use crate::ids::{BranchId, TenantId, UserId};
use std::future::Future;
use std::pin::Pin;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait MailSender: Send + Sync {
    fn send(
        &self,
        to: &str,
        subject: &str,
        body: &str,
    ) -> impl std::future::Future<Output = Result<(), AppError>> + Send;
}

pub trait FileStorage: Send + Sync {
    fn put(
        &self,
        path: &str,
        data: &[u8],
        content_type: &str,
    ) -> impl std::future::Future<Output = Result<String, AppError>> + Send;

    fn get(
        &self,
        path: &str,
    ) -> impl std::future::Future<Output = Result<Vec<u8>, AppError>> + Send;
}

pub trait AccessModeSource: Send + Sync {
    fn check_access<'a>(
        &'a self,
        tenant_id: TenantId,
        is_write: bool,
    ) -> BoxFuture<'a, Result<(), AppError>>;
}

pub trait PermissionSource: Send + Sync {
    fn permissions_at<'a>(
        &'a self,
        user: UserId,
        branch: Option<BranchId>,
    ) -> BoxFuture<'a, Result<Vec<String>, AppError>>;

    fn branch_access<'a>(
        &'a self,
        user: UserId,
    ) -> BoxFuture<'a, Result<Vec<BranchId>, AppError>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyMail;
    impl MailSender for DummyMail {
        async fn send(&self, _to: &str, _subject: &str, _body: &str) -> Result<(), AppError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn mail_trait_works() {
        let mail = DummyMail;
        assert!(mail.send("test@example.com", "sub", "body").await.is_ok());
    }

    #[tokio::test]
    async fn access_mode_source_is_object_safe() {
        struct DummySource;
        impl AccessModeSource for DummySource {
            fn check_access<'a>(
                &'a self,
                _tenant_id: TenantId,
                _is_write: bool,
            ) -> BoxFuture<'a, Result<(), AppError>> {
                Box::pin(async { Ok(()) })
            }
        }

        let source: Box<dyn AccessModeSource> = Box::new(DummySource);
        let tid = TenantId::new_v7();
        assert!(source.check_access(tid, false).await.is_ok());
        assert!(source.check_access(tid, true).await.is_ok());
    }

    #[tokio::test]
    async fn permission_source_port_is_object_safe() {
        struct DummyPermissionSource;
        impl PermissionSource for DummyPermissionSource {
            fn permissions_at<'a>(
                &'a self,
                _user: UserId,
                _branch: Option<BranchId>,
            ) -> BoxFuture<'a, Result<Vec<String>, AppError>> {
                Box::pin(async { Ok(vec!["role.view".to_string()]) })
            }

            fn branch_access<'a>(
                &'a self,
                _user: UserId,
            ) -> BoxFuture<'a, Result<Vec<BranchId>, AppError>> {
                Box::pin(async { Ok(vec![BranchId::new_v7()]) })
            }
        }

        let source: Box<dyn PermissionSource> = Box::new(DummyPermissionSource);
        let uid = UserId::new_v7();
        let bid = BranchId::new_v7();
        assert!(source.permissions_at(uid, Some(bid)).await.is_ok());
        assert!(source.branch_access(uid).await.is_ok());
    }
}
