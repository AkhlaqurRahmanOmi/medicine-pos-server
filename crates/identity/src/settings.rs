//! Token lifetimes, read from configuration rather than written as literals.
//!
//! Module 03's Q1 and Q3 were answered on 2026-10-24 as *configuration, not constants*:
//! a deployment sets how long a refresh token and an impersonation session live. The
//! defaults in [`pos_core::config::AuthConfig`] are the values this crate used to hardcode,
//! so a deployment that sets nothing behaves as before.

use pos_core::config::AuthConfig;

/// Everything the auth surface needs to sign a token and decide when it dies.
///
/// Carried as an axum `Extension` so no handler reaches for a constant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenSettings {
    pub jwt_secret: String,
    pub access_ttl_secs: i64,
    pub refresh_ttl_secs: i64,
    pub impersonation_ttl_secs: i64,
}

impl From<&AuthConfig> for TokenSettings {
    fn from(auth: &AuthConfig) -> Self {
        Self {
            jwt_secret: auth.jwt_secret.clone(),
            access_ttl_secs: auth.access_token_ttl_secs,
            refresh_ttl_secs: auth.refresh_token_ttl_secs,
            impersonation_ttl_secs: auth.impersonation_ttl_secs,
        }
    }
}

impl TokenSettings {
    /// A fixed set for tests, so a test states the lifetimes it is asserting about.
    #[must_use]
    pub fn for_test(jwt_secret: &str) -> Self {
        Self {
            jwt_secret: jwt_secret.to_string(),
            access_ttl_secs: 900,
            refresh_ttl_secs: 30 * 24 * 60 * 60,
            impersonation_ttl_secs: 60 * 60,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn token_settings_are_read_from_auth_config() {
        let auth = AuthConfig {
            jwt_secret: "a-configured-secret".to_string(),
            access_token_ttl_secs: 60,
            refresh_token_ttl_secs: 7 * 24 * 60 * 60,
            impersonation_ttl_secs: 120,
        };

        let settings = TokenSettings::from(&auth);

        assert_eq!(settings.jwt_secret, "a-configured-secret");
        assert_eq!(settings.access_ttl_secs, 60);
        assert_eq!(settings.refresh_ttl_secs, 7 * 24 * 60 * 60);
        assert_eq!(settings.impersonation_ttl_secs, 120);
    }
}
