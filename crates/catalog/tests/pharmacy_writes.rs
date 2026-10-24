#![allow(clippy::unwrap_used, clippy::expect_used)]

use catalog::{commands, queries, types::*};
use db::{Db, Tx};
use pos_core::context::TenantContext;
use pos_core::ids::{
    CompanyId, DosageFormId, GenericId, IngredientId, PackUnitId, ProductId, TenantId,
    TherapeuticClassId, UserId,
};
use sea_orm::{ConnectionTrait, Database};
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use uuid::Uuid;

const APP_URL: &str = "postgres://pos_app:devpass@127.0.0.1:5432/medicine_pos";
const OWNER_URL: &str = "postgres://pos_owner:devpass@127.0.0.1:5432/medicine_pos";

async fn get_db() -> Db {
    let url = std::env::var("APP_DATABASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| APP_URL.to_string())
        .replace("localhost", "127.0.0.1");
    let conn = Database::connect(&url).await.expect("connect to db");
    Db::from_connection(conn, url)
}

async fn owner_pool() -> sqlx::PgPool {
    let url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| OWNER_URL.to_string())
        .replace("localhost", "127.0.0.1");
    PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(1000))
        .connect(&url)
        .await
        .expect("connect to owner pool")
}

async fn seed_company(pool: &sqlx::PgPool, name: &str) -> CompanyId {
    let id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO companies (id, name, status) VALUES ($1, $2, 'approved')",
        id,
        name
    )
    .execute(pool)
    .await
    .expect("seed company");
    CompanyId::from_uuid(id)
}

async fn seed_generic(pool: &sqlx::PgPool, name: &str) -> GenericId {
    let id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO generics (id, name, status) VALUES ($1, $2, 'approved')",
        id,
        name
    )
    .execute(pool)
    .await
    .expect("seed generic");
    GenericId::from_uuid(id)
}

async fn seed_pack_unit(pool: &sqlx::PgPool, name: &str) -> PackUnitId {
    let id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO pack_units (id, name) VALUES ($1, $2) ON CONFLICT (name) DO UPDATE SET name = EXCLUDED.name RETURNING id",
        id,
        name
    )
    .fetch_one(pool)
    .await
    .map(|r| PackUnitId::from_uuid(r.id))
    .expect("seed pack unit")
}

async fn seed_dosage_form(pool: &sqlx::PgPool, name: &str) -> DosageFormId {
    let puid = seed_pack_unit(pool, &format!("PackUnit {}", &Uuid::now_v7().to_string()[..8])).await;
    let id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO dosage_forms (id, name, base_unit_id) VALUES ($1, $2, $3) ON CONFLICT (name) DO UPDATE SET name = EXCLUDED.name RETURNING id",
        id,
        name,
        puid.as_uuid()
    )
    .fetch_one(pool)
    .await
    .map(|r| DosageFormId::from_uuid(r.id))
    .expect("seed dosage form")
}

async fn seed_ingredient(pool: &sqlx::PgPool, name: &str) -> IngredientId {
    let id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO ingredients (id, name) VALUES ($1, $2) ON CONFLICT (name) DO UPDATE SET name = EXCLUDED.name RETURNING id",
        id,
        name
    )
    .fetch_one(pool)
    .await
    .map(|r| IngredientId::from_uuid(r.id))
    .expect("seed ingredient")
}

async fn seed_therapeutic_class(pool: &sqlx::PgPool, name: &str) -> TherapeuticClassId {
    let id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO therapeutic_classes (id, name) VALUES ($1, $2) ON CONFLICT (name) DO UPDATE SET name = EXCLUDED.name RETURNING id",
        id,
        name
    )
    .fetch_one(pool)
    .await
    .map(|r| TherapeuticClassId::from_uuid(r.id))
    .expect("seed therapeutic class")
}

