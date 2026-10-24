#![allow(clippy::too_many_arguments)]

use crate::entity;
use crate::types::{
    Company, DosageForm, Generic, GenericId, Ingredient, IssueReport, IssueReportId,
    IssueReportStatus, Lookups, Monograph, MonographInput, NewCompany, NewGeneric, NewIssueReport,
    NewProduct, PackDefault, PackUnit, Product, ProductId, ProductSummary, ProductType, QueueItem,
    RecordKind, RecordStatus, Reviewed, Submission, SuggestedMatch, TherapeuticClass,
};
use api::envelope::ListPage;
use api::extract::Pagination;
use db::{Db, Tx};
use pos_core::context::TenantContext;
use pos_core::error::{AppError, ErrorCode};
use pos_core::ids::{
    CompanyId, DosageFormId, IngredientId, PackUnitId, TenantId, TherapeuticClassId, UserId,
};
use sea_orm::entity::prelude::DateTimeWithTimeZone;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbBackend, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    Statement, Value,
};
use std::str::FromStr;
use uuid::Uuid;

fn map_db_err(e: sea_orm::DbErr) -> AppError {
    let msg = e.to_string();
    if msg.contains("idx_companies_name") {
        return AppError::new(
            ErrorCode::Conflict("duplicate_company_name"),
            "A manufacturer with this name already exists.",
        );
    }
    if msg.contains("idx_generics_name") {
        return AppError::new(
            ErrorCode::Conflict("duplicate_generic_name"),
            "A generic with this name already exists.",
        );
    }
    if msg.contains("idx_products_unique") {
        return AppError::new(
            ErrorCode::Conflict("duplicate_product"),
            "This product is already in the catalogue. Search for it instead of adding it.",
        );
    }
    if msg.contains("chk_issue_reports_single_target") {
        return AppError::bad_request(
            "Choose the one product, generic or manufacturer this report is about.",
        );
    }
    if msg.contains("violates row-level security policy") || msg.contains("permission denied") {
        return AppError::forbidden("Access forbidden: row-level security policy violation");
    }
    db::error::db_error_to_app_error(e)
}

// =============================================================================
// Reads
// =============================================================================

pub async fn is_sellable(db: &Db, tenant: TenantId, id: ProductId) -> Result<bool, AppError> {
    let ctx = TenantContext::new(tenant, UserId::from_uuid(Uuid::nil()), vec![]);
    let tx = Tx::begin(db.conn(), &ctx).await?;

    let prod = entity::products::Entity::find_by_id(id.as_uuid())
        .one(tx.transaction()?)
        .await
        .map_err(map_db_err)?;

    Ok(prod.map(|p| p.status == "approved" || p.status == "pending").unwrap_or(false))
}

pub async fn lookups(db: &Db) -> Result<Lookups, AppError> {
    let forms = entity::dosage_forms::Entity::find()
        .order_by_asc(entity::dosage_forms::Column::Name)
        .all(db.conn())
        .await
        .map_err(map_db_err)?;

    let units = entity::pack_units::Entity::find()
        .order_by_asc(entity::pack_units::Column::Name)
        .all(db.conn())
        .await
        .map_err(map_db_err)?;

    let classes = entity::therapeutic_classes::Entity::find()
        .order_by_asc(entity::therapeutic_classes::Column::Name)
        .all(db.conn())
        .await
        .map_err(map_db_err)?;

    Ok(Lookups {
        dosage_forms: forms
            .into_iter()
            .map(|f| DosageForm {
                id: DosageFormId::from_uuid(f.id),
                name: f.name,
                base_unit_id: PackUnitId::from_uuid(f.base_unit_id),
            })
            .collect(),
        pack_units: units
            .into_iter()
            .map(|u| PackUnit {
                id: PackUnitId::from_uuid(u.id),
                name: u.name,
            })
            .collect(),
        therapeutic_classes: classes
            .into_iter()
            .map(|c| TherapeuticClass {
                id: TherapeuticClassId::from_uuid(c.id),
                name: c.name,
            })
            .collect(),
    })
}

pub async fn search(
    db: &Db,
    tenant: TenantId,
    q: &str,
    page: Pagination,
) -> Result<ListPage<ProductSummary>, AppError> {
    let query_str = q.trim();
    if query_str.is_empty() {
        return Err(AppError::bad_request("Search query cannot be empty"));
    }

    let ctx = TenantContext::new(tenant, UserId::from_uuid(Uuid::nil()), vec![]);
    let tx = Tx::begin(db.conn(), &ctx).await?;

    let limit = (page.limit.clamp(1, 200) + 1) as i64;

    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"
        SELECT
            p.id,
            p.brand_name,
            p.strength,
            c.name AS company_name,
            g.name AS generic_name,
            d.name AS dosage_form,
            p.product_type,
            p.reference_unit_mrp,
            p.is_discontinued,
            p.status,
            greatest(similarity(p.brand_name, $1), coalesce(similarity(g.name, $1), 0.0)) AS sim
        FROM products p
        JOIN companies c ON c.id = p.company_id
        LEFT JOIN generics g ON g.id = p.generic_id
        LEFT JOIN dosage_forms d ON d.id = p.dosage_form_id
        WHERE (p.brand_name % $1 OR (g.name IS NOT NULL AND g.name % $1))
          AND p.is_discontinued = false
          AND p.status <> 'merged'
          AND p.status <> 'rejected'
        ORDER BY sim DESC, p.id ASC
        LIMIT $2
        "#,
        [Value::from(query_str), Value::from(limit)],
    );

    let rows = tx.transaction()?.query_all_raw(stmt).await.map_err(map_db_err)?;

    let has_more = rows.len() as i64 == limit;
    let items: Result<Vec<ProductSummary>, _> = rows
        .into_iter()
        .take(page.limit as usize)
        .map(|r| {
            let id: Uuid = r.try_get("", "id")?;
            let brand_name: String = r.try_get("", "brand_name")?;
            let strength: Option<String> = r.try_get("", "strength")?;
            let company_name: String = r.try_get("", "company_name")?;
            let generic_name: Option<String> = r.try_get("", "generic_name")?;
            let dosage_form: Option<String> = r.try_get("", "dosage_form")?;
            let product_type: String = r.try_get("", "product_type")?;
            let reference_unit_mrp: Option<rust_decimal::Decimal> =
                r.try_get("", "reference_unit_mrp")?;
            let is_discontinued: bool = r.try_get("", "is_discontinued")?;
            let status: String = r.try_get("", "status")?;

            Ok(ProductSummary {
                id: ProductId::from_uuid(id),
                brand_name,
                strength,
                company_name,
                generic_name,
                dosage_form,
                product_type: ProductType::from_str(&product_type)
                    .unwrap_or(ProductType::Allopathic),
                reference_unit_mrp,
                is_discontinued,
                status: RecordStatus::from_str(&status).unwrap_or(RecordStatus::Pending),
            })
        })
        .collect();

    let items = items.map_err(map_db_err)?;
    let next_cursor = if has_more {
        items.last().map(|p| p.id.to_string())
    } else {
        None
    };

    Ok(ListPage::new(items, next_cursor))
}

