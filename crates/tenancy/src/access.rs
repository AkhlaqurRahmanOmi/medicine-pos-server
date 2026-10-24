use cache::Cache;
use chrono::{DateTime, Utc};
use db::Db;
use pos_core::error::{AppError, ErrorCode};
use pos_core::ids::TenantId;
use pos_core::ports::{AccessModeSource, BoxFuture};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccessMode {
    Blocked(BlockReason),
    ReadOnly,
    Full,
    FullWithWarning { until: DateTime<Utc> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockReason {
    PendingApproval,
    Suspended,
}

impl AccessMode {
    pub fn permits_write(&self) -> bool {
        matches!(self, Self::Full | Self::FullWithWarning { .. })
    }

    pub fn error(&self) -> Option<AppError> {
        match self {
            Self::Blocked(BlockReason::PendingApproval) => Some(AppError::with_detail(
                ErrorCode::TenantPendingApproval,
                "Tenant is awaiting platform approval",
                "Your pharmacy registration is pending approval.",
            )),
            Self::Blocked(BlockReason::Suspended) => Some(AppError::with_detail(
                ErrorCode::TenantSuspended,
                "Tenant account has been suspended",
                "Your pharmacy account has been suspended. Please contact support.",
            )),
            Self::ReadOnly => Some(AppError::with_detail(
                ErrorCode::SubscriptionReadOnly,
                "Subscription expired and grace period lapsed",
                "Your subscription has lapsed and grace period expired. Read-only mode.",
            )),
            Self::Full | Self::FullWithWarning { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TenantStatus {
    PendingApproval,
    Active,
    Suspended,
    Closing,
}

impl TryFrom<&str> for TenantStatus {
    type Error = AppError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "pending_approval" => Ok(Self::PendingApproval),
            "active" => Ok(Self::Active),
            "suspended" => Ok(Self::Suspended),
            "closing" => Ok(Self::Closing),
            other => Err(AppError::validation_failed(format!(
                "Unknown tenant status '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SubscriptionWindow {
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct AccessFacts {
    pub status: TenantStatus,
    pub covering: Option<SubscriptionWindow>,
    pub latest_end: Option<DateTime<Utc>>,
    pub grace_days: i32,
}

pub fn decide(facts: &AccessFacts, now: DateTime<Utc>) -> AccessMode {
    match facts.status {
        TenantStatus::PendingApproval => AccessMode::Blocked(BlockReason::PendingApproval),
        TenantStatus::Suspended => AccessMode::Blocked(BlockReason::Suspended),
        TenantStatus::Active | TenantStatus::Closing => {
            if let Some(covering) = &facts.covering {
                if covering.starts_at <= now && now <= covering.ends_at {
                    return AccessMode::Full;
                }
            }
            if let Some(latest_end) = facts.latest_end {
                let grace_until = latest_end + chrono::Duration::days(facts.grace_days as i64);
                if now > latest_end && now <= grace_until {
                    return AccessMode::FullWithWarning { until: grace_until };
                }
            }
            AccessMode::ReadOnly
        }
    }
}

pub struct DummyAccessModeSource;

impl AccessModeSource for DummyAccessModeSource {
    fn check_access<'a>(
        &'a self,
        _tenant_id: TenantId,
        _is_write: bool,
    ) -> BoxFuture<'a, Result<(), AppError>> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Clone)]
pub struct TenancyAccessModeSource {
    db: Db,
    cache: Cache,
}

impl TenancyAccessModeSource {
    pub fn new(db: Db, cache: Cache) -> Self {
        Self { db, cache }
    }

    pub async fn resolve_access_mode(&self, tenant_id: TenantId) -> Result<AccessMode, AppError> {
        let cache_key = format!("tenant:{}:access", tenant_id.as_uuid());
        // 1. Try cache
        match self.cache.get(&cache_key).await {
            Ok(Some(raw)) => {
                if let Ok(mode) = serde_json::from_str::<AccessMode>(&raw) {
                    return Ok(mode);
                }
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(tenant_id = %tenant_id.as_uuid(), error = %e, "Cache read failed, falling back to database");
            }
        }

        // 2. Cache miss or cache failure: fall back to database
        let mode = crate::repo::get_access_mode(&self.db, tenant_id, Utc::now()).await?;

        // 3. Populate cache (TTL 60 seconds)
        if let Ok(serialized) = serde_json::to_string(&mode) {
            if let Err(e) = self.cache.set_ex(&cache_key, &serialized, 60).await {
                tracing::warn!(tenant_id = %tenant_id.as_uuid(), error = %e, "Cache write failed");
            }
        }

        Ok(mode)
    }
}

impl AccessModeSource for TenancyAccessModeSource {
    fn check_access<'a>(
        &'a self,
        tenant_id: TenantId,
        is_write: bool,
    ) -> BoxFuture<'a, Result<(), AppError>> {
        Box::pin(async move {
            let mode = self.resolve_access_mode(tenant_id).await?;
            match mode {
                AccessMode::Blocked(BlockReason::PendingApproval) => {
                    Err(AppError::with_detail(
                        ErrorCode::TenantPendingApproval,
                        "Tenant is awaiting platform approval",
                        "Your pharmacy registration is pending approval.",
                    ))
                }
                AccessMode::Blocked(BlockReason::Suspended) => {
                    Err(AppError::with_detail(
                        ErrorCode::TenantSuspended,
                        "Tenant account has been suspended",
                        "Your pharmacy account has been suspended. Please contact support.",
                    ))
                }
                AccessMode::ReadOnly => {
                    if is_write {
                        Err(AppError::with_detail(
                            ErrorCode::SubscriptionReadOnly,
                            "Subscription expired and grace period lapsed",
                            "Your subscription has lapsed and grace period expired. Read-only mode.",
                        ))
                    } else {
                        Ok(())
                    }
                }
                AccessMode::Full | AccessMode::FullWithWarning { .. } => Ok(()),
            }
        })
    }
}

pub async fn invalidate_access_mode(cache: &Cache, tenant_id: TenantId) -> Result<(), AppError> {
    let cache_key = format!("tenant:{}:access", tenant_id.as_uuid());
    cache.del(&cache_key).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn access_mode_state_table() {
        let now = Utc::now();
        let covering = Some(SubscriptionWindow {
            starts_at: now - Duration::days(10),
            ends_at: now + Duration::days(20),
        });
        let expired_end = now - Duration::days(5);
        let grace_days = 7;

        // 1. PendingApproval: always blocked, subscription irrelevant
        let facts_pending = AccessFacts {
            status: TenantStatus::PendingApproval,
            covering: covering.clone(),
            latest_end: Some(now + Duration::days(20)),
            grace_days,
        };
        assert_eq!(
            decide(&facts_pending, now),
            AccessMode::Blocked(BlockReason::PendingApproval)
        );

        // 2. Suspended: always blocked, subscription irrelevant
        let facts_suspended = AccessFacts {
            status: TenantStatus::Suspended,
            covering: covering.clone(),
            latest_end: Some(now + Duration::days(20)),
            grace_days,
        };
        assert_eq!(
            decide(&facts_suspended, now),
            AccessMode::Blocked(BlockReason::Suspended)
        );

        // 3. Active + Covering: Full
        let facts_active_covered = AccessFacts {
            status: TenantStatus::Active,
            covering: covering.clone(),
            latest_end: Some(now + Duration::days(20)),
            grace_days,
        };
        assert_eq!(decide(&facts_active_covered, now), AccessMode::Full);

        // 4. Active + Inside Grace: FullWithWarning
        let facts_active_grace = AccessFacts {
            status: TenantStatus::Active,
            covering: None,
            latest_end: Some(expired_end),
            grace_days: 7,
        };
        let expected_warning_until = expired_end + Duration::days(7);
        assert_eq!(
            decide(&facts_active_grace, now),
            AccessMode::FullWithWarning {
                until: expected_warning_until
            }
        );

        // 5. Active + Past Grace: ReadOnly
        let facts_active_past_grace = AccessFacts {
            status: TenantStatus::Active,
            covering: None,
            latest_end: Some(now - Duration::days(10)),
            grace_days: 7,
        };
        assert_eq!(decide(&facts_active_past_grace, now), AccessMode::ReadOnly);

        // 6. Active + No subscription ever: ReadOnly
        let facts_active_no_sub = AccessFacts {
            status: TenantStatus::Active,
            covering: None,
            latest_end: None,
            grace_days: 7,
        };
        assert_eq!(decide(&facts_active_no_sub, now), AccessMode::ReadOnly);

        // 7. Closing: same logic as Active
        let facts_closing = AccessFacts {
            status: TenantStatus::Closing,
            covering,
            latest_end: Some(now + Duration::days(20)),
            grace_days,
        };
        assert_eq!(decide(&facts_closing, now), AccessMode::Full);
    }

    #[test]
    fn suspended_and_pending_are_different_codes() {
        let pending_mode = AccessMode::Blocked(BlockReason::PendingApproval);
        let suspended_mode = AccessMode::Blocked(BlockReason::Suspended);

        let err_pending = pending_mode.error().expect("pending has error");
        let err_suspended = suspended_mode.error().expect("suspended has error");

        assert_eq!(err_pending.code, ErrorCode::TenantPendingApproval);
        assert_eq!(err_suspended.code, ErrorCode::TenantSuspended);
        assert_ne!(err_pending.code, err_suspended.code);

        assert_eq!(err_pending.code.as_str(), "tenant_pending_approval");
        assert_eq!(err_suspended.code.as_str(), "tenant_suspended");
    }

    #[test]
    fn inside_grace_is_full_with_a_warning() {
        let now = Utc::now();
        let ended_at = now - Duration::days(3);
        let grace_days = 7;
        let expected_until = ended_at + Duration::days(grace_days as i64);

        let facts = AccessFacts {
            status: TenantStatus::Active,
            covering: None,
            latest_end: Some(ended_at),
            grace_days,
        };

        let mode = decide(&facts, now);
        match mode {
            AccessMode::FullWithWarning { until } => {
                assert_eq!(until, expected_until);
            }
            other => panic!("Expected FullWithWarning, got {other:?}"),
        }

        assert!(mode.permits_write(), "Grace period permits writes");
        assert!(mode.error().is_none(), "Grace period has no error");
    }
}
