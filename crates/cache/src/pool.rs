use deadpool_redis::{Config as RedisConfig, Pool, Runtime};
use pos_core::config::Config;
use pos_core::error::{AppError, ErrorCode};

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type InMemoryEntry = (String, Option<Instant>);
type InMemoryStore = Arc<Mutex<HashMap<String, InMemoryEntry>>>;

#[derive(Clone)]
pub struct Cache {
    pool: Option<Pool>,
    prefix: String,
    in_memory: InMemoryStore,
}

impl Cache {
    pub fn new(redis_url: &str, prefix: impl Into<String>) -> Result<Self, AppError> {
        let cfg = RedisConfig::from_url(redis_url);
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
            in_memory: Arc::new(Mutex::new(HashMap::new())),
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
            in_memory: Arc::new(Mutex::new(HashMap::new())),
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

    pub async fn get(&self, key: &str) -> Result<Option<String>, AppError> {
        let full_key = crate::keys::prefixed_key(&self.prefix, key);
        if let Some(pool) = &self.pool {
            let mut conn = pool.get().await.map_err(|e| {
                AppError::internal("Redis connection failed", e.to_string())
            })?;
            let val: Option<String> = deadpool_redis::redis::cmd("GET")
                .arg(&full_key)
                .query_async(&mut conn)
                .await
                .map_err(|e| AppError::internal("Redis GET failed", e.to_string()))?;
            Ok(val)
        } else {
            let mut map = self
                .in_memory
                .lock()
                .map_err(|e| AppError::internal("Cache in-memory lock poisoned", e.to_string()))?;
            if let Some((val, expiry)) = map.get(&full_key) {
                if let Some(exp) = expiry {
                    if Instant::now() > *exp {
                        map.remove(&full_key);
                        return Ok(None);
                    }
                }
                Ok(Some(val.clone()))
            } else {
                Ok(None)
            }
        }
    }

    pub async fn set_ex(&self, key: &str, value: &str, ttl_secs: u64) -> Result<(), AppError> {
        let full_key = crate::keys::prefixed_key(&self.prefix, key);
        if let Some(pool) = &self.pool {
            let mut conn = pool.get().await.map_err(|e| {
                AppError::internal("Redis connection failed", e.to_string())
            })?;
            let _: () = deadpool_redis::redis::cmd("SETEX")
                .arg(&full_key)
                .arg(ttl_secs)
                .arg(value)
                .query_async(&mut conn)
                .await
                .map_err(|e| AppError::internal("Redis SETEX failed", e.to_string()))?;
            Ok(())
        } else {
            let mut map = self
                .in_memory
                .lock()
                .map_err(|e| AppError::internal("Cache in-memory lock poisoned", e.to_string()))?;
            let expiry = Some(Instant::now() + Duration::from_secs(ttl_secs));
            map.insert(full_key, (value.to_string(), expiry));
            Ok(())
        }
    }

    pub async fn del(&self, key: &str) -> Result<(), AppError> {
        let full_key = crate::keys::prefixed_key(&self.prefix, key);
        if let Some(pool) = &self.pool {
            let mut conn = pool.get().await.map_err(|e| {
                AppError::internal("Redis connection failed", e.to_string())
            })?;
            let _: () = deadpool_redis::redis::cmd("DEL")
                .arg(&full_key)
                .query_async(&mut conn)
                .await
                .map_err(|e| AppError::internal("Redis DEL failed", e.to_string()))?;
            Ok(())
        } else {
            let mut map = self
                .in_memory
                .lock()
                .map_err(|e| AppError::internal("Cache in-memory lock poisoned", e.to_string()))?;
            map.remove(&full_key);
            Ok(())
        }
    }
}