pub async fn products(
    db: &Db,
    tenant: TenantId,
    company_id: Option<CompanyId>,
    generic_id: Option<GenericId>,
    product_type: Option<ProductType>,
    page: Pagination,
) -> Result<ListPage<ProductSummary>, AppError> {
    let ctx = TenantContext::new(tenant, UserId::from_uuid(Uuid::nil()), vec![]);
    let tx = Tx::begin(db.conn(), &ctx).await?;

    let limit = (page.limit.clamp(1, 200) + 1) as i64;
    let cid = company_id.map(|c| c.as_uuid());
    let gid = generic_id.map(|g| g.as_uuid());
    let ptype = product_type.map(|t| t.as_str().to_string());
    let cursor_id = page.cursor.as_deref().and_then(|c| Uuid::parse_str(c).ok());

    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"
        SELECT
            p.id,
            p.brand_name,
            p.strength,
            c.name AS company_name,
            g.name AS generic_name,
            d.name AS dosage_form,
            p.product_type,
            p.reference_unit_mrp,
            p.is_discontinued,
            p.status
        FROM products p
        JOIN companies c ON c.id = p.company_id
        LEFT JOIN generics g ON g.id = p.generic_id
        LEFT JOIN dosage_forms d ON d.id = p.dosage_form_id
        WHERE ($1::uuid IS NULL OR p.company_id = $1)
          AND ($2::uuid IS NULL OR p.generic_id = $2)
          AND ($3::text IS NULL OR p.product_type = $3)
          AND ($4::uuid IS NULL OR p.id > $4)
        ORDER BY p.id ASC
        LIMIT $5
        "#,
        [
            Value::from(cid),
            Value::from(gid),
            Value::from(ptype),
            Value::from(cursor_id),
            Value::from(limit),
        ],
    );

    let rows = tx.transaction()?.query_all_raw(stmt).await.map_err(map_db_err)?;

    let has_more = rows.len() as i64 == limit;
    let items: Result<Vec<ProductSummary>, _> = rows
        .into_iter()
        .take(page.limit as usize)
        .map(|r| {
            let id: Uuid = r.try_get("", "id")?;
            let brand_name: String = r.try_get("", "brand_name")?;
            let strength: Option<String> = r.try_get("", "strength")?;
            let company_name: String = r.try_get("", "company_name")?;
            let generic_name: Option<String> = r.try_get("", "generic_name")?;
            let dosage_form: Option<String> = r.try_get("", "dosage_form")?;
            let product_type: String = r.try_get("", "product_type")?;
            let reference_unit_mrp: Option<rust_decimal::Decimal> =
                r.try_get("", "reference_unit_mrp")?;
            let is_discontinued: bool = r.try_get("", "is_discontinued")?;
            let status: String = r.try_get("", "status")?;

            Ok(ProductSummary {
                id: ProductId::from_uuid(id),
                brand_name,
                strength,
                company_name,
                generic_name,
                dosage_form,
                product_type: ProductType::from_str(&product_type)
                    .unwrap_or(ProductType::Allopathic),
                reference_unit_mrp,
                is_discontinued,
                status: RecordStatus::from_str(&status).unwrap_or(RecordStatus::Pending),
            })
        })
        .collect();

    let items = items.map_err(map_db_err)?;
    let next_cursor = if has_more {
        items.last().map(|p| p.id.to_string())
    } else {
        None
    };

    Ok(ListPage::new(items, next_cursor))
}

pub async fn product(db: &Db, tenant: TenantId, id: ProductId) -> Result<Product, AppError> {
    let ctx = TenantContext::new(tenant, UserId::from_uuid(Uuid::nil()), vec![]);
    let tx = Tx::begin(db.conn(), &ctx).await?;

    let p = entity::products::Entity::find_by_id(id.as_uuid())
        .one(tx.transaction()?)
        .await
        .map_err(map_db_err)?
        .ok_or_else(|| AppError::not_found("Product not found"))?;

    let company = entity::companies::Entity::find_by_id(p.company_id)
        .one(tx.transaction()?)
        .await
        .map_err(map_db_err)?
        .map(|c| c.name)
        .unwrap_or_default();

    let generic_name = if let Some(gid) = p.generic_id {
        entity::generics::Entity::find_by_id(gid)
            .one(tx.transaction()?)
            .await
            .map_err(map_db_err)?
            .map(|g| g.name)
    } else {
        None
    };

    let dosage_form = if let Some(dfid) = p.dosage_form_id {
        entity::dosage_forms::Entity::find_by_id(dfid)
            .one(tx.transaction()?)
            .await
            .map_err(map_db_err)?
            .map(|df| df.name)
    } else {
        None
    };

    let packs = entity::product_packs::Entity::find()
        .filter(entity::product_packs::Column::ProductId.eq(p.id))
        .all(tx.transaction()?)
        .await
        .map_err(map_db_err)?;

    let mut pack_defaults = Vec::new();
    for pack in packs {
        let unit_name = entity::pack_units::Entity::find_by_id(pack.pack_unit_id)
            .one(tx.transaction()?)
            .await
            .map_err(map_db_err)?
            .map(|u| u.name)
            .unwrap_or_default();
        pack_defaults.push(PackDefault {
            pack_unit_id: PackUnitId::from_uuid(pack.pack_unit_id),
            pack_unit_name: unit_name,
            base_units: pack.base_units,
        });
    }

    Ok(Product {
        id: ProductId::from_uuid(p.id),
        company_id: CompanyId::from_uuid(p.company_id),
        company_name: company,
        generic_id: p.generic_id.map(GenericId::from_uuid),
        generic_name,
        dosage_form_id: p.dosage_form_id.map(DosageFormId::from_uuid),
        dosage_form,
        brand_name: p.brand_name,
        strength: p.strength,
        product_type: ProductType::from_str(&p.product_type).unwrap_or(ProductType::Allopathic),
        reference_unit_mrp: p.reference_unit_mrp,
        is_discontinued: p.is_discontinued,
        status: RecordStatus::from_str(&p.status).unwrap_or(RecordStatus::Pending),
        merged_into_id: p.merged_into_id.map(ProductId::from_uuid),
        packs: pack_defaults,
        created_at: p.created_at.into(),
        updated_at: p.updated_at.into(),
    })
}

pub async fn generic(db: &Db, tenant: TenantId, id: GenericId) -> Result<Generic, AppError> {
    let ctx = TenantContext::new(tenant, UserId::from_uuid(Uuid::nil()), vec![]);
    let tx = Tx::begin(db.conn(), &ctx).await?;

    let g = entity::generics::Entity::find_by_id(id.as_uuid())
        .one(tx.transaction()?)
        .await
        .map_err(map_db_err)?
        .ok_or_else(|| AppError::not_found("Generic not found"))?;

    let g_ingredients = entity::generic_ingredients::Entity::find()
        .filter(entity::generic_ingredients::Column::GenericId.eq(g.id))
        .order_by_asc(entity::generic_ingredients::Column::Position)
        .all(tx.transaction()?)
        .await
        .map_err(map_db_err)?;

    let mut ingredients = Vec::new();
    for gi in g_ingredients {
        if let Some(ing) = entity::ingredients::Entity::find_by_id(gi.ingredient_id)
            .one(tx.transaction()?)
            .await
            .map_err(map_db_err)?
        {
            ingredients.push(Ingredient {
                id: IngredientId::from_uuid(ing.id),
                name: ing.name,
            });
        }
    }

    let g_classes = entity::generic_therapeutic_classes::Entity::find()
        .filter(entity::generic_therapeutic_classes::Column::GenericId.eq(g.id))
        .all(tx.transaction()?)
        .await
        .map_err(map_db_err)?;

    let mut classes = Vec::new();
    for gc in g_classes {
        if let Some(tc) = entity::therapeutic_classes::Entity::find_by_id(gc.therapeutic_class_id)
            .one(tx.transaction()?)
            .await
            .map_err(map_db_err)?
        {
            classes.push(TherapeuticClass {
                id: TherapeuticClassId::from_uuid(tc.id),
                name: tc.name,
            });
        }
    }

    Ok(Generic {
        id: GenericId::from_uuid(g.id),
        name: g.name,
        medex_id: g.medex_id,
        status: RecordStatus::from_str(&g.status).unwrap_or(RecordStatus::Pending),
        merged_into_id: g.merged_into_id.map(GenericId::from_uuid),
        ingredients,
        classes,
        created_at: g.created_at.into(),
        updated_at: g.updated_at.into(),
    })
}

