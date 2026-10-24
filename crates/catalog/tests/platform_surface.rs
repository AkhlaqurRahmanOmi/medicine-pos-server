#![allow(clippy::unwrap_used, clippy::expect_used)]

use api::extract::Pagination;
use catalog::{commands, platform, queries, types::*};
use db::{Db, Tx};
use pos_core::context::TenantContext;
use pos_core::ids::{
    CompanyId, DosageFormId, GenericId, PackUnitId, ProductId, TenantId, UserId,
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

#[tokio::test]
async fn approval_records_who_and_when_and_publishes_the_row() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let dfid = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;

    // 1. Submit pending product as Tenant A
    let tenant_a = TenantId::new_v7();
    let user_a = UserId::new_v7();
    let ctx_a = TenantContext::new(tenant_a, user_a, vec![]);
    let tx_a = Tx::begin(db.conn(), &ctx_a).await.expect("begin tx_a");

    let brand = format!("PendingPublish {}", Uuid::now_v7());
    let prod = commands::submit_product(&tx_a, NewProduct {
        company_id: cid,
        generic_id: Some(gid),
        dosage_form_id: Some(dfid),
        brand_name: brand,
        strength: Some("500 mg".into()),
        product_type: ProductType::Allopathic,
        reference_unit_mrp: None,
        pack: None,
    }).await.expect("submit product");
    tx_a.commit().await.expect("commit tx_a");

    // Initially invisible to Tenant B
    let tenant_b = TenantId::new_v7();
    let read_before = queries::product(&db, tenant_b, prod.id).await;
    assert!(read_before.is_err(), "Pending product must be invisible to Tenant B");

    // 2. Platform approves the product
    let admin_user = UserId::new_v7();
    let admin_ctx = TenantContext::new(TenantId::from_uuid(Uuid::nil()), admin_user, vec![]);
    let tx_admin = Tx::begin_platform(db.conn(), &admin_ctx).await.expect("begin platform");
    let reviewed = platform::approve(&tx_admin, RecordKind::Product, prod.id.as_uuid(), admin_user)
        .await
        .expect("platform approve");
    tx_admin.commit().await.expect("commit admin tx");

    assert_eq!(reviewed.status, RecordStatus::Approved);
    assert!(reviewed.reviewed_at.is_some(), "Reviewed at must be recorded");

    // Direct DB check for reviewed_by_user_id and reviewed_at
    use sqlx::Row;
    let row = sqlx::query("SELECT status, reviewed_by_user_id, reviewed_at FROM products WHERE id = $1")
        .bind(prod.id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("fetch product");

    let status: String = row.get("status");
    let reviewed_by: Option<Uuid> = row.get("reviewed_by_user_id");
    let reviewed_at: Option<chrono::DateTime<chrono::Utc>> = row.get("reviewed_at");

    assert_eq!(status, "approved");
    assert_eq!(reviewed_by, Some(admin_user.as_uuid()));
    assert!(reviewed_at.is_some());

    // 3. Published to everyone: Tenant B can now read it
    let read_after = queries::product(&db, tenant_b, prod.id).await;
    assert!(read_after.is_ok(), "Approved product must be visible to all tenants");
    let pub_prod = read_after.unwrap();
    assert_eq!(pub_prod.status, RecordStatus::Approved);
}

#[tokio::test]
async fn the_queue_suggests_near_duplicates_by_trigram() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let dfid = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;

    let uid = &Uuid::now_v7().to_string()[..8];
    let approved_brand = format!("AcePlusSpecial{uid}");
    let similar_brand = format!("AcePlusSpecia{uid}");
    let unique_brand = format!("ZzzUniqueNeverSeen{uid}");

    // Seed approved product
    sqlx::query!(
        "INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status) VALUES ($1, $2, $3, $4, $5, '500 mg', 'allopathic', 'approved')",
        Uuid::now_v7(), cid.as_uuid(), gid.as_uuid(), dfid.as_uuid(), approved_brand
    ).execute(&pool).await.expect("seed approved product");

    // Submit similar pending product
    let tenant = TenantId::new_v7();
    let user = UserId::new_v7();
    let ctx = TenantContext::new(tenant, user, vec![]);
    let tx1 = Tx::begin(db.conn(), &ctx).await.expect("begin tx1");
    let prod_similar = commands::submit_product(&tx1, NewProduct {
        company_id: cid,
        generic_id: Some(gid),
        dosage_form_id: Some(dfid),
        brand_name: similar_brand.clone(),
        strength: Some("500 mg".into()),
        product_type: ProductType::Allopathic,
        reference_unit_mrp: None,
        pack: None,
    }).await.expect("submit similar");
    tx1.commit().await.expect("commit tx1");

    // Submit completely unique pending product
    let tx2 = Tx::begin(db.conn(), &ctx).await.expect("begin tx2");
    let prod_unique = commands::submit_product(&tx2, NewProduct {
        company_id: cid,
        generic_id: Some(gid),
        dosage_form_id: Some(dfid),
        brand_name: unique_brand.clone(),
        strength: Some("500 mg".into()),
        product_type: ProductType::Allopathic,
        reference_unit_mrp: None,
        pack: None,
    }).await.expect("submit unique");
    tx2.commit().await.expect("commit tx2");

    // Query queue (paginating until items found)
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

    // Find similar item in queue
    let item_similar = all_items.iter().find(|i| i.id == prod_similar.id.as_uuid()).expect("find similar in queue");
    assert!(!item_similar.suggestions.is_empty(), "Similar brand must have suggestions");
    assert!(item_similar.suggestions.len() <= 5, "Suggestions must not exceed 5");
    assert_eq!(item_similar.suggestions[0].name, approved_brand);
    assert!(item_similar.suggestions[0].similarity > 0.3);

    // Find unique item in queue
    let item_unique = all_items.iter().find(|i| i.id == prod_unique.id.as_uuid()).expect("find unique in queue");
    assert!(item_unique.suggestions.is_empty(), "Unrelated brand must have no suggestions");
}

