use pos_core::error::{AppError, ErrorCode};
use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock};

use crate::keys::prefixed_key;
use crate::pool::Cache;

#[derive(Debug, Clone)]
pub struct IdempotencyGuard {
    key: String,
}

impl IdempotencyGuard {
    #[must_use]
    pub fn new(key: impl Into<String>) -> Self {
        Self { key: key.into() }
    }

    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }
}

static IN_MEMORY_KEYS: OnceLock<Arc<Mutex<HashSet<String>>>> = OnceLock::new();

fn in_memory_store() -> &'static Arc<Mutex<HashSet<String>>> {
    IN_MEMORY_KEYS.get_or_init(|| Arc::new(Mutex::new(HashSet::new())))
}

impl Cache {
    pub async fn idempotency_guard(&self, key: &str) -> Result<IdempotencyGuard, AppError> {
        let full_key = prefixed_key(self.prefix(), &format!("idempotency:{key}"));
        let store = in_memory_store();
        let mut keys = store
            .lock()
            .map_err(|e| AppError::internal("Idempotency lock poisoned", e.to_string()))?;

        if keys.contains(&full_key) {
            Err(AppError::new(
                ErrorCode::Conflict,
                "Request replay rejected: key already processed or in progress",
            ))
        } else {
            keys.insert(full_key.clone());
            Ok(IdempotencyGuard::new(full_key))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn idempotency_guard_rejects_replay() {
        let cache = Cache::in_memory("test");
        let client_key = "unique-transaction-key-001";

        // First attempt succeeds
        let first_try = cache.idempotency_guard(client_key).await;
        assert!(first_try.is_ok(), "First request with key should succeed");

        // Replay attempt fails with conflict
        let second_try = cache.idempotency_guard(client_key).await;
        assert!(
            second_try.is_err(),
            "Replay request with duplicate key must be rejected"
        );
        assert_eq!(second_try.unwrap_err().code, ErrorCode::Conflict);
    }
}