pub async fn monograph(db: &Db, id: GenericId) -> Result<Option<Monograph>, AppError> {
    let exists = entity::generics::Entity::find_by_id(id.as_uuid())
        .one(db.conn())
        .await
        .map_err(map_db_err)?;
    if exists.is_none() {
        return Err(AppError::not_found("Generic not found"));
    }

    let mono = entity::generic_monographs::Entity::find_by_id(id.as_uuid())
        .one(db.conn())
        .await
        .map_err(map_db_err)?;

    match mono {
        Some(m) => {
            let has_text = m.indications.is_some()
                || m.description.is_some()
                || m.pharmacology.is_some()
                || m.dosage.is_some()
                || m.interactions.is_some()
                || m.contraindications.is_some()
                || m.side_effects.is_some()
                || m.pregnancy_and_lactation.is_some()
                || m.precautions.is_some()
                || m.storage.is_some();

            if !has_text {
                Ok(None)
            } else {
                Ok(Some(Monograph {
                    generic_id: GenericId::from_uuid(m.generic_id),
                    indications: m.indications,
                    description: m.description,
                    pharmacology: m.pharmacology,
                    dosage: m.dosage,
                    interactions: m.interactions,
                    contraindications: m.contraindications,
                    side_effects: m.side_effects,
                    pregnancy_and_lactation: m.pregnancy_and_lactation,
                    precautions: m.precautions,
                    storage: m.storage,
                    updated_at: m.updated_at.into(),
                }))
            }
        }
        None => Ok(None),
    }
}

pub async fn companies(
    db: &Db,
    tenant: TenantId,
    page: Pagination,
) -> Result<ListPage<Company>, AppError> {
    let ctx = TenantContext::new(tenant, UserId::from_uuid(Uuid::nil()), vec![]);
    let tx = Tx::begin(db.conn(), &ctx).await?;

    let limit = page.limit.clamp(1, 200) as u64;
    let mut query = entity::companies::Entity::find()
        .order_by_asc(entity::companies::Column::Id)
        .limit(limit + 1);

    if let Some(ref c) = page.cursor {
        if let Ok(cursor_id) = Uuid::parse_str(c) {
            query = query.filter(entity::companies::Column::Id.gt(cursor_id));
        }
    }

    let rows = query.all(tx.transaction()?).await.map_err(map_db_err)?;
    let has_more = rows.len() as u64 > limit;
    let items: Vec<Company> = rows
        .into_iter()
        .take(limit as usize)
        .map(|m| Company {
            id: CompanyId::from_uuid(m.id),
            name: m.name,
            medex_id: m.medex_id,
            status: RecordStatus::from_str(&m.status).unwrap_or(RecordStatus::Pending),
            merged_into_id: m.merged_into_id.map(CompanyId::from_uuid),
            created_at: m.created_at.into(),
            updated_at: m.updated_at.into(),
        })
        .collect();

    let next_cursor = if has_more {
        items.last().map(|c| c.id.to_string())
    } else {
        None
    };

    Ok(ListPage::new(items, next_cursor))
}

pub async fn submissions(
    db: &Db,
    tenant: TenantId,
    page: Pagination,
) -> Result<ListPage<Submission>, AppError> {
    let ctx = TenantContext::new(tenant, UserId::from_uuid(Uuid::nil()), vec![]);
    let tx = Tx::begin(db.conn(), &ctx).await?;

    let limit = (page.limit.clamp(1, 200) + 1) as i64;
    let cursor_id = page.cursor.as_deref().and_then(|c| Uuid::parse_str(c).ok());

    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"
        SELECT * FROM (
            SELECT
                'company' AS kind,
                id,
                name AS display_name,
                status,
                review_note,
                merged_into_id,
                created_at AS submitted_at
            FROM companies
            WHERE submitted_by_tenant_id = $1
            UNION ALL
            SELECT
                'generic' AS kind,
                id,
                name AS display_name,
                status,
                review_note,
                merged_into_id,
                created_at AS submitted_at
            FROM generics
            WHERE submitted_by_tenant_id = $1
            UNION ALL
            SELECT
                'product' AS kind,
                id,
                brand_name AS display_name,
                status,
                review_note,
                merged_into_id,
                created_at AS submitted_at
            FROM products
            WHERE submitted_by_tenant_id = $1
        ) sub
        WHERE ($2::uuid IS NULL OR sub.id > $2)
        ORDER BY sub.submitted_at DESC, sub.id ASC
        LIMIT $3
        "#,
        [
            Value::from(tenant.as_uuid()),
            Value::from(cursor_id),
            Value::from(limit),
        ],
    );

    let rows = tx.transaction()?.query_all_raw(stmt).await.map_err(map_db_err)?;

    let has_more = rows.len() as i64 == limit;
    let items: Result<Vec<Submission>, _> = rows
        .into_iter()
        .take(page.limit as usize)
        .map(|r| {
            let kind: String = r.try_get("", "kind")?;
            let id: Uuid = r.try_get("", "id")?;
            let display_name: String = r.try_get("", "display_name")?;
            let status: String = r.try_get("", "status")?;
            let review_note: Option<String> = r.try_get("", "review_note")?;
            let merged_into_id: Option<Uuid> = r.try_get("", "merged_into_id")?;
            let submitted_at: DateTimeWithTimeZone = r.try_get("", "submitted_at")?;

            Ok(Submission {
                kind: RecordKind::from_str(&kind).unwrap_or(RecordKind::Product),
                id,
                display_name,
                status: RecordStatus::from_str(&status).unwrap_or(RecordStatus::Pending),
                review_note,
                merged_into_id,
                submitted_at: submitted_at.into(),
            })
        })
        .collect();

    let items = items.map_err(map_db_err)?;
    let next_cursor = if has_more {
        items.last().map(|s| s.id.to_string())
    } else {
        None
    };

    Ok(ListPage::new(items, next_cursor))
}

pub async fn issues(
    db: &Db,
    tenant: TenantId,
    page: Pagination,
) -> Result<ListPage<IssueReport>, AppError> {
    let ctx = TenantContext::new(tenant, UserId::from_uuid(Uuid::nil()), vec![]);
    let tx = Tx::begin(db.conn(), &ctx).await?;

    let limit = page.limit.clamp(1, 200) as u64;
    let mut query = entity::catalog_issue_reports::Entity::find()
        .order_by_desc(entity::catalog_issue_reports::Column::CreatedAt)
        .limit(limit + 1);

    if let Some(ref c) = page.cursor {
        if let Ok(cursor_id) = Uuid::parse_str(c) {
            query = query.filter(entity::catalog_issue_reports::Column::Id.gt(cursor_id));
        }
    }

    let rows = query.all(tx.transaction()?).await.map_err(map_db_err)?;
    let has_more = rows.len() as u64 > limit;
    let items: Vec<IssueReport> = rows
        .into_iter()
        .take(limit as usize)
        .map(|r| IssueReport {
            id: IssueReportId::from_uuid(r.id),
            tenant_id: TenantId::from_uuid(r.tenant_id),
            user_id: UserId::from_uuid(r.user_id),
            company_id: r.company_id.map(CompanyId::from_uuid),
            generic_id: r.generic_id.map(GenericId::from_uuid),
            product_id: r.product_id.map(ProductId::from_uuid),
            message: r.message,
            status: IssueReportStatus::from_str(&r.status).unwrap_or(IssueReportStatus::Open),
            admin_note: r.admin_note,
            resolved_by_user_id: r.resolved_by_user_id.map(UserId::from_uuid),
            resolved_at: r.resolved_at.map(Into::into),
            created_at: r.created_at.into(),
            updated_at: r.updated_at.into(),
        })
        .collect();

    let next_cursor = if has_more {
        items.last().map(|i| i.id.to_string())
    } else {
        None
    };

    Ok(ListPage::new(items, next_cursor))
}

// =============================================================================
// Pharmacy Submissions (Commands)
// =============================================================================

pub async fn submit_company(tx: &Tx, input: NewCompany) -> Result<Company, AppError> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::bad_request("Company name cannot be empty"));
    }

    let id = Uuid::now_v7();
    let tenant_id = tx.tenant_id()?.as_uuid();
    let user_id = tx.user_id()?.as_uuid();

    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"
        INSERT INTO companies (id, name, medex_id, status, submitted_by_tenant_id, submitted_by_user_id)
        VALUES ($1, $2, $3, 'pending', $4, $5)
        RETURNING id, name, medex_id, status, merged_into_id, created_at, updated_at
        "#,
        [
            Value::from(id),
            Value::from(name),
            Value::from(input.medex_id),
            Value::from(tenant_id),
            Value::from(user_id),
        ],
    );

    let row = tx
        .transaction()?
        .query_one_raw(stmt)
        .await
        .map_err(map_db_err)?
        .ok_or_else(|| AppError::internal("Insert company returned no row", ""))?;

    let row_id: Uuid = row.try_get("", "id").map_err(map_db_err)?;
    let row_name: String = row.try_get("", "name").map_err(map_db_err)?;
    let row_medex: Option<i32> = row.try_get("", "medex_id").map_err(map_db_err)?;
    let row_status: String = row.try_get("", "status").map_err(map_db_err)?;
    let row_merged: Option<Uuid> = row.try_get("", "merged_into_id").map_err(map_db_err)?;
    let row_created: DateTimeWithTimeZone = row.try_get("", "created_at").map_err(map_db_err)?;
    let row_updated: DateTimeWithTimeZone = row.try_get("", "updated_at").map_err(map_db_err)?;

    Ok(Company {
        id: CompanyId::from_uuid(row_id),
        name: row_name,
        medex_id: row_medex,
        status: RecordStatus::from_str(&row_status).unwrap_or(RecordStatus::Pending),
        merged_into_id: row_merged.map(CompanyId::from_uuid),
        created_at: row_created.into(),
        updated_at: row_updated.into(),
    })
}

