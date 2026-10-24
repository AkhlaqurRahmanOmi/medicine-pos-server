use chrono::{DateTime, Utc};
pub use pos_core::ids::{
    CompanyId, DosageFormId, GenericId, IngredientId, IssueReportId, PackUnitId, ProductId,
    TenantId, TherapeuticClassId, UserId,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use utoipa::ToSchema;

pub mod decimal_str_opt {
    use rust_decimal::Decimal;
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &Option<Decimal>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(d) => serializer.serialize_some(&d.to_string()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Decimal>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<String>::deserialize(deserializer)?
            .map(|s| s.parse::<Decimal>().map_err(serde::de::Error::custom))
            .transpose()
    }
}

pub mod decimal_str {
    use rust_decimal::Decimal;
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &Decimal, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Decimal, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse::<Decimal>().map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecordStatus {
    Pending,
    Approved,
    Rejected,
    Merged,
}

impl RecordStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Merged => "merged",
        }
    }
}

impl fmt::Display for RecordStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for RecordStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "rejected" => Ok(Self::Rejected),
            "merged" => Ok(Self::Merged),
            other => Err(format!("Unknown record status: {other}")),
        }
    }
}

impl TryFrom<&str> for RecordStatus {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        s.parse()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    Company,
    Generic,
    Product,
}

impl RecordKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Company => "company",
            Self::Generic => "generic",
            Self::Product => "product",
        }
    }

    pub fn table_name(&self) -> &'static str {
        match self {
            Self::Company => "companies",
            Self::Generic => "generics",
            Self::Product => "products",
        }
    }
}

impl fmt::Display for RecordKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for RecordKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "company" => Ok(Self::Company),
            "generic" => Ok(Self::Generic),
            "product" => Ok(Self::Product),
            other => Err(format!("Unknown record kind: {other}")),
        }
    }
}

impl TryFrom<&str> for RecordKind {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        s.parse()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductType {
    Allopathic,
    Herbal,
    NonMedicine,
}

impl ProductType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Allopathic => "allopathic",
            Self::Herbal => "herbal",
            Self::NonMedicine => "non_medicine",
        }
    }
}

impl fmt::Display for ProductType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for ProductType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "allopathic" => Ok(Self::Allopathic),
            "herbal" => Ok(Self::Herbal),
            "non_medicine" => Ok(Self::NonMedicine),
            other => Err(format!("Unknown product type: {other}")),
        }
    }
}

impl TryFrom<&str> for ProductType {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        s.parse()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueReportStatus {
    Open,
    Resolved,
    Dismissed,
}

impl IssueReportStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Resolved => "resolved",
            Self::Dismissed => "dismissed",
        }
    }
}

impl fmt::Display for IssueReportStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for IssueReportStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "open" => Ok(Self::Open),
            "resolved" => Ok(Self::Resolved),
            "dismissed" => Ok(Self::Dismissed),
            other => Err(format!("Unknown issue report status: {other}")),
        }
    }
}

