use deadpool_redis::{Config as RedisConfig, Pool, Runtime};
use pos_core::config::Config;
use pos_core::error::{AppError, ErrorCode};

#[derive(Clone)]
pub struct Cache {
    pool: Option<Pool>,
    prefix: String,
}

impl Cache {
    pub fn new(redis_url: &str, prefix: impl Into<String>) -> Result<Self, AppError> {
        let cfg = RedisConfig {
            url: Some(redis_url.to_string()),
            ..Default::default()
        };
        let pool = cfg.create_pool(Some(Runtime::Tokio1)).map_err(|e| {
            AppError::with_detail(
                ErrorCode::Internal,
                "Failed to create Redis connection pool",
                e.to_string(),
            )
        })?;
        Ok(Self {
            pool: Some(pool),
            prefix: prefix.into(),
        })
    }

    pub fn from_config(config: &Config, prefix: impl Into<String>) -> Result<Self, AppError> {
        Self::new(&config.redis.url, prefix)
    }

    #[must_use]
    pub fn in_memory(prefix: impl Into<String>) -> Self {
        Self {
            pool: None,
            prefix: prefix.into(),
        }
    }

    #[must_use]
    pub fn pool(&self) -> Option<&Pool> {
        self.pool.as_ref()
    }

    #[must_use]
    pub fn prefix(&self) -> &str {
        &self.prefix
    }
}