pub async fn submit_generic(tx: &Tx, input: NewGeneric) -> Result<Generic, AppError> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::bad_request("Generic name cannot be empty"));
    }

    let id = Uuid::now_v7();
    let tenant_id = tx.tenant_id()?.as_uuid();
    let user_id = tx.user_id()?.as_uuid();

    for ing_id in &input.ingredient_ids {
        let exists = entity::ingredients::Entity::find_by_id(ing_id.as_uuid())
            .one(tx.transaction()?)
            .await
            .map_err(map_db_err)?;
        if exists.is_none() {
            return Err(AppError::not_found(format!(
                "Ingredient {ing_id} not found"
            )));
        }
    }

    for tc_id in &input.therapeutic_class_ids {
        let exists = entity::therapeutic_classes::Entity::find_by_id(tc_id.as_uuid())
            .one(tx.transaction()?)
            .await
            .map_err(map_db_err)?;
        if exists.is_none() {
            return Err(AppError::not_found(format!(
                "Therapeutic class {tc_id} not found"
            )));
        }
    }

    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"
        INSERT INTO generics (id, name, medex_id, status, submitted_by_tenant_id, submitted_by_user_id)
        VALUES ($1, $2, $3, 'pending', $4, $5)
        RETURNING id, name, medex_id, status, merged_into_id, created_at, updated_at
        "#,
        [
            Value::from(id),
            Value::from(name),
            Value::from(input.medex_id),
            Value::from(tenant_id),
            Value::from(user_id),
        ],
    );

    let row = tx
        .transaction()?
        .query_one_raw(stmt)
        .await
        .map_err(map_db_err)?
        .ok_or_else(|| AppError::internal("Insert generic returned no row", ""))?;

    let row_id: Uuid = row.try_get("", "id").map_err(map_db_err)?;
    let row_name: String = row.try_get("", "name").map_err(map_db_err)?;
    let row_medex: Option<i32> = row.try_get("", "medex_id").map_err(map_db_err)?;
    let row_status: String = row.try_get("", "status").map_err(map_db_err)?;
    let row_merged: Option<Uuid> = row.try_get("", "merged_into_id").map_err(map_db_err)?;
    let row_created: DateTimeWithTimeZone = row.try_get("", "created_at").map_err(map_db_err)?;
    let row_updated: DateTimeWithTimeZone = row.try_get("", "updated_at").map_err(map_db_err)?;

    let mut ingredients = Vec::new();
    for (pos, ing_id) in input.ingredient_ids.iter().enumerate() {
        let pos_i32 = i32::try_from(pos + 1).unwrap_or(i32::MAX);
        let ins_stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            "INSERT INTO generic_ingredients (generic_id, ingredient_id, position) VALUES ($1, $2, $3)",
            [Value::from(id), Value::from(ing_id.as_uuid()), Value::from(pos_i32)],
        );
        tx.transaction()?.execute_raw(ins_stmt).await.map_err(map_db_err)?;

        let ing = entity::ingredients::Entity::find_by_id(ing_id.as_uuid())
            .one(tx.transaction()?)
            .await
            .map_err(map_db_err)?;
        let name = ing.map(|i| i.name).unwrap_or_default();
        ingredients.push(Ingredient {
            id: *ing_id,
            name,
        });
    }

    let mut classes = Vec::new();
    for tc_id in &input.therapeutic_class_ids {
        let ins_tc = Statement::from_sql_and_values(
            DbBackend::Postgres,
            "INSERT INTO generic_therapeutic_classes (generic_id, therapeutic_class_id) VALUES ($1, $2)",
            [Value::from(id), Value::from(tc_id.as_uuid())],
        );
        tx.transaction()?.execute_raw(ins_tc).await.map_err(map_db_err)?;

        let tc = entity::therapeutic_classes::Entity::find_by_id(tc_id.as_uuid())
            .one(tx.transaction()?)
            .await
            .map_err(map_db_err)?;
        let name = tc.map(|t| t.name).unwrap_or_default();
        classes.push(TherapeuticClass {
            id: *tc_id,
            name,
        });
    }

    Ok(Generic {
        id: GenericId::from_uuid(row_id),
        name: row_name,
        medex_id: row_medex,
        status: RecordStatus::from_str(&row_status).unwrap_or(RecordStatus::Pending),
        merged_into_id: row_merged.map(GenericId::from_uuid),
        ingredients,
        classes,
        created_at: row_created.into(),
        updated_at: row_updated.into(),
    })
}

