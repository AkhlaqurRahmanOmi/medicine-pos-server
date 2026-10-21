use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{BranchId, RequestId, TenantId, UserId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantContext {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub branch_ids: Vec<BranchId>,
}

impl TenantContext {
    #[must_use]
    pub fn new(tenant_id: TenantId, user_id: UserId, branch_ids: Vec<BranchId>) -> Self {
        Self {
            tenant_id,
            user_id,
            branch_ids,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestContext {
    pub request_id: RequestId,
    pub now: DateTime<Utc>,
}

impl RequestContext {
    #[must_use]
    pub fn new(request_id: RequestId, now: DateTime<Utc>) -> Self {
        Self { request_id, now }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_stores_ids() {
        let tenant_id = TenantId::new_v7();
        let user_id = UserId::new_v7();
        let branch_id = BranchId::new_v7();

        let ctx = TenantContext::new(tenant_id, user_id, vec![branch_id]);
        assert_eq!(ctx.tenant_id, tenant_id);
        assert_eq!(ctx.user_id, user_id);
        assert_eq!(ctx.branch_ids.len(), 1);
    }
}