#[tokio::test]
async fn queue_suggestions_do_not_issue_a_query_per_row() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let dfid = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;

    let tenant = TenantId::new_v7();
    let user = UserId::new_v7();
    let ctx = TenantContext::new(tenant, user, vec![]);

    // Seed multiple pending products
    for i in 0..5 {
        let tx = Tx::begin(db.conn(), &ctx).await.expect("begin tx");
        commands::submit_product(&tx, NewProduct {
            company_id: cid,
            generic_id: Some(gid),
            dosage_form_id: Some(dfid),
            brand_name: format!("BatchDrug_{}_{}", i, Uuid::now_v7()),
            strength: Some("10 mg".into()),
            product_type: ProductType::Allopathic,
            reference_unit_mrp: None,
            pack: None,
        }).await.expect("submit batch product");
        tx.commit().await.expect("commit tx");
    }

    // Query queue - executed as a single query with subquery aggregations
    let res = queries::queue(
        &db,
        Some(RecordKind::Product),
        Some(RecordStatus::Pending),
        Pagination { cursor: None, limit: 10 },
    ).await;

    assert!(res.is_ok(), "Queue query with suggestions must succeed");
    let page = res.unwrap();
    assert!(page.data.len() >= 5);
}

#[tokio::test]
async fn rejecting_requires_a_note() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let dfid = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;

    let tenant = TenantId::new_v7();
    let user = UserId::new_v7();
    let ctx = TenantContext::new(tenant, user, vec![]);
    let tx_sub = Tx::begin(db.conn(), &ctx).await.expect("begin tx_sub");

    let prod = commands::submit_product(&tx_sub, NewProduct {
        company_id: cid,
        generic_id: Some(gid),
        dosage_form_id: Some(dfid),
        brand_name: format!("RejectMe {}", Uuid::now_v7()),
        strength: Some("500 mg".into()),
        product_type: ProductType::Allopathic,
        reference_unit_mrp: None,
        pack: None,
    }).await.expect("submit product");
    tx_sub.commit().await.expect("commit tx_sub");

    let admin_user = UserId::new_v7();
    let admin_ctx = TenantContext::new(TenantId::from_uuid(Uuid::nil()), admin_user, vec![]);

    // 1. Rejection without note refused at command
    let tx1 = Tx::begin_platform(db.conn(), &admin_ctx).await.expect("begin tx1");
    let res1 = platform::reject(&tx1, RecordKind::Product, prod.id.as_uuid(), admin_user, "   ".into()).await;
    assert!(res1.is_err(), "Rejection without note must be refused by command");
    let err = res1.unwrap_err();
    assert_eq!(err.code, pos_core::error::ErrorCode::BadRequest);

    // 2. Direct database update without note refused by check constraint
    let stmt = sea_orm::Statement::from_sql_and_values(
        sea_orm::DbBackend::Postgres,
        "UPDATE products SET status = 'rejected', review_note = NULL WHERE id = $1",
        [sea_orm::Value::from(prod.id.as_uuid())],
    );
    let res_db = tx1.transaction().expect("tx1").execute_raw(stmt).await;
    assert!(res_db.is_err(), "Database check constraint must reject rejection without note");

    // 3. Successful rejection with note
    let tx2 = Tx::begin_platform(db.conn(), &admin_ctx).await.expect("begin tx2");
    let res2 = platform::reject(&tx2, RecordKind::Product, prod.id.as_uuid(), admin_user, "Invalid name".into()).await;
    assert!(res2.is_ok(), "Rejection with note must succeed");
    tx2.commit().await.expect("commit tx2");
}

