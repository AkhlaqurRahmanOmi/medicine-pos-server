use pos_core::error::{AppError, ErrorCode};
use pos_core::ports::MailSender;

#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from_address: String,
}

#[derive(Debug, Clone)]
pub struct SmtpMailSender {
    config: SmtpConfig,
}

impl SmtpMailSender {
    pub fn from_config(config: &SmtpConfig) -> Result<Self, AppError> {
        if config.host.trim().is_empty() {
            return Err(AppError::new(
                ErrorCode::Internal,
                "SMTP host cannot be empty",
            ));
        }
        if config.from_address.trim().is_empty() {
            return Err(AppError::new(
                ErrorCode::Internal,
                "SMTP from_address cannot be empty",
            ));
        }
        Ok(Self {
            config: config.clone(),
        })
    }

    #[must_use]
    pub fn config(&self) -> &SmtpConfig {
        &self.config
    }
}

impl MailSender for SmtpMailSender {
    async fn send(&self, _to: &str, _subject: &str, _body: &str) -> Result<(), AppError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smtp_sender_builds_from_config() {
        let config = SmtpConfig {
            host: "smtp.mailgun.org".into(),
            port: 587,
            username: Some("pos_mailer".into()),
            password: Some("secretpass".into()),
            from_address: "no-reply@medicinepos.com".into(),
        };

        let sender = SmtpMailSender::from_config(&config);
        assert!(
            sender.is_ok(),
            "SmtpMailSender must build from valid config"
        );
        assert_eq!(sender.unwrap().config().port, 587);
    }
}
