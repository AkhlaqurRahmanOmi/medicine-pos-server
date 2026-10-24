#![allow(clippy::unwrap_used, clippy::expect_used)]

use api::extract::Pagination;
use catalog::{commands, queries, types::*};
use db::{Db, Tx};
use pos_core::context::TenantContext;
use pos_core::error::ErrorCode;
use pos_core::ids::{
    CompanyId, DosageFormId, GenericId, PackUnitId, ProductId, TenantId, UserId,
};
use sea_orm::Database;
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

async fn app_pool() -> sqlx::PgPool {
    let url = std::env::var("APP_DATABASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| APP_URL.to_string())
        .replace("localhost", "127.0.0.1");
    PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(1000))
        .connect(&url)
        .await
        .expect("connect to app pool")
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

#[tokio::test]
async fn a_pending_row_is_invisible_to_another_pharmacy() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let dfid = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;

    let tenant_a = TenantId::new_v7();
    let tenant_b = TenantId::new_v7();
    let user_a = UserId::new_v7();
    let ctx_a = TenantContext::new(tenant_a, user_a, vec![]);
    let tx_a = Tx::begin(db.conn(), &ctx_a).await.expect("begin tx_a");

    let brand = format!("SecretPendingDrug {}", Uuid::now_v7());
    let prod = commands::submit_product(&tx_a, NewProduct {
        company_id: cid,
        generic_id: Some(gid),
        dosage_form_id: Some(dfid),
        brand_name: brand.clone(),
        strength: Some("500 mg".into()),
        product_type: ProductType::Allopathic,
        reference_unit_mrp: None,
        pack: None,
    }).await.expect("submit product");
    tx_a.commit().await.expect("commit tx_a");

    // 1. Invisible in product list for Tenant B
    let list_b = queries::products(
        &db,
        tenant_b,
        Some(cid),
        None,
        None,
        Pagination { cursor: None, limit: 100 },
    ).await.expect("query products");
    assert!(!list_b.data.iter().any(|p| p.id == prod.id), "Tenant B should not see Tenant A's pending product in list");

    // 2. Invisible in search for Tenant B
    let search_b = queries::search(
        &db,
        tenant_b,
        &brand,
        Pagination { cursor: None, limit: 100 },
    ).await.expect("search products");
    assert!(!search_b.data.iter().any(|p| p.id == prod.id), "Tenant B should not find Tenant A's pending product in search");

    // 3. Invisible in submissions for Tenant B
    let sub_b = queries::submissions(
        &db,
        tenant_b,
        Pagination { cursor: None, limit: 100 },
    ).await.expect("query submissions");
    assert!(!sub_b.data.iter().any(|s| s.id == prod.id.as_uuid()), "Tenant B should not see Tenant A's submission");
}

#[tokio::test]
async fn another_pharmacys_pending_row_is_404_not_403() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let dfid = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;

    let tenant_a = TenantId::new_v7();
    let tenant_b = TenantId::new_v7();
    let user_a = UserId::new_v7();
    let ctx_a = TenantContext::new(tenant_a, user_a, vec![]);
    let tx_a = Tx::begin(db.conn(), &ctx_a).await.expect("begin tx_a");

    let prod = commands::submit_product(&tx_a, NewProduct {
        company_id: cid,
        generic_id: Some(gid),
        dosage_form_id: Some(dfid),
        brand_name: format!("Pending404Drug {}", Uuid::now_v7()),
        strength: Some("500 mg".into()),
        product_type: ProductType::Allopathic,
        reference_unit_mrp: None,
        pack: None,
    }).await.expect("submit product");
    tx_a.commit().await.expect("commit tx_a");

    // Attempt direct read by Tenant B
    let res = queries::product(&db, tenant_b, prod.id).await;
    assert!(res.is_err(), "Reading another pharmacy's pending product must fail");
    let err = res.unwrap_err();
    assert_eq!(
        err.code,
        ErrorCode::NotFound,
        "Another pharmacy's pending row must return 404 NotFound, never 403 Forbidden"
    );
}

#[tokio::test]
async fn an_issue_report_is_invisible_to_another_pharmacy() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let dfid = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;

    let prod_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status) VALUES ($1, $2, $3, $4, $5, '500 mg', 'allopathic', 'approved')",
        prod_id, cid.as_uuid(), gid.as_uuid(), dfid.as_uuid(), format!("IssueDrug {}", Uuid::now_v7())
    ).execute(&pool).await.expect("seed approved product");

    let tenant_a = TenantId::new_v7();
    let tenant_b = TenantId::new_v7();
    let user_a = UserId::new_v7();
    let ctx_a = TenantContext::new(tenant_a, user_a, vec![]);
    let tx_a = Tx::begin(db.conn(), &ctx_a).await.expect("begin tx_a");

    let report_a = commands::report_issue(&tx_a, NewIssueReport {
        company_id: None,
        generic_id: None,
        product_id: Some(ProductId::from_uuid(prod_id)),
        message: "Confidential error report from A".into(),
    }).await.expect("report issue a");
    tx_a.commit().await.expect("commit tx_a");

    // Tenant B queries issue reports
    let reports_b = queries::issues(
        &db,
        tenant_b,
        Pagination { cursor: None, limit: 100 },
    ).await.expect("query issues for tenant b");

    assert!(
        !reports_b.data.iter().any(|r| r.id == report_a.id),
        "Tenant B must not see Tenant A's issue report"
    );
}

