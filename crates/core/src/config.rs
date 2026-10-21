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
                    ])
                    .map(|k| match k.as_str() {
                        "APP_PORT" => "server.port".into(),
                        "APP_ENV" => "server.env".into(),
                        "DATABASE_URL" => "database.url".into(),
                        "APP_DATABASE_URL" => "database.app_url".into(),
                        "REDIS_URL" => "redis.url".into(),
                        "JWT_SECRET" => "auth.jwt_secret".into(),
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