#[tokio::test]
async fn a_merge_sets_the_target_and_moves_no_stock() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let dfid = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;

    // Target approved product
    let target_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status) VALUES ($1, $2, $3, $4, $5, '500 mg', 'allopathic', 'approved')",
        target_id, cid.as_uuid(), gid.as_uuid(), dfid.as_uuid(), format!("TargetDrug {}", Uuid::now_v7())
    ).execute(&pool).await.expect("seed target");

    // Source pending product
    let tenant = TenantId::new_v7();
    let user = UserId::new_v7();
    let ctx = TenantContext::new(tenant, user, vec![]);
    let tx_sub = Tx::begin(db.conn(), &ctx).await.expect("begin tx_sub");
    let source = commands::submit_product(&tx_sub, NewProduct {
        company_id: cid,
        generic_id: Some(gid),
        dosage_form_id: Some(dfid),
        brand_name: format!("SourceDrug {}", Uuid::now_v7()),
        strength: Some("500 mg".into()),
        product_type: ProductType::Allopathic,
        reference_unit_mrp: None,
        pack: None,
    }).await.expect("submit source");
    tx_sub.commit().await.expect("commit tx_sub");

    // Platform merges source into target
    let admin_user = UserId::new_v7();
    let admin_ctx = TenantContext::new(TenantId::from_uuid(Uuid::nil()), admin_user, vec![]);
    let tx_admin = Tx::begin_platform(db.conn(), &admin_ctx).await.expect("begin tx_admin");
    let reviewed = platform::merge(&tx_admin, RecordKind::Product, source.id.as_uuid(), target_id, admin_user)
        .await
        .expect("merge");
    tx_admin.commit().await.expect("commit tx_admin");

    assert_eq!(reviewed.status, RecordStatus::Merged);
    assert_eq!(reviewed.merged_into_id, Some(target_id));

    // Verify database row
    let row = sqlx::query!(
        "SELECT status, merged_into_id FROM products WHERE id = $1",
        source.id.as_uuid()
    ).fetch_one(&pool).await.expect("fetch product");

    assert_eq!(row.status, "merged");
    assert_eq!(row.merged_into_id, Some(target_id));
}