pub async fn submit_product(tx: &Tx, input: NewProduct) -> Result<Product, AppError> {
    let brand_name = input.brand_name.trim();

    if input.product_type != ProductType::NonMedicine {
        let mut missing_fields = Vec::new();
        if input.generic_id.is_none() {
            missing_fields.push("generic_id");
        }
        if input.dosage_form_id.is_none() {
            missing_fields.push("dosage_form_id");
        }
        if input.strength.as_ref().map(|s| s.trim().is_empty()).unwrap_or(true) {
            missing_fields.push("strength");
        }
        if brand_name.is_empty() {
            missing_fields.push("brand_name");
        }

        if !missing_fields.is_empty() {
            let mut err = AppError::bad_request("Fill in the manufacturer, generic, strength and dosage form.");
            for field in missing_fields {
                err = err.detail_field(field, "Required for medicine");
            }
            return Err(err);
        }
    } else if brand_name.is_empty() {
        return Err(AppError::bad_request("brand_name is required").detail_field("brand_name", "Required"));
    }

    let company = entity::companies::Entity::find_by_id(input.company_id.as_uuid())
        .one(tx.transaction()?)
        .await
        .map_err(map_db_err)?
        .ok_or_else(|| AppError::not_found("Company not found or not visible"))?;

    let generic_name = if let Some(gid) = input.generic_id {
        let g = entity::generics::Entity::find_by_id(gid.as_uuid())
            .one(tx.transaction()?)
            .await
            .map_err(map_db_err)?
            .ok_or_else(|| AppError::not_found("Generic not found or not visible"))?;
        Some(g.name)
    } else {
        None
    };

    let dosage_form_name = if let Some(dfid) = input.dosage_form_id {
        let df = entity::dosage_forms::Entity::find_by_id(dfid.as_uuid())
            .one(tx.transaction()?)
            .await
            .map_err(map_db_err)?
            .ok_or_else(|| AppError::not_found("Dosage form not found"))?;
        Some(df.name)
    } else {
        None
    };

    if let Some(ref pack) = input.pack {
        let exists = entity::pack_units::Entity::find_by_id(pack.pack_unit_id.as_uuid())
            .one(tx.transaction()?)
            .await
            .map_err(map_db_err)?;
        if exists.is_none() {
            return Err(AppError::not_found("Pack unit not found"));
        }
    }

    let id = Uuid::now_v7();
    let tenant_id = tx.tenant_id()?.as_uuid();
    let user_id = tx.user_id()?.as_uuid();

    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"
        INSERT INTO products (
            id, company_id, generic_id, dosage_form_id, brand_name,
            strength, product_type, reference_unit_mrp, status,
            submitted_by_tenant_id, submitted_by_user_id
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'pending', $9, $10)
        RETURNING id, company_id, generic_id, dosage_form_id, brand_name,
                  strength, product_type, reference_unit_mrp, is_discontinued,
                  status, merged_into_id, created_at, updated_at
        "#,
        [
            Value::from(id),
            Value::from(input.company_id.as_uuid()),
            Value::from(input.generic_id.map(|g| g.as_uuid())),
            Value::from(input.dosage_form_id.map(|d| d.as_uuid())),
            Value::from(brand_name),
            Value::from(input.strength),
            Value::from(input.product_type.as_str()),
            Value::from(input.reference_unit_mrp),
            Value::from(tenant_id),
            Value::from(user_id),
        ],
    );

    let row = tx
        .transaction()?
        .query_one_raw(stmt)
        .await
        .map_err(map_db_err)?
        .ok_or_else(|| AppError::internal("Insert product returned no row", ""))?;

    let row_id: Uuid = row.try_get("", "id").map_err(map_db_err)?;
    let row_cid: Uuid = row.try_get("", "company_id").map_err(map_db_err)?;
    let row_gid: Option<Uuid> = row.try_get("", "generic_id").map_err(map_db_err)?;
    let row_dfid: Option<Uuid> = row.try_get("", "dosage_form_id").map_err(map_db_err)?;
    let row_brand: String = row.try_get("", "brand_name").map_err(map_db_err)?;
    let row_strength: Option<String> = row.try_get("", "strength").map_err(map_db_err)?;
    let row_type: String = row.try_get("", "product_type").map_err(map_db_err)?;
    let row_mrp: Option<rust_decimal::Decimal> =
        row.try_get("", "reference_unit_mrp").map_err(map_db_err)?;
    let row_disc: bool = row.try_get("", "is_discontinued").map_err(map_db_err)?;
    let row_status: String = row.try_get("", "status").map_err(map_db_err)?;
    let row_merged: Option<Uuid> = row.try_get("", "merged_into_id").map_err(map_db_err)?;
    let row_created: DateTimeWithTimeZone = row.try_get("", "created_at").map_err(map_db_err)?;
    let row_updated: DateTimeWithTimeZone = row.try_get("", "updated_at").map_err(map_db_err)?;

    let mut pack_defaults = Vec::new();
    if let Some(pack) = input.pack {
        let ins_pack = Statement::from_sql_and_values(
            DbBackend::Postgres,
            "INSERT INTO product_packs (product_id, pack_unit_id, base_units) VALUES ($1, $2, $3)",
            [Value::from(id), Value::from(pack.pack_unit_id.as_uuid()), Value::from(pack.base_units)],
        );
        tx.transaction()?.execute_raw(ins_pack).await.map_err(map_db_err)?;

        let unit_name = entity::pack_units::Entity::find_by_id(pack.pack_unit_id.as_uuid())
            .one(tx.transaction()?)
            .await
            .map_err(map_db_err)?
            .map(|u| u.name)
            .unwrap_or_default();

        pack_defaults.push(PackDefault {
            pack_unit_id: pack.pack_unit_id,
            pack_unit_name: unit_name,
            base_units: pack.base_units,
        });
    }

    Ok(Product {
        id: ProductId::from_uuid(row_id),
        company_id: CompanyId::from_uuid(row_cid),
        company_name: company.name,
        generic_id: row_gid.map(GenericId::from_uuid),
        generic_name,
        dosage_form_id: row_dfid.map(DosageFormId::from_uuid),
        dosage_form: dosage_form_name,
        brand_name: row_brand,
        strength: row_strength,
        product_type: ProductType::from_str(&row_type).unwrap_or(ProductType::Allopathic),
        reference_unit_mrp: row_mrp,
        is_discontinued: row_disc,
        status: RecordStatus::from_str(&row_status).unwrap_or(RecordStatus::Pending),
        merged_into_id: row_merged.map(ProductId::from_uuid),
        packs: pack_defaults,
        created_at: row_created.into(),
        updated_at: row_updated.into(),
    })
}

pub async fn report_issue(tx: &Tx, input: NewIssueReport) -> Result<IssueReport, AppError> {
    let msg = input.message.trim();
    if msg.is_empty() {
        return Err(AppError::bad_request("Issue report message cannot be empty"));
    }

    let targets_count = [
        input.company_id.is_some(),
        input.generic_id.is_some(),
        input.product_id.is_some(),
    ]
    .iter()
    .filter(|&&b| b)
    .count();

    if targets_count != 1 {
        return Err(AppError::bad_request(
            "Choose the one product, generic or manufacturer this report is about.",
        ));
    }

    if let Some(cid) = input.company_id {
        let exists = entity::companies::Entity::find_by_id(cid.as_uuid())
            .one(tx.transaction()?)
            .await
            .map_err(map_db_err)?;
        if exists.is_none() {
            return Err(AppError::not_found("Target company not found"));
        }
    }
    if let Some(gid) = input.generic_id {
        let exists = entity::generics::Entity::find_by_id(gid.as_uuid())
            .one(tx.transaction()?)
            .await
            .map_err(map_db_err)?;
        if exists.is_none() {
            return Err(AppError::not_found("Target generic not found"));
        }
    }
    if let Some(pid) = input.product_id {
        let exists = entity::products::Entity::find_by_id(pid.as_uuid())
            .one(tx.transaction()?)
            .await
            .map_err(map_db_err)?;
        if exists.is_none() {
            return Err(AppError::not_found("Target product not found"));
        }
    }

    let id = Uuid::now_v7();
    let tenant_id = tx.tenant_id()?.as_uuid();
    let user_id = tx.user_id()?.as_uuid();

    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"
        INSERT INTO catalog_issue_reports (
            id, tenant_id, user_id, company_id, generic_id, product_id, message, status
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, 'open')
        RETURNING id, tenant_id, user_id, company_id, generic_id, product_id,
                  message, status, admin_note, resolved_by_user_id, resolved_at,
                  created_at, updated_at
        "#,
        [
            Value::from(id),
            Value::from(tenant_id),
            Value::from(user_id),
            Value::from(input.company_id.map(|c| c.as_uuid())),
            Value::from(input.generic_id.map(|g| g.as_uuid())),
            Value::from(input.product_id.map(|p| p.as_uuid())),
            Value::from(msg),
        ],
    );

    let row = tx
        .transaction()?
        .query_one_raw(stmt)
        .await
        .map_err(map_db_err)?
        .ok_or_else(|| AppError::internal("Insert issue report returned no row", ""))?;

    let row_id: Uuid = row.try_get("", "id").map_err(map_db_err)?;
    let row_tid: Uuid = row.try_get("", "tenant_id").map_err(map_db_err)?;
    let row_uid: Uuid = row.try_get("", "user_id").map_err(map_db_err)?;
    let row_cid: Option<Uuid> = row.try_get("", "company_id").map_err(map_db_err)?;
    let row_gid: Option<Uuid> = row.try_get("", "generic_id").map_err(map_db_err)?;
    let row_pid: Option<Uuid> = row.try_get("", "product_id").map_err(map_db_err)?;
    let row_msg: String = row.try_get("", "message").map_err(map_db_err)?;
    let row_status: String = row.try_get("", "status").map_err(map_db_err)?;
    let row_note: Option<String> = row.try_get("", "admin_note").map_err(map_db_err)?;
    let row_ruid: Option<Uuid> = row.try_get("", "resolved_by_user_id").map_err(map_db_err)?;
    let row_rat: Option<DateTimeWithTimeZone> =
        row.try_get("", "resolved_at").map_err(map_db_err)?;
    let row_created: DateTimeWithTimeZone = row.try_get("", "created_at").map_err(map_db_err)?;
    let row_updated: DateTimeWithTimeZone = row.try_get("", "updated_at").map_err(map_db_err)?;

    Ok(IssueReport {
        id: IssueReportId::from_uuid(row_id),
        tenant_id: TenantId::from_uuid(row_tid),
        user_id: UserId::from_uuid(row_uid),
        company_id: row_cid.map(CompanyId::from_uuid),
        generic_id: row_gid.map(GenericId::from_uuid),
        product_id: row_pid.map(ProductId::from_uuid),
        message: row_msg,
        status: IssueReportStatus::from_str(&row_status).unwrap_or(IssueReportStatus::Open),
        admin_note: row_note,
        resolved_by_user_id: row_ruid.map(UserId::from_uuid),
        resolved_at: row_rat.map(Into::into),
        created_at: row_created.into(),
        updated_at: row_updated.into(),
    })
}