#[tokio::test]
async fn a_submission_is_pending_and_owned_by_its_submitter() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let dfid = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;

    let tenant_a = TenantId::new_v7();
    let user_a = UserId::new_v7();
    let ctx_a = TenantContext::new(tenant_a, user_a, vec![]);
    let tx_a = Tx::begin(db.conn(), &ctx_a).await.expect("begin tx_a");

    let brand = format!("Brand {}", Uuid::now_v7());
    let new_prod = NewProduct {
        company_id: cid,
        generic_id: Some(gid),
        dosage_form_id: Some(dfid),
        brand_name: brand.clone(),
        strength: Some("500 mg".into()),
        product_type: ProductType::Allopathic,
        reference_unit_mrp: None,
        pack: None,
    };

    let prod = commands::submit_product(&tx_a, new_prod).await.expect("submit product");
    tx_a.commit().await.expect("commit tx_a");

    assert_eq!(prod.status, RecordStatus::Pending);

    // Direct DB check for submitter ownership
    let row = sqlx::query!(
        "SELECT status, submitted_by_tenant_id, submitted_by_user_id FROM products WHERE id = $1",
        prod.id.as_uuid()
    )
    .fetch_one(&pool)
    .await
    .expect("fetch product from db");

    assert_eq!(row.status, "pending");
    assert_eq!(row.submitted_by_tenant_id, Some(tenant_a.as_uuid()));
    assert_eq!(row.submitted_by_user_id, Some(user_a.as_uuid()));

    // Visible to Tenant A
    let read_a = queries::product(&db, tenant_a, prod.id).await;
    assert!(read_a.is_ok(), "Submitter must be able to read own pending product");

    // Invisible to Tenant B
    let tenant_b = TenantId::new_v7();
    let read_b = queries::product(&db, tenant_b, prod.id).await;
    assert!(read_b.is_err(), "Other tenant must NOT be able to read pending product");
    let err = read_b.unwrap_err();
    assert_eq!(err.code, pos_core::error::ErrorCode::NotFound);
}

#[tokio::test]
async fn a_pharmacy_cannot_insert_an_approved_row() {
    let db = get_db().await;

    let tenant = TenantId::new_v7();
    let user = UserId::new_v7();
    let ctx = TenantContext::new(tenant, user, vec![]);
    let tx = Tx::begin(db.conn(), &ctx).await.expect("begin tx");

    let stmt = sea_orm::Statement::from_sql_and_values(
        sea_orm::DbBackend::Postgres,
        "INSERT INTO companies (id, name, status, submitted_by_tenant_id, submitted_by_user_id) VALUES ($1, $2, 'approved', $3, $4)",
        [
            sea_orm::Value::from(Uuid::now_v7()),
            sea_orm::Value::from(format!("Bypassed Company {}", Uuid::now_v7())),
            sea_orm::Value::from(tenant.as_uuid()),
            sea_orm::Value::from(user.as_uuid()),
        ],
    );

    let res = tx.transaction().expect("tx").execute_raw(stmt).await;
    assert!(res.is_err(), "Pharmacy must not be able to insert an approved row");
    let err_str = res.unwrap_err().to_string();
    assert!(
        err_str.contains("violates row-level security policy") || err_str.contains("check constraint"),
        "Expected RLS policy violation, got: {err_str}"
    );
}

#[tokio::test]
async fn a_submission_cannot_be_filed_in_another_tenants_name() {
    let db = get_db().await;

    let tenant_a = TenantId::new_v7();
    let tenant_b = TenantId::new_v7();
    let user_a = UserId::new_v7();
    let ctx_a = TenantContext::new(tenant_a, user_a, vec![]);
    let tx = Tx::begin(db.conn(), &ctx_a).await.expect("begin tx");

    let stmt = sea_orm::Statement::from_sql_and_values(
        sea_orm::DbBackend::Postgres,
        "INSERT INTO companies (id, name, status, submitted_by_tenant_id, submitted_by_user_id) VALUES ($1, $2, 'pending', $3, $4)",
        [
            sea_orm::Value::from(Uuid::now_v7()),
            sea_orm::Value::from(format!("Fake Company {}", Uuid::now_v7())),
            sea_orm::Value::from(tenant_b.as_uuid()),
            sea_orm::Value::from(user_a.as_uuid()),
        ],
    );

    let res = tx.transaction().expect("tx").execute_raw(stmt).await;
    assert!(res.is_err(), "Pharmacy must not be able to insert on behalf of another tenant");
    let err_str = res.unwrap_err().to_string();
    assert!(
        err_str.contains("violates row-level security policy"),
        "Expected RLS policy violation, got: {err_str}"
    );
}

