use figment::{
    providers::{Env, Format, Toml},
    Figment,
};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, ErrorCode};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Owner connection URL (used for migrations only).
    pub url: String,
    /// Non-owner application connection URL (used for running server and queries).
    pub app_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    pub port: u16,
    pub env: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedisConfig {
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthConfig {
    pub jwt_secret: String,
    /// How long an access token is accepted for. `[D-75]`'s "about 15 minutes".
    #[serde(default = "default_access_token_ttl_secs")]
    pub access_token_ttl_secs: i64,
    /// How long a refresh token — and so a row in `sessions` — survives.
    /// Module 03 Q1, answered 2026-10-24: configuration, not a constant.
    #[serde(default = "default_refresh_token_ttl_secs")]
    pub refresh_token_ttl_secs: i64,
    /// How long a platform impersonation session stays open.
    /// Module 03 Q3, answered 2026-10-24: configuration, not a constant.
    #[serde(default = "default_impersonation_ttl_secs")]
    pub impersonation_ttl_secs: i64,
}

const fn default_access_token_ttl_secs() -> i64 {
    900
}

const fn default_refresh_token_ttl_secs() -> i64 {
    30 * 24 * 60 * 60
}

const fn default_impersonation_ttl_secs() -> i64 {
    60 * 60
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub redis: RedisConfig,
    pub auth: AuthConfig,
}

impl Config {
    #[must_use]
    pub fn figment() -> Figment {
        Figment::new()
            .merge(Toml::file("Config.toml").nested())
            .merge(
                Env::raw()
                    .only(&[
                        "APP_PORT",
                        "APP_ENV",
                        "DATABASE_URL",
                        "APP_DATABASE_URL",
                        "REDIS_URL",
                        "JWT_SECRET",
                        "ACCESS_TOKEN_TTL_SECS",
                        "REFRESH_TOKEN_TTL_SECS",
                        "IMPERSONATION_TTL_SECS",
                    ])
                    .map(|k| match k.as_str() {
                        "APP_PORT" => "server.port".into(),
                        "APP_ENV" => "server.env".into(),
                        "DATABASE_URL" => "database.url".into(),
                        "APP_DATABASE_URL" => "database.app_url".into(),
                        "REDIS_URL" => "redis.url".into(),
                        "JWT_SECRET" => "auth.jwt_secret".into(),
                        "ACCESS_TOKEN_TTL_SECS" => "auth.access_token_ttl_secs".into(),
                        "REFRESH_TOKEN_TTL_SECS" => "auth.refresh_token_ttl_secs".into(),
                        "IMPERSONATION_TTL_SECS" => "auth.impersonation_ttl_secs".into(),
                        _ => k.into(),
                    }),
            )
            .merge(Env::prefixed("CONFIG_"))
    }

    pub fn from_figment(figment: &Figment) -> Result<Self, AppError> {
        figment.extract::<Self>().map_err(|e| {
            AppError::with_detail(
                ErrorCode::Internal,
                "Failed to parse application configuration",
                e.to_string(),
            )
        })
    }

    pub fn load() -> Result<Self, AppError> {
        Self::from_figment(&Self::figment())
    }

    pub fn load_or_exit() -> Self {
        match Self::load() {
            Ok(config) => config,
            Err(e) => {
                eprintln!("Configuration error: {e}");
                std::process::exit(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_rejects_missing_database_url() {
        let figment = Figment::new()
            .merge(("database.app_url", "postgres://pos_app@localhost:5432/pos"))
            .merge(("server.port", 8080))
            .merge(("server.env", "test"))
            .merge(("redis.url", "redis://localhost:6379"))
            .merge(("auth.jwt_secret", "secret"));

        let res = Config::from_figment(&figment);
        assert!(res.is_err());
    }

    #[test]
    fn token_lifetimes_default_when_unset_and_take_the_value_given() {
        let base = Figment::new()
            .merge(("database.url", "postgres://pos_owner@localhost:5432/pos"))
            .merge(("database.app_url", "postgres://pos_app@localhost:5432/pos"))
            .merge(("server.port", 8080))
            .merge(("server.env", "test"))
            .merge(("redis.url", "redis://localhost:6379"))
            .merge(("auth.jwt_secret", "secret"));

        // Absent: the shipped defaults, which are what module 03 had hardcoded.
        let defaults = Config::from_figment(&base).expect("config without lifetimes parses");
        assert_eq!(defaults.auth.access_token_ttl_secs, 900);
        assert_eq!(defaults.auth.refresh_token_ttl_secs, 30 * 24 * 60 * 60);
        assert_eq!(defaults.auth.impersonation_ttl_secs, 60 * 60);

        // Present: the deployment's values win, which is the whole point of Q1 and Q3.
        let set = Config::from_figment(
            &base
                .merge(("auth.access_token_ttl_secs", 60))
                .merge(("auth.refresh_token_ttl_secs", 7 * 24 * 60 * 60))
                .merge(("auth.impersonation_ttl_secs", 120)),
        )
        .expect("config with lifetimes parses");
        assert_eq!(set.auth.access_token_ttl_secs, 60);
        assert_eq!(set.auth.refresh_token_ttl_secs, 7 * 24 * 60 * 60);
        assert_eq!(set.auth.impersonation_ttl_secs, 120);
    }

    #[test]
    fn config_rejects_malformed_port() {
        let figment = Figment::new()
            .merge(("database.url", "postgres://pos_owner@localhost:5432/pos"))
            .merge(("database.app_url", "postgres://pos_app@localhost:5432/pos"))
            .merge(("server.port", "not_a_valid_port"))
            .merge(("server.env", "test"))
            .merge(("redis.url", "redis://localhost:6379"))
            .merge(("auth.jwt_secret", "secret"));

        let res = Config::from_figment(&figment);
        assert!(res.is_err());
    }
}