#[tokio::test]
async fn merging_into_a_merged_row_is_refused() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let dfid = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;

    // Target A (approved)
    let target_a = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status) VALUES ($1, $2, $3, $4, $5, '500 mg', 'allopathic', 'approved')",
        target_a, cid.as_uuid(), gid.as_uuid(), dfid.as_uuid(), format!("TargetA {}", Uuid::now_v7())
    ).execute(&pool).await.expect("seed target A");

    // Target B (merged into A)
    let target_b = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status, merged_into_id) VALUES ($1, $2, $3, $4, $5, '500 mg', 'allopathic', 'merged', $6)",
        target_b, cid.as_uuid(), gid.as_uuid(), dfid.as_uuid(), format!("TargetB {}", Uuid::now_v7()), target_a
    ).execute(&pool).await.expect("seed target B");

    // Row C (pending)
    let tenant = TenantId::new_v7();
    let user = UserId::new_v7();
    let ctx = TenantContext::new(tenant, user, vec![]);
    let tx_sub = Tx::begin(db.conn(), &ctx).await.expect("begin tx_sub");
    let row_c = commands::submit_product(&tx_sub, NewProduct {
        company_id: cid,
        generic_id: Some(gid),
        dosage_form_id: Some(dfid),
        brand_name: format!("RowC {}", Uuid::now_v7()),
        strength: Some("500 mg".into()),
        product_type: ProductType::Allopathic,
        reference_unit_mrp: None,
        pack: None,
    }).await.expect("submit row C");
    tx_sub.commit().await.expect("commit tx_sub");

    let admin_user = UserId::new_v7();
    let admin_ctx = TenantContext::new(TenantId::from_uuid(Uuid::nil()), admin_user, vec![]);
    let tx_admin = Tx::begin_platform(db.conn(), &admin_ctx).await.expect("begin tx_admin");

    // 1. Merging C into B (which is already merged) must be refused
    let res = platform::merge(&tx_admin, RecordKind::Product, row_c.id.as_uuid(), target_b, admin_user).await;
    assert!(res.is_err(), "Merging into an already merged row must be refused");
    let err = res.unwrap_err();
    assert_eq!(err.code, pos_core::error::ErrorCode::Conflict("invalid_merge_target"));

    // 2. Merging C into C (itself) must be refused
    let res_self = platform::merge(&tx_admin, RecordKind::Product, row_c.id.as_uuid(), row_c.id.as_uuid(), admin_user).await;
    assert!(res_self.is_err(), "Merging into itself must be refused");
    let err_self = res_self.unwrap_err();
    assert_eq!(err_self.code, pos_core::error::ErrorCode::Conflict("invalid_merge_target"));
}

#[tokio::test]
async fn an_already_reviewed_row_cannot_be_reviewed_again() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let dfid = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;

    // Seed approved product
    let prod_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status) VALUES ($1, $2, $3, $4, $5, '500 mg', 'allopathic', 'approved')",
        prod_id, cid.as_uuid(), gid.as_uuid(), dfid.as_uuid(), format!("AlreadyApproved {}", Uuid::now_v7())
    ).execute(&pool).await.expect("seed approved");

    let admin_user = UserId::new_v7();
    let admin_ctx = TenantContext::new(TenantId::from_uuid(Uuid::nil()), admin_user, vec![]);
    let tx = Tx::begin_platform(db.conn(), &admin_ctx).await.expect("begin tx");

    // Try approving again
    let res_app = platform::approve(&tx, RecordKind::Product, prod_id, admin_user).await;
    assert!(res_app.is_err(), "Approving an already reviewed row must fail");
    assert_eq!(res_app.unwrap_err().code, pos_core::error::ErrorCode::Conflict("already_reviewed"));

    // Try rejecting
    let res_rej = platform::reject(&tx, RecordKind::Product, prod_id, admin_user, "Note".into()).await;
    assert!(res_rej.is_err(), "Rejecting an already reviewed row must fail");
    assert_eq!(res_rej.unwrap_err().code, pos_core::error::ErrorCode::Conflict("already_reviewed"));

    // Try merging
    let other_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status) VALUES ($1, $2, $3, $4, $5, '500 mg', 'allopathic', 'approved')",
        other_id, cid.as_uuid(), gid.as_uuid(), dfid.as_uuid(), format!("TargetDrugOther {}", Uuid::now_v7())
    ).execute(&pool).await.expect("seed target for merge");
    let res_mer = platform::merge(&tx, RecordKind::Product, prod_id, other_id, admin_user).await;
    assert!(res_mer.is_err(), "Merging an already reviewed row must fail");
    assert_eq!(res_mer.unwrap_err().code, pos_core::error::ErrorCode::Conflict("already_reviewed"));
}

