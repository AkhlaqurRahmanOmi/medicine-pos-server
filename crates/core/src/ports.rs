use crate::error::AppError;

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
}