#[tokio::test]
async fn a_submission_cannot_reference_another_pharmacys_pending_company() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let dfid = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;

    // Tenant A submits a pending company
    let tenant_a = TenantId::new_v7();
    let user_a = UserId::new_v7();
    let ctx_a = TenantContext::new(tenant_a, user_a, vec![]);
    let tx_a = Tx::begin(db.conn(), &ctx_a).await.expect("begin tx_a");

    let comp_a = commands::submit_company(&tx_a, NewCompany {
        name: format!("Pending Company A {}", Uuid::now_v7()),
        medex_id: None,
    }).await.expect("submit company");
    tx_a.commit().await.expect("commit tx_a");

    // Tenant B attempts to submit a product referencing Tenant A's pending company
    let tenant_b = TenantId::new_v7();
    let user_b = UserId::new_v7();
    let ctx_b = TenantContext::new(tenant_b, user_b, vec![]);
    let tx_b = Tx::begin(db.conn(), &ctx_b).await.expect("begin tx_b");

    let new_prod = NewProduct {
        company_id: comp_a.id,
        generic_id: Some(gid),
        dosage_form_id: Some(dfid),
        brand_name: format!("Drug B {}", Uuid::now_v7()),
        strength: Some("500 mg".into()),
        product_type: ProductType::Allopathic,
        reference_unit_mrp: None,
        pack: None,
    };

    let res = commands::submit_product(&tx_b, new_prod).await;
    assert!(res.is_err(), "Submission cannot reference another pharmacy's pending company");
    let err = res.unwrap_err();
    assert_eq!(err.code, pos_core::error::ErrorCode::NotFound);
}

#[tokio::test]
async fn a_duplicate_submission_is_a_conflict_not_a_queue_entry() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let dfid = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;

    let brand = format!("UniqueDrug {}", Uuid::now_v7());

    let tenant = TenantId::new_v7();
    let user = UserId::new_v7();
    let ctx = TenantContext::new(tenant, user, vec![]);

    // First submission
    let tx1 = Tx::begin(db.conn(), &ctx).await.expect("begin tx1");
    commands::submit_product(&tx1, NewProduct {
        company_id: cid,
        generic_id: Some(gid),
        dosage_form_id: Some(dfid),
        brand_name: brand.clone(),
        strength: Some("500 mg".into()),
        product_type: ProductType::Allopathic,
        reference_unit_mrp: None,
        pack: None,
    }).await.expect("submit product 1");
    tx1.commit().await.expect("commit tx1");

    // Second submission with exact same fields
    let tx2 = Tx::begin(db.conn(), &ctx).await.expect("begin tx2");
    let res = commands::submit_product(&tx2, NewProduct {
        company_id: cid,
        generic_id: Some(gid),
        dosage_form_id: Some(dfid),
        brand_name: brand.clone(),
        strength: Some("500 mg".into()),
        product_type: ProductType::Allopathic,
        reference_unit_mrp: None,
        pack: None,
    }).await;

    assert!(res.is_err(), "Duplicate submission must be rejected with conflict");
    let err = res.unwrap_err();
    assert_eq!(err.code, pos_core::error::ErrorCode::Conflict("duplicate_product"));
}

#[tokio::test]
async fn a_medicine_submission_names_every_missing_field() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;

    let tenant = TenantId::new_v7();
    let user = UserId::new_v7();
    let ctx = TenantContext::new(tenant, user, vec![]);
    let tx = Tx::begin(db.conn(), &ctx).await.expect("begin tx");

    let new_prod = NewProduct {
        company_id: cid,
        generic_id: None,
        dosage_form_id: None,
        brand_name: "   ".into(),
        strength: None,
        product_type: ProductType::Allopathic,
        reference_unit_mrp: None,
        pack: None,
    };

    let res = commands::submit_product(&tx, new_prod).await;
    assert!(res.is_err(), "Medicine submission with missing fields must fail");
    let err = res.unwrap_err();
    assert_eq!(err.code, pos_core::error::ErrorCode::BadRequest);
    assert!(err.details.contains_key("generic_id"), "missing generic_id");
    assert!(err.details.contains_key("dosage_form_id"), "missing dosage_form_id");
    assert!(err.details.contains_key("strength"), "missing strength");
    assert!(err.details.contains_key("brand_name"), "missing brand_name");
}