impl TryFrom<&str> for IssueReportStatus {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        s.parse()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductSummary {
    #[schema(value_type = String)]
    pub id: ProductId,
    pub brand_name: String,
    pub strength: Option<String>,
    pub company_name: String,
    pub generic_name: Option<String>,
    pub dosage_form: Option<String>,
    pub product_type: ProductType,
    #[serde(default, with = "decimal_str_opt")]
    #[schema(value_type = Option<String>)]
    pub reference_unit_mrp: Option<Decimal>,
    pub is_discontinued: bool,
    pub status: RecordStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PackDefault {
    #[schema(value_type = String)]
    pub pack_unit_id: PackUnitId,
    pub pack_unit_name: String,
    pub base_units: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Product {
    #[schema(value_type = String)]
    pub id: ProductId,
    #[schema(value_type = String)]
    pub company_id: CompanyId,
    pub company_name: String,
    #[schema(value_type = Option<String>)]
    pub generic_id: Option<GenericId>,
    pub generic_name: Option<String>,
    #[schema(value_type = Option<String>)]
    pub dosage_form_id: Option<DosageFormId>,
    pub dosage_form: Option<String>,
    pub brand_name: String,
    pub strength: Option<String>,
    pub product_type: ProductType,
    #[serde(default, with = "decimal_str_opt")]
    #[schema(value_type = Option<String>)]
    pub reference_unit_mrp: Option<Decimal>,
    pub is_discontinued: bool,
    pub status: RecordStatus,
    #[schema(value_type = Option<String>)]
    pub merged_into_id: Option<ProductId>,
    pub packs: Vec<PackDefault>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Company {
    #[schema(value_type = String)]
    pub id: CompanyId,
    pub name: String,
    pub medex_id: Option<i32>,
    pub status: RecordStatus,
    #[schema(value_type = Option<String>)]
    pub merged_into_id: Option<CompanyId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Ingredient {
    #[schema(value_type = String)]
    pub id: IngredientId,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TherapeuticClass {
    #[schema(value_type = String)]
    pub id: TherapeuticClassId,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DosageForm {
    #[schema(value_type = String)]
    pub id: DosageFormId,
    pub name: String,
    #[schema(value_type = String)]
    pub base_unit_id: PackUnitId,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PackUnit {
    #[schema(value_type = String)]
    pub id: PackUnitId,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Generic {
    #[schema(value_type = String)]
    pub id: GenericId,
    pub name: String,
    pub medex_id: Option<i32>,
    pub status: RecordStatus,
    #[schema(value_type = Option<String>)]
    pub merged_into_id: Option<GenericId>,
    pub ingredients: Vec<Ingredient>,
    pub classes: Vec<TherapeuticClass>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Monograph {
    #[schema(value_type = String)]
    pub generic_id: GenericId,
    pub indications: Option<String>,
    pub description: Option<String>,
    pub pharmacology: Option<String>,
    pub dosage: Option<String>,
    pub interactions: Option<String>,
    pub contraindications: Option<String>,
    pub side_effects: Option<String>,
    pub pregnancy_and_lactation: Option<String>,
    pub precautions: Option<String>,
    pub storage: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MonographInput {
    pub indications: Option<String>,
    pub description: Option<String>,
    pub pharmacology: Option<String>,
    pub dosage: Option<String>,
    pub interactions: Option<String>,
    pub contraindications: Option<String>,
    pub side_effects: Option<String>,
    pub pregnancy_and_lactation: Option<String>,
    pub precautions: Option<String>,
    pub storage: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Lookups {
    pub dosage_forms: Vec<DosageForm>,
    pub pack_units: Vec<PackUnit>,
    pub therapeutic_classes: Vec<TherapeuticClass>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Submission {
    pub kind: RecordKind,
    #[schema(value_type = String)]
    pub id: uuid::Uuid,
    pub display_name: String,
    pub status: RecordStatus,
    pub review_note: Option<String>,
    #[schema(value_type = Option<String>)]
    pub merged_into_id: Option<uuid::Uuid>,
    pub submitted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SuggestedMatch {
    #[schema(value_type = String)]
    pub id: uuid::Uuid,
    pub name: String,
    pub similarity: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct QueueItem {
    pub kind: RecordKind,
    #[schema(value_type = String)]
    pub id: uuid::Uuid,
    pub display_name: String,
    pub status: RecordStatus,
    #[schema(value_type = Option<String>)]
    pub submitted_by_tenant_id: Option<TenantId>,
    #[schema(value_type = Option<String>)]
    pub submitted_by_user_id: Option<UserId>,
    pub submitted_at: DateTime<Utc>,
    pub suggestions: Vec<SuggestedMatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IssueReport {
    #[schema(value_type = String)]
    pub id: IssueReportId,
    #[schema(value_type = String)]
    pub tenant_id: TenantId,
    #[schema(value_type = String)]
    pub user_id: UserId,
    #[schema(value_type = Option<String>)]
    pub company_id: Option<CompanyId>,
    #[schema(value_type = Option<String>)]
    pub generic_id: Option<GenericId>,
    #[schema(value_type = Option<String>)]
    pub product_id: Option<ProductId>,
    pub message: String,
    pub status: IssueReportStatus,
    pub admin_note: Option<String>,
    #[schema(value_type = Option<String>)]
    pub resolved_by_user_id: Option<UserId>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Reviewed {
    pub kind: RecordKind,
    #[schema(value_type = String)]
    pub id: uuid::Uuid,
    pub status: RecordStatus,
    #[schema(value_type = Option<String>)]
    pub merged_into_id: Option<uuid::Uuid>,
    pub reviewed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NewCompany {
    pub name: String,
    pub medex_id: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NewGeneric {
    pub name: String,
    pub medex_id: Option<i32>,
    #[serde(default)]
    #[schema(value_type = Vec<String>)]
    pub ingredient_ids: Vec<IngredientId>,
    #[serde(default)]
    #[schema(value_type = Vec<String>)]
    pub therapeutic_class_ids: Vec<TherapeuticClassId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NewPack {
    #[schema(value_type = String)]
    pub pack_unit_id: PackUnitId,
    pub base_units: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NewProduct {
    #[schema(value_type = String)]
    pub company_id: CompanyId,
    #[schema(value_type = Option<String>)]
    pub generic_id: Option<GenericId>,
    #[schema(value_type = Option<String>)]
    pub dosage_form_id: Option<DosageFormId>,
    pub brand_name: String,
    pub strength: Option<String>,
    pub product_type: ProductType,
    #[serde(default, with = "decimal_str_opt")]
    #[schema(value_type = Option<String>)]
    pub reference_unit_mrp: Option<Decimal>,
    pub pack: Option<NewPack>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NewIssueReport {
    #[schema(value_type = Option<String>)]
    pub company_id: Option<CompanyId>,
    #[schema(value_type = Option<String>)]
    pub generic_id: Option<GenericId>,
    #[schema(value_type = Option<String>)]
    pub product_id: Option<ProductId>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PatchProduct {
    pub brand_name: Option<String>,
    pub strength: Option<Option<String>>,
    pub product_type: Option<ProductType>,
    #[schema(value_type = Option<String>)]
    pub reference_unit_mrp: Option<Option<Decimal>>,
    pub is_discontinued: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PatchGeneric {
    pub name: Option<String>,
    #[schema(value_type = Option<Vec<String>>)]
    pub ingredient_ids: Option<Vec<IngredientId>>,
    #[schema(value_type = Option<Vec<String>>)]
    pub therapeutic_class_ids: Option<Vec<TherapeuticClassId>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NewNamedLookup {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PatchNamedLookup {
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NewDosageForm {
    pub name: String,
    #[schema(value_type = String)]
    pub base_unit_id: PackUnitId,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PatchDosageForm {
    pub name: Option<String>,
    #[schema(value_type = Option<String>)]
    pub base_unit_id: Option<PackUnitId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RejectSubmissionInput {
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MergeSubmissionInput {
    #[schema(value_type = String)]
    pub target_id: uuid::Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CloseIssueInput {
    pub admin_note: String,
}