#[tokio::test]
async fn isolation_fails_when_the_product_read_policy_is_dropped() {
    let owner = owner_pool().await;
    let app = app_pool().await;

    let cid = seed_company(&owner, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&owner, &format!("Generic {}", Uuid::now_v7())).await;
    let dfid = seed_dosage_form(&owner, &format!("Dosage {}", Uuid::now_v7())).await;

    let tenant_a = TenantId::new_v7();
    let tenant_b = TenantId::new_v7();

    let prod_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status, submitted_by_tenant_id) VALUES ($1, $2, $3, $4, $5, '500 mg', 'allopathic', 'pending', $6)",
        prod_id, cid.as_uuid(), gid.as_uuid(), dfid.as_uuid(), format!("CanaryDrug {}", Uuid::now_v7()), tenant_a.as_uuid()
    ).execute(&owner).await.expect("insert pending product for tenant A");

    // 1. With policy intact: pos_app acting as Tenant B sees 0 rows of Tenant A's pending product
    let mut app_tx = app.begin().await.expect("begin app tx");
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_b.as_uuid().to_string())
        .execute(&mut *app_tx)
        .await
        .expect("set tenant B");

    let count_with_policy: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM products WHERE id = $1"
    )
    .bind(prod_id)
    .fetch_one(&mut *app_tx)
    .await
    .expect("count with policy");

    assert_eq!(count_with_policy.0, 0, "With policy, Tenant B cannot see Tenant A's pending product");
    app_tx.rollback().await.expect("rollback app tx");

    // 2. In an owner transaction: DROP POLICY products_select ON products;
    let mut owner_tx = owner.begin().await.expect("begin owner tx");
    sqlx::query("DROP POLICY products_select ON products")
        .execute(&mut *owner_tx)
        .await
        .expect("drop policy");

    // Without the policy, query returns 1 row
    let count_without_policy: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM products WHERE id = $1"
    )
    .bind(prod_id)
    .fetch_one(&mut *owner_tx)
    .await
    .expect("count without policy");

    assert_eq!(
        count_without_policy.0, 1,
        "When policy is dropped, cross-tenant row is visible (proves test tests RLS)"
    );

    // Rollback owner tx to restore policy
    owner_tx.rollback().await.expect("rollback owner tx");
}

#[tokio::test]
async fn app_is_platform_reads_every_pending_row() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let dfid = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;

    let tenant_a = TenantId::new_v7();
    let tenant_b = TenantId::new_v7();
    let user_a = UserId::new_v7();
    let user_b = UserId::new_v7();

    // Tenant A submits pending product
    let ctx_a = TenantContext::new(tenant_a, user_a, vec![]);
    let tx_a = Tx::begin(db.conn(), &ctx_a).await.expect("begin tx_a");
    let prod_a = commands::submit_product(&tx_a, NewProduct {
        company_id: cid,
        generic_id: Some(gid),
        dosage_form_id: Some(dfid),
        brand_name: format!("CrossPendingA {}", Uuid::now_v7()),
        strength: Some("500 mg".into()),
        product_type: ProductType::Allopathic,
        reference_unit_mrp: None,
        pack: None,
    }).await.expect("submit product a");
    tx_a.commit().await.expect("commit tx_a");

    // Tenant B submits pending product
    let ctx_b = TenantContext::new(tenant_b, user_b, vec![]);
    let tx_b = Tx::begin(db.conn(), &ctx_b).await.expect("begin tx_b");
    let prod_b = commands::submit_product(&tx_b, NewProduct {
        company_id: cid,
        generic_id: Some(gid),
        dosage_form_id: Some(dfid),
        brand_name: format!("CrossPendingB {}", Uuid::now_v7()),
        strength: Some("500 mg".into()),
        product_type: ProductType::Allopathic,
        reference_unit_mrp: None,
        pack: None,
    }).await.expect("submit product b");
    tx_b.commit().await.expect("commit tx_b");

    // Platform context reads queue (paginating until end)
    let mut all_items = Vec::new();
    let mut cursor = None;
    loop {
        let q = queries::queue(
            &db,
            Some(RecordKind::Product),
            Some(RecordStatus::Pending),
            Pagination { cursor, limit: 100 },
        ).await.expect("query queue");
        all_items.extend(q.data);
        if q.next_cursor.is_none() {
            break;
        }
        cursor = q.next_cursor;
    }

    let has_a = all_items.iter().any(|p| p.id == prod_a.id.as_uuid());
    let has_b = all_items.iter().any(|p| p.id == prod_b.id.as_uuid());

    assert!(has_a, "Platform must be able to read Tenant A's pending row");
    assert!(has_b, "Platform must be able to read Tenant B's pending row");
}