// =============================================================================
// Platform Review Surface
// =============================================================================

pub async fn approve(
    tx: &Tx,
    kind: RecordKind,
    id: Uuid,
    by: UserId,
) -> Result<Reviewed, AppError> {
    let now = chrono::Utc::now();
    let table = kind.table_name();
    let sql = format!(
        r#"
        UPDATE {table}
        SET status = 'approved', reviewed_by_user_id = $2, reviewed_at = $3, updated_at = $3
        WHERE id = $1 AND status = 'pending'
        RETURNING id, status, merged_into_id, reviewed_at
        "#
    );

    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        &sql,
        [
            Value::from(id),
            Value::from(by.as_uuid()),
            Value::from(now.naive_utc()),
        ],
    );

    let row = tx.transaction()?.query_one_raw(stmt).await.map_err(map_db_err)?;
    let r = row.ok_or_else(|| {
        AppError::new(
            ErrorCode::Conflict("already_reviewed"),
            "This submission has already been reviewed.",
        )
    })?;

    let row_id: Uuid = r.try_get("", "id").map_err(map_db_err)?;
    let row_status: String = r.try_get("", "status").map_err(map_db_err)?;
    let row_merged: Option<Uuid> = r.try_get("", "merged_into_id").map_err(map_db_err)?;
    let row_rat: Option<DateTimeWithTimeZone> =
        r.try_get("", "reviewed_at").map_err(map_db_err)?;

    Ok(Reviewed {
        kind,
        id: row_id,
        status: RecordStatus::from_str(&row_status).unwrap_or(RecordStatus::Approved),
        merged_into_id: row_merged,
        reviewed_at: row_rat.map(Into::into),
    })
}

pub async fn reject(
    tx: &Tx,
    kind: RecordKind,
    id: Uuid,
    by: UserId,
    note: String,
) -> Result<Reviewed, AppError> {
    let note = note.trim();
    if note.is_empty() {
        return Err(AppError::bad_request("Rejection note is required"));
    }

    let now = chrono::Utc::now();
    let table = kind.table_name();
    let sql = format!(
        r#"
        UPDATE {table}
        SET status = 'rejected', review_note = $2, reviewed_by_user_id = $3, reviewed_at = $4, updated_at = $4
        WHERE id = $1 AND status = 'pending'
        RETURNING id, status, merged_into_id, reviewed_at
        "#
    );

    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        &sql,
        [
            Value::from(id),
            Value::from(note),
            Value::from(by.as_uuid()),
            Value::from(now.naive_utc()),
        ],
    );

    let row = tx.transaction()?.query_one_raw(stmt).await.map_err(map_db_err)?;
    let r = row.ok_or_else(|| {
        AppError::new(
            ErrorCode::Conflict("already_reviewed"),
            "This submission has already been reviewed.",
        )
    })?;

    let row_id: Uuid = r.try_get("", "id").map_err(map_db_err)?;
    let row_status: String = r.try_get("", "status").map_err(map_db_err)?;
    let row_merged: Option<Uuid> = r.try_get("", "merged_into_id").map_err(map_db_err)?;
    let row_rat: Option<DateTimeWithTimeZone> =
        r.try_get("", "reviewed_at").map_err(map_db_err)?;

    Ok(Reviewed {
        kind,
        id: row_id,
        status: RecordStatus::from_str(&row_status).unwrap_or(RecordStatus::Rejected),
        merged_into_id: row_merged,
        reviewed_at: row_rat.map(Into::into),
    })
}

pub async fn merge(
    tx: &Tx,
    kind: RecordKind,
    id: Uuid,
    into: Uuid,
    by: UserId,
) -> Result<Reviewed, AppError> {
    if id == into {
        return Err(AppError::new(
            ErrorCode::Conflict("invalid_merge_target"),
            "A row cannot merge into itself.",
        ));
    }

    let table = kind.table_name();

    // Check target exists and is not merged
    let check_sql = format!("SELECT status FROM {table} WHERE id = $1");
    let check_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        &check_sql,
        [Value::from(into)],
    );
    let target = tx.transaction()?.query_one_raw(check_stmt).await.map_err(map_db_err)?;
    let target_row = target.ok_or_else(|| {
        AppError::new(
            ErrorCode::Conflict("invalid_merge_target"),
            "Target not found.",
        )
    })?;
    let target_status: String = target_row.try_get("", "status").map_err(map_db_err)?;
    if target_status == "merged" {
        return Err(AppError::new(
            ErrorCode::Conflict("invalid_merge_target"),
            "Pick a product that is still active to merge into.",
        ));
    }

    let now = chrono::Utc::now();
    let sql = format!(
        r#"
        UPDATE {table}
        SET status = 'merged', merged_into_id = $2, reviewed_by_user_id = $3, reviewed_at = $4, updated_at = $4
        WHERE id = $1 AND status = 'pending'
        RETURNING id, status, merged_into_id, reviewed_at
        "#
    );

    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        &sql,
        [
            Value::from(id),
            Value::from(into),
            Value::from(by.as_uuid()),
            Value::from(now.naive_utc()),
        ],
    );

    let row = tx.transaction()?.query_one_raw(stmt).await.map_err(map_db_err)?;
    let r = row.ok_or_else(|| {
        AppError::new(
            ErrorCode::Conflict("already_reviewed"),
            "This submission has already been reviewed.",
        )
    })?;

    let row_id: Uuid = r.try_get("", "id").map_err(map_db_err)?;
    let row_status: String = r.try_get("", "status").map_err(map_db_err)?;
    let row_merged: Option<Uuid> = r.try_get("", "merged_into_id").map_err(map_db_err)?;
    let row_rat: Option<DateTimeWithTimeZone> =
        r.try_get("", "reviewed_at").map_err(map_db_err)?;

    Ok(Reviewed {
        kind,
        id: row_id,
        status: RecordStatus::from_str(&row_status).unwrap_or(RecordStatus::Merged),
        merged_into_id: row_merged,
        reviewed_at: row_rat.map(Into::into),
    })
}

pub async fn upsert_monograph(
    tx: &Tx,
    id: GenericId,
    input: MonographInput,
) -> Result<Monograph, AppError> {
    let exists = entity::generics::Entity::find_by_id(id.as_uuid())
        .one(tx.transaction()?)
        .await
        .map_err(map_db_err)?;
    if exists.is_none() {
        return Err(AppError::not_found("Generic not found"));
    }

    let now = chrono::Utc::now();
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"
        INSERT INTO generic_monographs (
            generic_id, indications, description, pharmacology, dosage,
            interactions, contraindications, side_effects, pregnancy_and_lactation,
            precautions, storage, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        ON CONFLICT (generic_id) DO UPDATE SET
            indications = EXCLUDED.indications,
            description = EXCLUDED.description,
            pharmacology = EXCLUDED.pharmacology,
            dosage = EXCLUDED.dosage,
            interactions = EXCLUDED.interactions,
            contraindications = EXCLUDED.contraindications,
            side_effects = EXCLUDED.side_effects,
            pregnancy_and_lactation = EXCLUDED.pregnancy_and_lactation,
            precautions = EXCLUDED.precautions,
            storage = EXCLUDED.storage,
            updated_at = EXCLUDED.updated_at
        RETURNING generic_id, indications, description, pharmacology, dosage,
                  interactions, contraindications, side_effects, pregnancy_and_lactation,
                  precautions, storage, updated_at
        "#,
        [
            Value::from(id.as_uuid()),
            Value::from(input.indications),
            Value::from(input.description),
            Value::from(input.pharmacology),
            Value::from(input.dosage),
            Value::from(input.interactions),
            Value::from(input.contraindications),
            Value::from(input.side_effects),
            Value::from(input.pregnancy_and_lactation),
            Value::from(input.precautions),
            Value::from(input.storage),
            Value::from(now.naive_utc()),
        ],
    );

    let row = tx
        .transaction()?
        .query_one_raw(stmt)
        .await
        .map_err(map_db_err)?
        .ok_or_else(|| AppError::internal("Upsert monograph returned no row", ""))?;

    let row_gid: Uuid = row.try_get("", "generic_id").map_err(map_db_err)?;
    let indications: Option<String> = row.try_get("", "indications").map_err(map_db_err)?;
    let description: Option<String> = row.try_get("", "description").map_err(map_db_err)?;
    let pharmacology: Option<String> = row.try_get("", "pharmacology").map_err(map_db_err)?;
    let dosage: Option<String> = row.try_get("", "dosage").map_err(map_db_err)?;
    let interactions: Option<String> = row.try_get("", "interactions").map_err(map_db_err)?;
    let contraindications: Option<String> =
        row.try_get("", "contraindications").map_err(map_db_err)?;
    let side_effects: Option<String> = row.try_get("", "side_effects").map_err(map_db_err)?;
    let pregnancy_and_lactation: Option<String> =
        row.try_get("", "pregnancy_and_lactation").map_err(map_db_err)?;
    let precautions: Option<String> = row.try_get("", "precautions").map_err(map_db_err)?;
    let storage: Option<String> = row.try_get("", "storage").map_err(map_db_err)?;
    let updated_at: DateTimeWithTimeZone = row.try_get("", "updated_at").map_err(map_db_err)?;

    Ok(Monograph {
        generic_id: GenericId::from_uuid(row_gid),
        indications,
        description,
        pharmacology,
        dosage,
        interactions,
        contraindications,
        side_effects,
        pregnancy_and_lactation,
        precautions,
        storage,
        updated_at: updated_at.into(),
    })
}

