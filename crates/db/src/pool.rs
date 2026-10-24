use pos_core::config::Config;
use pos_core::error::{AppError, ErrorCode};
use sea_orm::{Database, DatabaseConnection};
use sqlx::PgPool;

#[derive(Clone)]
pub struct Db {
    conn: DatabaseConnection,
    app_url: String,
}

impl Db {
    pub async fn connect(config: &Config) -> Result<Self, AppError> {
        let app_url = Self::target_url(config).to_string();
        let conn = Database::connect(&app_url).await.map_err(|e| {
            AppError::with_detail(
                ErrorCode::Internal,
                "Failed to connect to database with non-owner role",
                e.to_string(),
            )
        })?;
        Ok(Self { conn, app_url })
    }

    #[must_use]
    pub fn target_url(config: &Config) -> &str {
        &config.database.app_url
    }

    #[must_use]
    pub fn from_connection(conn: DatabaseConnection, app_url: impl Into<String>) -> Self {
        Self {
            conn,
            app_url: app_url.into(),
        }
    }

    #[must_use]
    pub fn conn(&self) -> &DatabaseConnection {
        &self.conn
    }

    #[must_use]
    pub fn pool(&self) -> &PgPool {
        self.conn.get_postgres_connection_pool()
    }

    #[must_use]
    pub fn app_url(&self) -> &str {
        &self.app_url
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pos_core::config::{AuthConfig, DatabaseConfig, RedisConfig, ServerConfig};

    #[tokio::test]
    async fn pool_connects_as_non_owner() {
        let config = Config {
            server: ServerConfig {
                port: 8080,
                env: "test".into(),
            },
            database: DatabaseConfig {
                url: "postgres://pos_owner:devpass@localhost:5432/medicine_pos".into(),
                app_url: "postgres://pos_app:devpass@localhost:5432/medicine_pos".into(),
            },
            redis: RedisConfig {
                url: "redis://localhost:6379".into(),
            },
            auth: AuthConfig {
                jwt_secret: "secret".into(),
                access_token_ttl_secs: 900,
                refresh_token_ttl_secs: 30 * 24 * 60 * 60,
                impersonation_ttl_secs: 60 * 60,
            },
        };

        // Assert target connection URL is the non-owner app_url and not the owner url
        let target_url = Db::target_url(&config);
        assert_eq!(target_url, config.database.app_url);
        assert_ne!(target_url, config.database.url);
        assert!(
            target_url.contains("pos_app"),
            "Application connection must use pos_app non-owner role"
        );
        assert!(
            !target_url.contains("pos_owner"),
            "Application connection must never connect as pos_owner"
        );
    }
}