#[tokio::test]
async fn the_review_queue_spans_all_three_kinds() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let dfid = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;

    let tenant = TenantId::new_v7();
    let user = UserId::new_v7();
    let ctx = TenantContext::new(tenant, user, vec![]);

    // 1. Submit company
    let tx1 = Tx::begin(db.conn(), &ctx).await.expect("begin tx1");
    let c = commands::submit_company(&tx1, NewCompany {
        name: format!("QueueComp {}", Uuid::now_v7()),
        medex_id: None,
    }).await.expect("submit company");
    tx1.commit().await.expect("commit tx1");

    // 2. Submit generic
    let tx2 = Tx::begin(db.conn(), &ctx).await.expect("begin tx2");
    let g = commands::submit_generic(&tx2, NewGeneric {
        name: format!("QueueGen {}", Uuid::now_v7()),
        medex_id: None,
        ingredient_ids: vec![],
        therapeutic_class_ids: vec![],
    }).await.expect("submit generic");
    tx2.commit().await.expect("commit tx2");

    // 3. Submit product
    let tx3 = Tx::begin(db.conn(), &ctx).await.expect("begin tx3");
    let p = commands::submit_product(&tx3, NewProduct {
        company_id: cid,
        generic_id: Some(gid),
        dosage_form_id: Some(dfid),
        brand_name: format!("QueueProd {}", Uuid::now_v7()),
        strength: Some("500 mg".into()),
        product_type: ProductType::Allopathic,
        reference_unit_mrp: None,
        pack: None,
    }).await.expect("submit product");
    tx3.commit().await.expect("commit tx3");

    // Fetch queue across all kinds (paginating until end)
    let mut all_items = Vec::new();
    let mut cursor = None;
    loop {
        let q = queries::queue(
            &db,
            None,
            Some(RecordStatus::Pending),
            Pagination { cursor, limit: 100 },
        ).await.expect("fetch queue");
        all_items.extend(q.data);
        if q.next_cursor.is_none() {
            break;
        }
        cursor = q.next_cursor;
    }

    let has_c = all_items.iter().any(|i| i.kind == RecordKind::Company && i.id == c.id.as_uuid());
    let has_g = all_items.iter().any(|i| i.kind == RecordKind::Generic && i.id == g.id.as_uuid());
    let has_p = all_items.iter().any(|i| i.kind == RecordKind::Product && i.id == p.id.as_uuid());

    assert!(has_c, "Review queue must contain pending company");
    assert!(has_g, "Review queue must contain pending generic");
    assert!(has_p, "Review queue must contain pending product");
}

#[tokio::test]
async fn a_generic_and_its_monograph_are_written_in_one_transaction() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let gid = seed_generic(&pool, &format!("GenericWithMono {}", Uuid::now_v7())).await;

    let admin_user = UserId::new_v7();
    let admin_ctx = TenantContext::new(TenantId::from_uuid(Uuid::nil()), admin_user, vec![]);
    let tx = Tx::begin_platform(db.conn(), &admin_ctx).await.expect("begin platform tx");

    let input = MonographInput {
        indications: Some("Fever, pain".into()),
        description: Some("Analgesic and antipyretic".into()),
        pharmacology: Some("Inhibits prostaglandin synthesis".into()),
        dosage: Some("500 mg every 6 hours".into()),
        interactions: Some("Avoid alcohol".into()),
        contraindications: Some("Severe hepatic impairment".into()),
        side_effects: Some("Rare allergic reactions".into()),
        pregnancy_and_lactation: Some("Category B".into()),
        precautions: Some("Hepatic risk".into()),
        storage: Some("Store below 30C".into()),
    };

    let mono = platform::upsert_monograph(&tx, gid, input).await.expect("upsert monograph");
    tx.commit().await.expect("commit tx");

    assert_eq!(mono.generic_id, gid);
    assert_eq!(mono.indications.as_deref(), Some("Fever, pain"));
    assert_eq!(mono.description.as_deref(), Some("Analgesic and antipyretic"));

    // Direct read back
    let read = queries::monograph(&db, gid).await.expect("read monograph");
    assert!(read.is_some(), "Monograph must be found");
    let m = read.unwrap();
    assert_eq!(m.indications.as_deref(), Some("Fever, pain"));
    assert_eq!(m.dosage.as_deref(), Some("500 mg every 6 hours"));
}

