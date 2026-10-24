use crate::repo;
use crate::types::*;
use db::Tx;
use pos_core::error::AppError;
use pos_core::ids::{BranchId, UserId};

pub async fn create_pharmacy(
    tx: &Tx,
    name: &str,
    legal_name: Option<String>,
    vat_bin: Option<String>,
    phone: Option<String>,
    email: Option<String>,
) -> Result<Tenant, AppError> {
    repo::create_pharmacy(tx, name, legal_name, vat_bin, phone, email).await
}

pub async fn create_branch(tx: &Tx, input: NewBranch) -> Result<Branch, AppError> {
    repo::create_branch(tx, input).await
}

pub async fn deactivate_branch(tx: &Tx, id: BranchId) -> Result<Branch, AppError> {
    repo::deactivate_branch(tx, id).await
}

pub async fn update_settings(tx: &Tx, input: SettingsPatch) -> Result<TenantSettings, AppError> {
    repo::update_settings(tx, input).await
}

pub async fn submit_payment(tx: &Tx, input: NewPayment) -> Result<Payment, AppError> {
    repo::submit_payment(tx, input).await
}

pub async fn request_closure(tx: &Tx, by: UserId) -> Result<Tenant, AppError> {
    repo::request_closure(tx, by).await
}

pub async fn update_tenant(tx: &Tx, patch: TenantPatch) -> Result<Tenant, AppError> {
    repo::update_tenant(tx, patch).await
}

pub async fn update_branch(tx: &Tx, id: BranchId, patch: BranchPatch) -> Result<Branch, AppError> {
    repo::update_branch(tx, id, patch).await
}

pub async fn upload_payment_slip(
    tx: &Tx,
    payment_id: uuid::Uuid,
    proof_file_key: String,
) -> Result<Payment, AppError> {
    repo::upload_payment_slip(tx, payment_id, proof_file_key).await
}

pub async fn request_export(tx: &Tx, user_id: UserId) -> Result<TenantExport, AppError> {
    repo::request_export(tx, user_id).await
}
