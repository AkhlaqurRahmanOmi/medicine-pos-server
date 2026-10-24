use cache::Cache;
use pos_core::error::AppError;

pub const LOGIN_FAILURE_LIMIT: u32 = 5;
pub const LOGIN_WINDOW_SECS: u64 = 900;

pub async fn check_login_rate_limit(cache: &Cache, email: &str) -> Result<(), AppError> {
    let norm_email = email.trim().to_lowercase();
    let key = format!("rl:login:{norm_email}");
    if let Some(val) = cache.get(&key).await? {
        if let Ok(count) = val.parse::<u32>() {
            if count >= LOGIN_FAILURE_LIMIT {
                return Err(AppError::rate_limited(
                    "Too many failed login attempts. Account is locked out for 15 minutes.",
                ));
            }
        }
    }
    Ok(())
}

pub async fn record_login_failure(cache: &Cache, email: &str) -> Result<u32, AppError> {
    let norm_email = email.trim().to_lowercase();
    let key = format!("rl:login:{norm_email}");

    if let Some(pool) = cache.pool() {
        let mut conn = pool
            .get()
            .await
            .map_err(|e| AppError::internal("Redis pool error", e.to_string()))?;
        let full_key = cache::prefixed_key(cache.prefix(), &key);
        let count: u32 = deadpool_redis::redis::cmd("INCR")
            .arg(&full_key)
            .query_async(&mut conn)
            .await
            .map_err(|e| AppError::internal("Redis INCR failed", e.to_string()))?;
        if count == 1 {
            let _: () = deadpool_redis::redis::cmd("EXPIRE")
                .arg(&full_key)
                .arg(LOGIN_WINDOW_SECS)
                .query_async(&mut conn)
                .await
                .map_err(|e| AppError::internal("Redis EXPIRE failed", e.to_string()))?;
        }
        Ok(count)
    } else {
        let curr = cache
            .get(&key)
            .await?
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        let next = curr + 1;
        cache
            .set_ex(&key, &next.to_string(), LOGIN_WINDOW_SECS)
            .await?;
        Ok(next)
    }
}

pub async fn clear_login_attempts(cache: &Cache, email: &str) -> Result<(), AppError> {
    let norm_email = email.trim().to_lowercase();
    let key = format!("rl:login:{norm_email}");
    cache.del(&key).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rate_limiter_in_memory_records_and_locks_out() {
        let cache = Cache::in_memory("test_rl");
        let email = "test@example.com";

        for _ in 0..5 {
            assert!(check_login_rate_limit(&cache, email).await.is_ok());
            record_login_failure(&cache, email).await.unwrap();
        }

        // 6th check should fail
        let res = check_login_rate_limit(&cache, email).await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().code, pos_core::error::ErrorCode::TooManyRequests);

        // Clearing allows login again
        clear_login_attempts(&cache, email).await.unwrap();
        assert!(check_login_rate_limit(&cache, email).await.is_ok());
    }
}
