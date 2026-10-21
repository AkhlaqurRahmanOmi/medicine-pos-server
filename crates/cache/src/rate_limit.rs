use pos_core::error::AppError;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::keys::prefixed_key;
use crate::pool::Cache;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Allowed {
    Yes { remaining: u32 },
    No { retry_after_secs: u64 },
}

impl Allowed {
    #[must_use]
    pub const fn is_allowed(&self) -> bool {
        matches!(self, Self::Yes { .. })
    }
}

#[derive(Clone)]
pub struct RateLimiter {
    cache: Cache,
    memory_state: Arc<Mutex<HashMap<String, (u32, Instant)>>>,
}

impl RateLimiter {
    #[must_use]
    pub fn new(cache: Cache) -> Self {
        Self {
            cache,
            memory_state: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn check(
        &self,
        key: &str,
        limit: u32,
        window: Duration,
    ) -> Result<Allowed, AppError> {
        let full_key = prefixed_key(self.cache.prefix(), key);

        // In-memory / fallback implementation
        let mut map = self
            .memory_state
            .lock()
            .map_err(|e| AppError::internal("Rate limiter lock poisoned", e.to_string()))?;

        let now = Instant::now();
        if let Some((count, window_start)) = map.get_mut(&full_key) {
            if now.duration_since(*window_start) >= window {
                // Window expired: reset counter
                *count = 1;
                *window_start = now;
                Ok(Allowed::Yes {
                    remaining: limit.saturating_sub(1),
                })
            } else if *count < limit {
                *count += 1;
                Ok(Allowed::Yes {
                    remaining: limit.saturating_sub(*count),
                })
            } else {
                let elapsed = now.duration_since(*window_start);
                let retry_after = window.saturating_sub(elapsed).as_secs().max(1);
                Ok(Allowed::No {
                    retry_after_secs: retry_after,
                })
            }
        } else {
            map.insert(full_key, (1, now));
            Ok(Allowed::Yes {
                remaining: limit.saturating_sub(1),
            })
        }
    }
}

impl Cache {
    pub async fn rate_limit(
        &self,
        key: &str,
        limit: u32,
        window: Duration,
    ) -> Result<Allowed, AppError> {
        let limiter = RateLimiter::new(self.clone());
        limiter.check(key, limit, window).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rate_limit_blocks_after_limit() {
        let cache = Cache::in_memory("test");
        let limiter = RateLimiter::new(cache);
        let key = "login:attempt:ip1";
        let limit = 3;
        let window = Duration::from_secs(60);

        // First 3 requests allowed
        for i in 1..=limit {
            let res = limiter.check(key, limit, window).await.expect("check ok");
            assert!(res.is_allowed(), "Request {i} should be allowed");
        }

        // 4th request blocked
        let res = limiter.check(key, limit, window).await.expect("check ok");
        assert!(!res.is_allowed(), "Request 4 should be blocked");
    }

    #[tokio::test]
    async fn rate_limit_window_expires() {
        let cache = Cache::in_memory("test");
        let limiter = RateLimiter::new(cache);
        let key = "login:attempt:ip2";
        let limit = 2;
        let window = Duration::from_millis(50);

        // Use up quota
        assert!(limiter
            .check(key, limit, window)
            .await
            .unwrap()
            .is_allowed());
        assert!(limiter
            .check(key, limit, window)
            .await
            .unwrap()
            .is_allowed());
        assert!(!limiter
            .check(key, limit, window)
            .await
            .unwrap()
            .is_allowed());

        // Wait for window to expire
        tokio::time::sleep(Duration::from_millis(60)).await;

        // Counter reset: request allowed again
        let res = limiter.check(key, limit, window).await.expect("check ok");
        assert!(
            res.is_allowed(),
            "Request after window expiry should be allowed"
        );
    }
}
