use cache::Cache;
use pos_core::error::AppError;
use pos_core::ids::{SessionId, TenantId, UserId};

pub async fn revoke_session(
    cache: &Cache,
    session_id: SessionId,
    ttl_secs: u64,
) -> Result<(), AppError> {
    let key = format!("revoked:session:{session_id}");
    cache.set_ex(&key, "1", ttl_secs).await
}

pub async fn is_session_revoked(cache: &Cache, session_id: SessionId) -> Result<bool, AppError> {
    let key = format!("revoked:session:{session_id}");
    let val = cache.get(&key).await?;
    Ok(val.is_some())
}

pub async fn record_used_refresh_token(
    cache: &Cache,
    token_hash: &str,
    session_id: SessionId,
    ttl_secs: u64,
) -> Result<(), AppError> {
    let key = format!("used_refresh:{token_hash}");
    cache.set_ex(&key, &session_id.to_string(), ttl_secs).await
}

pub async fn check_refresh_token_reuse(
    cache: &Cache,
    token_hash: &str,
) -> Result<Option<SessionId>, AppError> {
    let key = format!("used_refresh:{token_hash}");
    if let Some(val) = cache.get(&key).await? {
        if let Ok(uuid) = uuid::Uuid::parse_str(&val) {
            return Ok(Some(SessionId::from(uuid)));
        }
    }
    Ok(None)
}

pub async fn clear_cached_branch_access(
    cache: &Cache,
    tenant_id: TenantId,
    user_id: UserId,
) -> Result<(), AppError> {
    let branch_key = format!("t:{tenant_id}:u:{user_id}:branches");
    let perms_key = format!("t:{tenant_id}:u:{user_id}:perms");
    cache.del(&branch_key).await?;
    cache.del(&perms_key).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn session_revocation_round_trip() {
        let cache = Cache::in_memory("test_rev");
        let session_id = SessionId::new_v7();

        assert!(!is_session_revoked(&cache, session_id).await.unwrap());
        revoke_session(&cache, session_id, 300).await.unwrap();
        assert!(is_session_revoked(&cache, session_id).await.unwrap());
    }

    #[tokio::test]
    async fn refresh_token_reuse_detection() {
        let cache = Cache::in_memory("test_reuse");
        let session_id = SessionId::new_v7();
        let token_hash = "some_sha256_hash_here";

        assert_eq!(
            check_refresh_token_reuse(&cache, token_hash).await.unwrap(),
            None
        );
        record_used_refresh_token(&cache, token_hash, session_id, 300)
            .await
            .unwrap();
        assert_eq!(
            check_refresh_token_reuse(&cache, token_hash).await.unwrap(),
            Some(session_id)
        );
    }
}
