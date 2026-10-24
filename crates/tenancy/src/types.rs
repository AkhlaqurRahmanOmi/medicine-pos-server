use chrono::{DateTime, Utc};
use pos_core::ids::{BranchId, TenantId};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AccessSummary {
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Tenant {
    #[schema(value_type = String)]
    pub id: TenantId,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legal_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vat_bin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    pub status: String,
    pub access: AccessSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TenantSummary {
    #[schema(value_type = String)]
    pub id: TenantId,
    pub name: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Branch {
    #[schema(value_type = String)]
    pub id: BranchId,
    #[schema(value_type = String)]
    pub tenant_id: TenantId,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drug_license_no: Option<String>,
    pub is_default: bool,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

pub mod decimal_str {
    use rust_decimal::Decimal;
    use serde::{Deserialize, Deserializer, Serializer};
    use std::str::FromStr;

    pub fn serialize<S>(d: &Decimal, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&d.to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Decimal, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Decimal::from_str(&s).map_err(serde::de::Error::custom)
    }
}

pub mod decimal_str_opt {
    use rust_decimal::Decimal;
    use serde::{Deserialize, Deserializer, Serializer};
    use std::str::FromStr;

    pub fn serialize<S>(d: &Option<Decimal>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match d {
            Some(d) => serializer.serialize_some(&d.to_string()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Decimal>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: Option<String> = Option::deserialize(deserializer)?;
        match s {
            Some(s) => Decimal::from_str(&s).map(Some).map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}

pub fn deserialize_nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TenantSettings {
    #[schema(value_type = String)]
    #[serde(with = "decimal_str")]
    pub vat_rate: Decimal,
    pub vat_included: bool,
    pub rounding_mode: String,
    pub near_expiry_days: i32,
    pub default_payment_terms_days: i32,
    pub allow_zero_stock_sale: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt_header: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt_footer: Option<String>,
    pub receipt_paper_width: i32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct SettingsPatch {
    #[schema(value_type = Option<String>)]
    #[serde(default, with = "decimal_str_opt")]
    pub vat_rate: Option<Decimal>,
    pub vat_included: Option<bool>,
    pub rounding_mode: Option<String>,
    pub near_expiry_days: Option<i32>,
    pub default_payment_terms_days: Option<i32>,
    pub allow_zero_stock_sale: Option<bool>,
    #[schema(value_type = Option<String>)]
    #[serde(default, deserialize_with = "deserialize_nullable")]
    pub receipt_header: Option<Option<String>>,
    #[schema(value_type = Option<String>)]
    #[serde(default, deserialize_with = "deserialize_nullable")]
    pub receipt_footer: Option<Option<String>>,
    pub receipt_paper_width: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Subscription {
    pub id: Uuid,
    #[schema(value_type = String)]
    pub tenant_id: TenantId,
    pub plan_code: String,
    pub billing_period: String,
    #[schema(value_type = String)]
    #[serde(with = "decimal_str")]
    pub price: Decimal,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub status: String,
    pub is_complimentary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Payment {
    pub id: Uuid,
    pub subscription_id: Option<Uuid>,
    #[schema(value_type = String)]
    #[serde(with = "decimal_str")]
    pub amount: Decimal,
    pub method: String,
    pub reference: String,
    pub status: String,
    pub paid_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Plan {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub grace_days: i32,
    pub is_active: bool,
    pub prices: Vec<PlanPrice>,
    pub limits: Vec<PlanLimit>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PlanPrice {
    pub billing_period: String,
    #[schema(value_type = String)]
    #[serde(with = "decimal_str")]
    pub price: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PlanLimit {
    pub limit_key: String,
    pub max_value: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NewBranch {
    pub name: String,
    pub address: Option<String>,
    pub phone: Option<String>,
    pub drug_license_no: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NewPayment {
    pub subscription_id: Option<Uuid>,
    #[schema(value_type = String)]
    #[serde(with = "decimal_str")]
    pub amount: Decimal,
    pub method: String,
    pub reference: String,
    pub proof_file_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NewSubscription {
    pub plan_id: Uuid,
    pub billing_period: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub is_complimentary: bool,
    pub grant_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct TenantPatch {
    pub name: Option<String>,
    #[schema(value_type = Option<String>)]
    #[serde(default, deserialize_with = "deserialize_nullable")]
    pub legal_name: Option<Option<String>>,
    #[schema(value_type = Option<String>)]
    #[serde(default, deserialize_with = "deserialize_nullable")]
    pub vat_bin: Option<Option<String>>,
    #[schema(value_type = Option<String>)]
    #[serde(default, deserialize_with = "deserialize_nullable")]
    pub phone: Option<Option<String>>,
    #[schema(value_type = Option<String>)]
    #[serde(default, deserialize_with = "deserialize_nullable")]
    pub email: Option<Option<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct BranchPatch {
    pub name: Option<String>,
    #[schema(value_type = Option<String>)]
    #[serde(default, deserialize_with = "deserialize_nullable")]
    pub address: Option<Option<String>>,
    #[schema(value_type = Option<String>)]
    #[serde(default, deserialize_with = "deserialize_nullable")]
    pub phone: Option<Option<String>>,
    #[schema(value_type = Option<String>)]
    #[serde(default, deserialize_with = "deserialize_nullable")]
    pub drug_license_no: Option<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UploadSlipRequest {
    pub proof_file_key: String,
    #[serde(default)]
    pub file_size_bytes: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TenantExport {
    pub id: Uuid,
    #[schema(value_type = String)]
    pub tenant_id: TenantId,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ready_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TenantDetail {
    pub tenant: Tenant,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings: Option<TenantSettings>,
    pub branches: Vec<Branch>,
    pub subscriptions: Vec<Subscription>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NewPlanPriceRequest {
    pub billing_period: String,
    #[schema(value_type = String)]
    #[serde(with = "decimal_str")]
    pub price: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NewPlanRequest {
    pub code: String,
    pub name: String,
    pub grace_days: i32,
    pub is_active: bool,
    #[serde(default)]
    pub prices: Vec<NewPlanPriceRequest>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct UpdatePlanRequest {
    pub name: Option<String>,
    pub grace_days: Option<i32>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SetPlanLimitRequest {
    pub max_value: Option<i32>,
}