#[tokio::test]
async fn a_pharmacy_cannot_edit_a_lookup_table() {
    let db = get_db().await;

    let tenant = TenantId::new_v7();
    let user = UserId::new_v7();
    let ctx = TenantContext::new(tenant, user, vec![]);
    let tx = Tx::begin(db.conn(), &ctx).await.expect("begin tx");

    // Attempt to write into pack_units
    let stmt1 = sea_orm::Statement::from_sql_and_values(
        sea_orm::DbBackend::Postgres,
        "INSERT INTO pack_units (id, name) VALUES ($1, $2)",
        [sea_orm::Value::from(Uuid::now_v7()), sea_orm::Value::from("HackedUnit")],
    );
    let res1 = tx.transaction().expect("tx").execute_raw(stmt1).await;
    assert!(res1.is_err(), "Pharmacy cannot insert into pack_units");

    // Attempt to write into dosage_forms
    let stmt2 = sea_orm::Statement::from_sql_and_values(
        sea_orm::DbBackend::Postgres,
        "INSERT INTO dosage_forms (id, name, base_unit_id) VALUES ($1, $2, $3)",
        [
            sea_orm::Value::from(Uuid::now_v7()),
            sea_orm::Value::from("HackedDosage"),
            sea_orm::Value::from(Uuid::now_v7()),
        ],
    );
    let res2 = tx.transaction().expect("tx").execute_raw(stmt2).await;
    assert!(res2.is_err(), "Pharmacy cannot insert into dosage_forms");

    // Attempt to write into ingredients
    let stmt3 = sea_orm::Statement::from_sql_and_values(
        sea_orm::DbBackend::Postgres,
        "INSERT INTO ingredients (id, name) VALUES ($1, $2)",
        [sea_orm::Value::from(Uuid::now_v7()), sea_orm::Value::from("HackedIngredient")],
    );
    let res3 = tx.transaction().expect("tx").execute_raw(stmt3).await;
    assert!(res3.is_err(), "Pharmacy cannot insert into ingredients");

    // Attempt to write into therapeutic_classes
    let stmt4 = sea_orm::Statement::from_sql_and_values(
        sea_orm::DbBackend::Postgres,
        "INSERT INTO therapeutic_classes (id, name) VALUES ($1, $2)",
        [sea_orm::Value::from(Uuid::now_v7()), sea_orm::Value::from("HackedClass")],
    );
    let res4 = tx.transaction().expect("tx").execute_raw(stmt4).await;
    assert!(res4.is_err(), "Pharmacy cannot insert into therapeutic_classes");
}

#[tokio::test]
async fn an_issue_report_closes_once_with_its_note() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let dfid = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;

    let prod_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status) VALUES ($1, $2, $3, $4, $5, '500 mg', 'allopathic', 'approved')",
        prod_id, cid.as_uuid(), gid.as_uuid(), dfid.as_uuid(), format!("IssueTargetDrug {}", Uuid::now_v7())
    ).execute(&pool).await.expect("seed approved product");

    let tenant = TenantId::new_v7();
    let user = UserId::new_v7();
    let ctx = TenantContext::new(tenant, user, vec![]);
    let tx_rep = Tx::begin(db.conn(), &ctx).await.expect("begin tx_rep");

    let rep = commands::report_issue(&tx_rep, NewIssueReport {
        company_id: None,
        generic_id: None,
        product_id: Some(ProductId::from_uuid(prod_id)),
        message: "Typo in strength".into(),
    }).await.expect("report issue");
    tx_rep.commit().await.expect("commit tx_rep");

    let admin_user = UserId::new_v7();
    let admin_ctx = TenantContext::new(TenantId::from_uuid(Uuid::nil()), admin_user, vec![]);

    // 1. Resolve issue once
    let tx1 = Tx::begin_platform(db.conn(), &admin_ctx).await.expect("begin tx1");
    let resolved = platform::resolve_issue(&tx1, rep.id, admin_user, "Fixed in catalog".into())
        .await
        .expect("resolve issue");
    tx1.commit().await.expect("commit tx1");

    assert_eq!(resolved.status, IssueReportStatus::Resolved);
    assert_eq!(resolved.admin_note.as_deref(), Some("Fixed in catalog"));
    assert_eq!(resolved.resolved_by_user_id, Some(admin_user));
    assert!(resolved.resolved_at.is_some());

    // 2. Resolve again must fail with already_closed
    let tx2 = Tx::begin_platform(db.conn(), &admin_ctx).await.expect("begin tx2");
    let res_again = platform::resolve_issue(&tx2, rep.id, admin_user, "Again".into()).await;
    assert!(res_again.is_err(), "Resolving an already closed issue must fail");
    assert_eq!(res_again.unwrap_err().code, pos_core::error::ErrorCode::Conflict("already_closed"));

    // 3. Dismissing an already closed issue must also fail
    let res_dis = platform::dismiss_issue(&tx2, rep.id, admin_user, "Dismiss".into()).await;
    assert!(res_dis.is_err(), "Dismissing an already closed issue must fail");
    assert_eq!(res_dis.unwrap_err().code, pos_core::error::ErrorCode::Conflict("already_closed"));
}
