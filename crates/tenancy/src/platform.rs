use crate::repo;
use crate::types::*;
use db::Tx;
use pos_core::error::AppError;
use pos_core::ids::{TenantId, UserId};
use uuid::Uuid;

pub async fn approve(tx: &Tx, tenant: TenantId, by: UserId) -> Result<Tenant, AppError> {
    repo::approve_tenant(tx, tenant, by).await
}

pub async fn suspend(tx: &Tx, tenant: TenantId, reason: String) -> Result<Tenant, AppError> {
    repo::suspend_tenant(tx, tenant, reason).await
}

pub async fn reinstate(tx: &Tx, tenant: TenantId) -> Result<Tenant, AppError> {
    repo::reinstate_tenant(tx, tenant).await
}

pub async fn confirm_payment(
    tx: &Tx,
    id: Uuid,
) -> Result<(Payment, Subscription), AppError> {
    repo::confirm_payment(tx, id).await
}

pub async fn reject_payment(
    tx: &Tx,
    id: Uuid,
    note: String,
) -> Result<Payment, AppError> {
    repo::reject_payment(tx, id, note).await
}

pub async fn create_subscription(
    tx: &Tx,
    tenant: TenantId,
    input: NewSubscription,
) -> Result<Subscription, AppError> {
    repo::create_subscription(tx, tenant, input).await
}

pub async fn get_tenant_detail(tx: &Tx, id: TenantId) -> Result<TenantDetail, AppError> {
    repo::get_tenant_detail(tx, id).await
}

pub async fn create_plan(tx: &Tx, req: NewPlanRequest) -> Result<Plan, AppError> {
    repo::create_plan(tx, req).await
}

pub async fn update_plan(tx: &Tx, id: Uuid, req: UpdatePlanRequest) -> Result<Plan, AppError> {
    repo::update_plan(tx, id, req).await
}

pub async fn set_plan_limit(
    tx: &Tx,
    plan_id: Uuid,
    limit_key: String,
    max_value: Option<i32>,
) -> Result<PlanLimit, AppError> {
    repo::set_plan_limit(tx, plan_id, limit_key, max_value).await
}