#[tokio::test]
async fn an_optional_pack_is_stored_with_the_submission() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let dfid = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;
    let puid = seed_pack_unit(&pool, &format!("PackUnit {}", Uuid::now_v7())).await;

    let tenant = TenantId::new_v7();
    let user = UserId::new_v7();
    let ctx = TenantContext::new(tenant, user, vec![]);

    // 1. With pack
    let tx1 = Tx::begin(db.conn(), &ctx).await.expect("begin tx1");
    let prod_with_pack = commands::submit_product(&tx1, NewProduct {
        company_id: cid,
        generic_id: Some(gid),
        dosage_form_id: Some(dfid),
        brand_name: format!("DrugWithPack {}", Uuid::now_v7()),
        strength: Some("10 mg".into()),
        product_type: ProductType::Allopathic,
        reference_unit_mrp: None,
        pack: Some(NewPack {
            pack_unit_id: puid,
            base_units: 10,
        }),
    }).await.expect("submit product with pack");
    tx1.commit().await.expect("commit tx1");

    assert_eq!(prod_with_pack.packs.len(), 1);
    assert_eq!(prod_with_pack.packs[0].pack_unit_id, puid);
    assert_eq!(prod_with_pack.packs[0].base_units, 10);

    // 2. Without pack
    let tx2 = Tx::begin(db.conn(), &ctx).await.expect("begin tx2");
    let prod_without_pack = commands::submit_product(&tx2, NewProduct {
        company_id: cid,
        generic_id: Some(gid),
        dosage_form_id: Some(dfid),
        brand_name: format!("DrugNoPack {}", Uuid::now_v7()),
        strength: Some("10 mg".into()),
        product_type: ProductType::Allopathic,
        reference_unit_mrp: None,
        pack: None,
    }).await.expect("submit product without pack");
    tx2.commit().await.expect("commit tx2");

    assert!(prod_without_pack.packs.is_empty(), "Product submitted without pack should have empty packs");
}

#[tokio::test]
async fn a_pack_insert_is_refused_outside_an_own_pending_product() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let dfid = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;
    let puid = seed_pack_unit(&pool, &format!("PackUnit {}", Uuid::now_v7())).await;

    let tenant_a = TenantId::new_v7();
    let tenant_b = TenantId::new_v7();
    let user_a = UserId::new_v7();
    let user_b = UserId::new_v7();

    // 1. Approved product: cannot add pack as pharmacy
    let app_prod_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status) VALUES ($1, $2, $3, $4, $5, '500 mg', 'allopathic', 'approved')",
        app_prod_id, cid.as_uuid(), gid.as_uuid(), dfid.as_uuid(), format!("ApprovedDrug {}", Uuid::now_v7())
    ).execute(&pool).await.expect("seed approved product");

    let ctx_a = TenantContext::new(tenant_a, user_a, vec![]);
    let tx1 = Tx::begin(db.conn(), &ctx_a).await.expect("begin tx1");
    let stmt1 = sea_orm::Statement::from_sql_and_values(
        sea_orm::DbBackend::Postgres,
        "INSERT INTO product_packs (product_id, pack_unit_id, base_units) VALUES ($1, $2, 10)",
        [sea_orm::Value::from(app_prod_id), sea_orm::Value::from(puid.as_uuid())],
    );
    let res1 = tx1.transaction().expect("tx1").execute_raw(stmt1).await;
    assert!(res1.is_err(), "Cannot insert pack into approved product");

    // 2. Pending product owned by Tenant A: Tenant B cannot add pack
    let tx_submit = Tx::begin(db.conn(), &ctx_a).await.expect("begin tx_submit");
    let pending_prod = commands::submit_product(&tx_submit, NewProduct {
        company_id: cid,
        generic_id: Some(gid),
        dosage_form_id: Some(dfid),
        brand_name: format!("PendingA {}", Uuid::now_v7()),
        strength: Some("500 mg".into()),
        product_type: ProductType::Allopathic,
        reference_unit_mrp: None,
        pack: None,
    }).await.expect("submit pending product");
    tx_submit.commit().await.expect("commit tx_submit");

    let ctx_b = TenantContext::new(tenant_b, user_b, vec![]);
    let tx2 = Tx::begin(db.conn(), &ctx_b).await.expect("begin tx2");
    let stmt2 = sea_orm::Statement::from_sql_and_values(
        sea_orm::DbBackend::Postgres,
        "INSERT INTO product_packs (product_id, pack_unit_id, base_units) VALUES ($1, $2, 10)",
        [sea_orm::Value::from(pending_prod.id.as_uuid()), sea_orm::Value::from(puid.as_uuid())],
    );
    let res2 = tx2.transaction().expect("tx2").execute_raw(stmt2).await;
    assert!(res2.is_err(), "Tenant B cannot insert pack into Tenant A's pending product");
}