pub async fn resolve_issue(
    tx: &Tx,
    id: IssueReportId,
    by: UserId,
    note: String,
) -> Result<IssueReport, AppError> {
    let now = chrono::Utc::now();
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"
        UPDATE catalog_issue_reports
        SET status = 'resolved', admin_note = $2, resolved_by_user_id = $3, resolved_at = $4, updated_at = $4
        WHERE id = $1 AND status = 'open'
        RETURNING id, tenant_id, user_id, company_id, generic_id, product_id,
                  message, status, admin_note, resolved_by_user_id, resolved_at,
                  created_at, updated_at
        "#,
        [
            Value::from(id.as_uuid()),
            Value::from(note),
            Value::from(by.as_uuid()),
            Value::from(now.naive_utc()),
        ],
    );

    let row = tx.transaction()?.query_one_raw(stmt).await.map_err(map_db_err)?;
    let r = row.ok_or_else(|| {
        AppError::new(
            ErrorCode::Conflict("already_closed"),
            "This issue report is already closed.",
        )
    })?;

    let row_id: Uuid = r.try_get("", "id").map_err(map_db_err)?;
    let row_tid: Uuid = r.try_get("", "tenant_id").map_err(map_db_err)?;
    let row_uid: Uuid = r.try_get("", "user_id").map_err(map_db_err)?;
    let row_cid: Option<Uuid> = r.try_get("", "company_id").map_err(map_db_err)?;
    let row_gid: Option<Uuid> = r.try_get("", "generic_id").map_err(map_db_err)?;
    let row_pid: Option<Uuid> = r.try_get("", "product_id").map_err(map_db_err)?;
    let row_msg: String = r.try_get("", "message").map_err(map_db_err)?;
    let _row_status: String = r.try_get("", "status").map_err(map_db_err)?;
    let row_note: Option<String> = r.try_get("", "admin_note").map_err(map_db_err)?;
    let row_ruid: Option<Uuid> = r.try_get("", "resolved_by_user_id").map_err(map_db_err)?;
    let row_rat: Option<DateTimeWithTimeZone> =
        r.try_get("", "resolved_at").map_err(map_db_err)?;
    let row_created: DateTimeWithTimeZone = r.try_get("", "created_at").map_err(map_db_err)?;
    let row_updated: DateTimeWithTimeZone = r.try_get("", "updated_at").map_err(map_db_err)?;

    Ok(IssueReport {
        id: IssueReportId::from_uuid(row_id),
        tenant_id: TenantId::from_uuid(row_tid),
        user_id: UserId::from_uuid(row_uid),
        company_id: row_cid.map(CompanyId::from_uuid),
        generic_id: row_gid.map(GenericId::from_uuid),
        product_id: row_pid.map(ProductId::from_uuid),
        message: row_msg,
        status: IssueReportStatus::Resolved,
        admin_note: row_note,
        resolved_by_user_id: row_ruid.map(UserId::from_uuid),
        resolved_at: row_rat.map(Into::into),
        created_at: row_created.into(),
        updated_at: row_updated.into(),
    })
}

pub async fn dismiss_issue(
    tx: &Tx,
    id: IssueReportId,
    by: UserId,
    note: String,
) -> Result<IssueReport, AppError> {
    let now = chrono::Utc::now();
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"
        UPDATE catalog_issue_reports
        SET status = 'dismissed', admin_note = $2, resolved_by_user_id = $3, resolved_at = $4, updated_at = $4
        WHERE id = $1 AND status = 'open'
        RETURNING id, tenant_id, user_id, company_id, generic_id, product_id,
                  message, status, admin_note, resolved_by_user_id, resolved_at,
                  created_at, updated_at
        "#,
        [
            Value::from(id.as_uuid()),
            Value::from(note),
            Value::from(by.as_uuid()),
            Value::from(now.naive_utc()),
        ],
    );

    let row = tx.transaction()?.query_one_raw(stmt).await.map_err(map_db_err)?;
    let r = row.ok_or_else(|| {
        AppError::new(
            ErrorCode::Conflict("already_closed"),
            "This issue report is already closed.",
        )
    })?;

    let row_id: Uuid = r.try_get("", "id").map_err(map_db_err)?;
    let row_tid: Uuid = r.try_get("", "tenant_id").map_err(map_db_err)?;
    let row_uid: Uuid = r.try_get("", "user_id").map_err(map_db_err)?;
    let row_cid: Option<Uuid> = r.try_get("", "company_id").map_err(map_db_err)?;
    let row_gid: Option<Uuid> = r.try_get("", "generic_id").map_err(map_db_err)?;
    let row_pid: Option<Uuid> = r.try_get("", "product_id").map_err(map_db_err)?;
    let row_msg: String = r.try_get("", "message").map_err(map_db_err)?;
    let _row_status: String = r.try_get("", "status").map_err(map_db_err)?;
    let row_note: Option<String> = r.try_get("", "admin_note").map_err(map_db_err)?;
    let row_ruid: Option<Uuid> = r.try_get("", "resolved_by_user_id").map_err(map_db_err)?;
    let row_rat: Option<DateTimeWithTimeZone> =
        r.try_get("", "resolved_at").map_err(map_db_err)?;
    let row_created: DateTimeWithTimeZone = r.try_get("", "created_at").map_err(map_db_err)?;
    let row_updated: DateTimeWithTimeZone = r.try_get("", "updated_at").map_err(map_db_err)?;

    Ok(IssueReport {
        id: IssueReportId::from_uuid(row_id),
        tenant_id: TenantId::from_uuid(row_tid),
        user_id: UserId::from_uuid(row_uid),
        company_id: row_cid.map(CompanyId::from_uuid),
        generic_id: row_gid.map(GenericId::from_uuid),
        product_id: row_pid.map(ProductId::from_uuid),
        message: row_msg,
        status: IssueReportStatus::Dismissed,
        admin_note: row_note,
        resolved_by_user_id: row_ruid.map(UserId::from_uuid),
        resolved_at: row_rat.map(Into::into),
        created_at: row_created.into(),
        updated_at: row_updated.into(),
    })
}