#[tokio::test]
async fn a_generic_is_submitted_with_its_ingredients_in_one_transaction() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let ing1 = seed_ingredient(&pool, &format!("Ing1 {}", Uuid::now_v7())).await;
    let ing2 = seed_ingredient(&pool, &format!("Ing2 {}", Uuid::now_v7())).await;
    let tc1 = seed_therapeutic_class(&pool, &format!("Class1 {}", Uuid::now_v7())).await;
    let tc2 = seed_therapeutic_class(&pool, &format!("Class2 {}", Uuid::now_v7())).await;

    let tenant = TenantId::new_v7();
    let user = UserId::new_v7();
    let ctx = TenantContext::new(tenant, user, vec![]);

    // 1. Successful submission with ingredients and classes
    let tx1 = Tx::begin(db.conn(), &ctx).await.expect("begin tx1");
    let gen_name = format!("Paracetamol + Caffeine {}", Uuid::now_v7());
    let gen = commands::submit_generic(&tx1, NewGeneric {
        name: gen_name.clone(),
        medex_id: None,
        ingredient_ids: vec![ing1, ing2],
        therapeutic_class_ids: vec![tc1, tc2],
    }).await.expect("submit generic");
    tx1.commit().await.expect("commit tx1");

    assert_eq!(gen.ingredients.len(), 2);
    assert_eq!(gen.classes.len(), 2);

    // Read back
    let read_back = queries::generic(&db, tenant, gen.id).await.expect("read generic back");
    assert_eq!(read_back.ingredients.len(), 2);
    assert_eq!(read_back.classes.len(), 2);

    // 2. Failed atomic submission (invalid ingredient)
    let bad_ing = IngredientId::new_v7();
    let tx2 = Tx::begin(db.conn(), &ctx).await.expect("begin tx2");
    let bad_name = format!("FailedGeneric {}", Uuid::now_v7());
    let res = commands::submit_generic(&tx2, NewGeneric {
        name: bad_name.clone(),
        medex_id: None,
        ingredient_ids: vec![ing1, bad_ing],
        therapeutic_class_ids: vec![tc1],
    }).await;
    assert!(res.is_err(), "Generic submission with missing ingredient must fail");

    // Verify bad generic was never written
    let rows = sqlx::query!("SELECT id FROM generics WHERE name = $1", bad_name)
        .fetch_all(&pool)
        .await
        .expect("check generic row");
    assert!(rows.is_empty(), "Generic should not exist in database");
}

#[tokio::test]
async fn an_issue_report_is_filed_open() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let dfid = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;

    let prod_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status) VALUES ($1, $2, $3, $4, $5, '500 mg', 'allopathic', 'approved')",
        prod_id, cid.as_uuid(), gid.as_uuid(), dfid.as_uuid(), format!("ReportTargetDrug {}", Uuid::now_v7())
    ).execute(&pool).await.expect("seed approved product");

    let tenant = TenantId::new_v7();
    let user = UserId::new_v7();
    let ctx = TenantContext::new(tenant, user, vec![]);
    let tx = Tx::begin(db.conn(), &ctx).await.expect("begin tx");

    let rep = commands::report_issue(&tx, NewIssueReport {
        company_id: None,
        generic_id: None,
        product_id: Some(ProductId::from_uuid(prod_id)),
        message: "Wrong strength listed".into(),
    }).await.expect("report issue");
    tx.commit().await.expect("commit tx");

    assert_eq!(rep.status, IssueReportStatus::Open);
    assert_eq!(rep.product_id, Some(ProductId::from_uuid(prod_id)));
    assert_eq!(rep.message, "Wrong strength listed");
    assert!(rep.company_id.is_none());
    assert!(rep.generic_id.is_none());
}

#[tokio::test]
async fn an_issue_report_with_two_targets_is_refused() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let dfid = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;

    let prod_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status) VALUES ($1, $2, $3, $4, $5, '500 mg', 'allopathic', 'approved')",
        prod_id, cid.as_uuid(), gid.as_uuid(), dfid.as_uuid(), format!("ReportTargetDrug {}", Uuid::now_v7())
    ).execute(&pool).await.expect("seed approved product");

    let tenant = TenantId::new_v7();
    let user = UserId::new_v7();
    let ctx = TenantContext::new(tenant, user, vec![]);
    let tx = Tx::begin(db.conn(), &ctx).await.expect("begin tx");

    let res = commands::report_issue(&tx, NewIssueReport {
        company_id: Some(cid),
        generic_id: None,
        product_id: Some(ProductId::from_uuid(prod_id)),
        message: "Two targets".into(),
    }).await;

    assert!(res.is_err(), "Issue report naming two targets must be refused");
    let err = res.unwrap_err();
    assert_eq!(err.code, pos_core::error::ErrorCode::BadRequest);
    assert_eq!(
        err.user_message,
        "Choose the one product, generic or manufacturer this report is about."
    );
}