pub async fn queue(
    db: &Db,
    kind: Option<RecordKind>,
    status: Option<RecordStatus>,
    page: Pagination,
) -> Result<ListPage<QueueItem>, AppError> {
    let limit = (page.limit.clamp(1, 200) + 1) as i64;
    let kind_str = kind.map(|k| k.as_str().to_string());
    let status_str = status
        .map(|s| s.as_str().to_string())
        .unwrap_or_else(|| "pending".to_string());
    let cursor_id = page.cursor.as_deref().and_then(|c| Uuid::parse_str(c).ok());

    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"
        SELECT * FROM (
            SELECT
                'company' AS kind,
                p.id,
                p.name AS display_name,
                p.status,
                p.submitted_by_tenant_id,
                p.submitted_by_user_id,
                p.created_at AS submitted_at,
                COALESCE(
                    (SELECT jsonb_agg(jsonb_build_object('id', s.id, 'name', s.name, 'similarity', s.similarity))
                     FROM (
                         SELECT c.id, c.name, similarity(c.name, p.name) AS similarity
                         FROM companies c
                         WHERE c.id <> p.id
                           AND c.status = 'approved'
                           AND c.name % p.name
                         ORDER BY similarity DESC
                         LIMIT 5
                     ) s
                    ),
                    '[]'::jsonb
                ) AS suggestions
            FROM companies p
            WHERE p.status = $1
            UNION ALL
            SELECT
                'generic' AS kind,
                p.id,
                p.name AS display_name,
                p.status,
                p.submitted_by_tenant_id,
                p.submitted_by_user_id,
                p.created_at AS submitted_at,
                COALESCE(
                    (SELECT jsonb_agg(jsonb_build_object('id', s.id, 'name', s.name, 'similarity', s.similarity))
                     FROM (
                         SELECT c.id, c.name, similarity(c.name, p.name) AS similarity
                         FROM generics c
                         WHERE c.id <> p.id
                           AND c.status = 'approved'
                           AND c.name % p.name
                         ORDER BY similarity DESC
                         LIMIT 5
                     ) s
                    ),
                    '[]'::jsonb
                ) AS suggestions
            FROM generics p
            WHERE p.status = $1
            UNION ALL
            SELECT
                'product' AS kind,
                p.id,
                p.brand_name AS display_name,
                p.status,
                p.submitted_by_tenant_id,
                p.submitted_by_user_id,
                p.created_at AS submitted_at,
                COALESCE(
                    (SELECT jsonb_agg(jsonb_build_object('id', s.id, 'name', s.name, 'similarity', s.similarity))
                     FROM (
                         SELECT c.id, c.brand_name AS name, similarity(c.brand_name, p.brand_name) AS similarity
                         FROM products c
                         WHERE c.id <> p.id
                           AND c.status = 'approved'
                           AND c.brand_name % p.brand_name
                         ORDER BY similarity DESC
                         LIMIT 5
                     ) s
                    ),
                    '[]'::jsonb
                ) AS suggestions
            FROM products p
            WHERE p.status = $1
        ) q
        WHERE ($2::text IS NULL OR q.kind = $2)
          AND ($3::uuid IS NULL OR q.id > $3)
        ORDER BY q.submitted_at ASC, q.id ASC
        LIMIT $4
        "#,
        [
            Value::from(status_str),
            Value::from(kind_str),
            Value::from(cursor_id),
            Value::from(limit),
        ],
    );

    let ctx = TenantContext::new(TenantId::from_uuid(Uuid::nil()), UserId::from_uuid(Uuid::nil()), vec![]);
    let tx = Tx::begin_platform(db.conn(), &ctx).await?;
    let rows = tx.transaction()?.query_all_raw(stmt).await.map_err(map_db_err)?;
    tx.commit().await?;

    let has_more = rows.len() as i64 == limit;
    let items: Result<Vec<QueueItem>, _> = rows
        .into_iter()
        .take(page.limit as usize)
        .map(|r| {
            let kind: String = r.try_get("", "kind")?;
            let id: Uuid = r.try_get("", "id")?;
            let display_name: String = r.try_get("", "display_name")?;
            let status: String = r.try_get("", "status")?;
            let submitted_by_tenant_id: Option<Uuid> =
                r.try_get("", "submitted_by_tenant_id")?;
            let submitted_by_user_id: Option<Uuid> = r.try_get("", "submitted_by_user_id")?;
            let submitted_at: DateTimeWithTimeZone = r.try_get("", "submitted_at")?;
            let suggestions_val: serde_json::Value = r.try_get("", "suggestions")?;
            let suggestions: Vec<SuggestedMatch> =
                serde_json::from_value(suggestions_val).unwrap_or_default();

            Ok(QueueItem {
                kind: RecordKind::from_str(&kind).unwrap_or(RecordKind::Product),
                id,
                display_name,
                status: RecordStatus::from_str(&status).unwrap_or(RecordStatus::Pending),
                submitted_by_tenant_id: submitted_by_tenant_id.map(TenantId::from_uuid),
                submitted_by_user_id: submitted_by_user_id.map(UserId::from_uuid),
                submitted_at: submitted_at.into(),
                suggestions,
            })
        })
        .collect();

    let items = items.map_err(map_db_err)?;
    let next_cursor = if has_more {
        items.last().map(|q| q.id.to_string())
    } else {
        None
    };

    Ok(ListPage::new(items, next_cursor))
}

pub async fn admin_issues(
    db: &Db,
    status: Option<IssueReportStatus>,
    page: Pagination,
) -> Result<ListPage<IssueReport>, AppError> {
    let limit = (page.limit.clamp(1, 200) + 1) as i64;
    let status_str = status.map(|s| s.as_str().to_string());
    let cursor_id = page.cursor.as_deref().and_then(|c| Uuid::parse_str(c).ok());

    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"
        SELECT
            id, tenant_id, user_id, company_id, generic_id, product_id,
            message, status, admin_note, resolved_by_user_id, resolved_at,
            created_at, updated_at
        FROM catalog_issue_reports
        WHERE ($1::text IS NULL OR status = $1)
          AND ($2::uuid IS NULL OR id > $2)
        ORDER BY created_at DESC, id ASC
        LIMIT $3
        "#,
        [
            Value::from(status_str),
            Value::from(cursor_id),
            Value::from(limit),
        ],
    );

    let ctx = TenantContext::new(TenantId::from_uuid(Uuid::nil()), UserId::from_uuid(Uuid::nil()), vec![]);
    let tx = Tx::begin_platform(db.conn(), &ctx).await?;
    let rows = tx.transaction()?.query_all_raw(stmt).await.map_err(map_db_err)?;
    tx.commit().await?;

    let has_more = rows.len() as i64 == limit;
    let items: Result<Vec<IssueReport>, _> = rows
        .into_iter()
        .take(page.limit as usize)
        .map(|r| {
            let id: Uuid = r.try_get("", "id")?;
            let tenant_id: Uuid = r.try_get("", "tenant_id")?;
            let user_id: Uuid = r.try_get("", "user_id")?;
            let company_id: Option<Uuid> = r.try_get("", "company_id")?;
            let generic_id: Option<Uuid> = r.try_get("", "generic_id")?;
            let product_id: Option<Uuid> = r.try_get("", "product_id")?;
            let message: String = r.try_get("", "message")?;
            let status: String = r.try_get("", "status")?;
            let admin_note: Option<String> = r.try_get("", "admin_note")?;
            let resolved_by_user_id: Option<Uuid> = r.try_get("", "resolved_by_user_id")?;
            let resolved_at: Option<DateTimeWithTimeZone> = r.try_get("", "resolved_at")?;
            let created_at: DateTimeWithTimeZone = r.try_get("", "created_at")?;
            let updated_at: DateTimeWithTimeZone = r.try_get("", "updated_at")?;

            Ok(IssueReport {
                id: IssueReportId::from_uuid(id),
                tenant_id: TenantId::from_uuid(tenant_id),
                user_id: UserId::from_uuid(user_id),
                company_id: company_id.map(CompanyId::from_uuid),
                generic_id: generic_id.map(GenericId::from_uuid),
                product_id: product_id.map(ProductId::from_uuid),
                message,
                status: IssueReportStatus::from_str(&status).unwrap_or(IssueReportStatus::Open),
                admin_note,
                resolved_by_user_id: resolved_by_user_id.map(UserId::from_uuid),
                resolved_at: resolved_at.map(Into::into),
                created_at: created_at.into(),
                updated_at: updated_at.into(),
            })
        })
        .collect();

    let items = items.map_err(map_db_err)?;
    let next_cursor = if has_more {
        items.last().map(|i| i.id.to_string())
    } else {
        None
    };

    Ok(ListPage::new(items, next_cursor))
}
