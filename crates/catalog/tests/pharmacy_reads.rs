#![allow(clippy::unwrap_used, clippy::expect_used)]

use api::extract::Pagination;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use catalog::{queries, rest, sellable, types::*};
use db::Db;
use pos_core::ids::{CompanyId, DosageFormId, GenericId, ProductId, TenantId};
use sea_orm::Database;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use tower::ServiceExt;
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
async fn search_matches_a_misspelled_brand_name() {
    let pool = owner_pool().await;
    let db = get_db().await;
    let tenant = TenantId::new_v7();

    let uid = Uuid::now_v7().to_string();
    let suffix = &uid[..8];
    let company_name = format!("Pharma Co {suffix}");
    let cid = seed_company(&pool, &company_name).await;
    let gid = seed_generic(&pool, &format!("GenericName {suffix}")).await;
    let did = seed_dosage_form(&pool, &format!("Form {suffix}")).await;

    let pid = Uuid::now_v7();
    let brand = format!("Zithrocare {suffix}");
    sqlx::query!(
        r#"
        INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status, is_discontinued)
        VALUES ($1, $2, $3, $4, $5, '500 mg', 'allopathic', 'approved', false)
        "#,
        pid,
        cid.as_uuid(),
        gid.as_uuid(),
        did.as_uuid(),
        brand
    )
    .execute(&pool)
    .await
    .expect("insert product");

    // Misspell "Zithrocare" as "Zithrocre"
    let misspelled = format!("Zithrocre {suffix}");
    let res = queries::search(&db, tenant, &misspelled, Pagination { limit: 10, cursor: None })
        .await
        .expect("search");

    assert!(
        res.data.iter().any(|p| p.id.as_uuid() == pid),
        "Expected misspelled search '{misspelled}' to match '{brand}'"
    );
}

#[tokio::test]
async fn search_returns_my_pending_row_and_not_yours() {
    let pool = owner_pool().await;
    let db = get_db().await;
    let tenant_a = TenantId::new_v7();
    let tenant_b = TenantId::new_v7();

    let uid = Uuid::now_v7().to_string();
    let suffix = &uid[..8];
    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let did = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;

    // 1. Approved product
    let pid_app = Uuid::now_v7();
    let brand_app = format!("PanadolCore {suffix}");
    sqlx::query!(
        r#"
        INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status, is_discontinued)
        VALUES ($1, $2, $3, $4, $5, '500 mg', 'allopathic', 'approved', false)
        "#,
        pid_app, cid.as_uuid(), gid.as_uuid(), did.as_uuid(), brand_app
    ).execute(&pool).await.expect("insert approved");

    // 2. Pending product for tenant A
    let pid_a = Uuid::now_v7();
    let brand_a = format!("PanadolAlpha {suffix}");
    sqlx::query!(
        r#"
        INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status, submitted_by_tenant_id, is_discontinued)
        VALUES ($1, $2, $3, $4, $5, '500 mg', 'allopathic', 'pending', $6, false)
        "#,
        pid_a, cid.as_uuid(), gid.as_uuid(), did.as_uuid(), brand_a, tenant_a.as_uuid()
    ).execute(&pool).await.expect("insert pending a");

    // 3. Pending product for tenant B
    let pid_b = Uuid::now_v7();
    let brand_b = format!("PanadolBeta {suffix}");
    sqlx::query!(
        r#"
        INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status, submitted_by_tenant_id, is_discontinued)
        VALUES ($1, $2, $3, $4, $5, '500 mg', 'allopathic', 'pending', $6, false)
        "#,
        pid_b, cid.as_uuid(), gid.as_uuid(), did.as_uuid(), brand_b, tenant_b.as_uuid()
    ).execute(&pool).await.expect("insert pending b");

    // Search as tenant A
    let q = format!("Panadol {suffix}");
    let res_a = queries::search(&db, tenant_a, &q, Pagination { limit: 10, cursor: None })
        .await
        .expect("search a");
    let ids_a: Vec<Uuid> = res_a.data.iter().map(|p| p.id.as_uuid()).collect();
    assert!(ids_a.contains(&pid_app), "Tenant A should see approved product");
    assert!(ids_a.contains(&pid_a), "Tenant A should see their own pending product");
    assert!(!ids_a.contains(&pid_b), "Tenant A should NOT see Tenant B's pending product");

    // Search as tenant B
    let res_b = queries::search(&db, tenant_b, &q, Pagination { limit: 10, cursor: None })
        .await
        .expect("search b");
    let ids_b: Vec<Uuid> = res_b.data.iter().map(|p| p.id.as_uuid()).collect();
    assert!(ids_b.contains(&pid_app), "Tenant B should see approved product");
    assert!(ids_b.contains(&pid_b), "Tenant B should see their own pending product");
    assert!(!ids_b.contains(&pid_a), "Tenant B should NOT see Tenant A's pending product");
}

#[tokio::test]
async fn a_discontinued_product_is_readable_but_not_searched() {
    let pool = owner_pool().await;
    let db = get_db().await;
    let tenant = TenantId::new_v7();

    let uid = Uuid::now_v7().to_string();
    let suffix = &uid[..8];
    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let did = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;

    let pid = Uuid::now_v7();
    let brand = format!("DiscontinuedDrug {suffix}");
    sqlx::query!(
        r#"
        INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status, is_discontinued)
        VALUES ($1, $2, $3, $4, $5, '100 mg', 'allopathic', 'approved', true)
        "#,
        pid, cid.as_uuid(), gid.as_uuid(), did.as_uuid(), brand
    ).execute(&pool).await.expect("insert discontinued product");

    // Search for brand
    let res = queries::search(&db, tenant, &brand, Pagination { limit: 10, cursor: None })
        .await
        .expect("search");
    assert!(
        !res.data.iter().any(|p| p.id.as_uuid() == pid),
        "Discontinued product should not appear in search results"
    );

    // Readable by ID
    let prod = queries::product(&db, tenant, ProductId::from_uuid(pid))
        .await
        .expect("read product by id");
    assert_eq!(prod.id.as_uuid(), pid);
    assert!(prod.is_discontinued, "Product should have is_discontinued = true");

    // Sellable
    let sellable = sellable::is_sellable(&db, tenant, ProductId::from_uuid(pid))
        .await
        .expect("check is_sellable");
    assert!(sellable, "Discontinued product should remain sellable to clear inventory");
}

#[tokio::test]
async fn a_merged_row_reads_back_with_its_target() {
    let pool = owner_pool().await;
    let db = get_db().await;
    let tenant = TenantId::new_v7();

    let uid = Uuid::now_v7().to_string();
    let suffix = &uid[..8];
    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let did = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;

    // Target product
    let target_id = Uuid::now_v7();
    let target_brand = format!("TargetDrug {suffix}");
    sqlx::query!(
        r#"
        INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status, is_discontinued)
        VALUES ($1, $2, $3, $4, $5, '500 mg', 'allopathic', 'approved', false)
        "#,
        target_id, cid.as_uuid(), gid.as_uuid(), did.as_uuid(), target_brand
    ).execute(&pool).await.expect("insert target product");

    // Merged product pointing to target
    let merged_id = Uuid::now_v7();
    let merged_brand = format!("MergedDrug {suffix}");
    sqlx::query!(
        r#"
        INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status, merged_into_id, is_discontinued, submitted_by_tenant_id)
        VALUES ($1, $2, $3, $4, $5, '500 mg', 'allopathic', 'merged', $6, false, $7)
        "#,
        merged_id, cid.as_uuid(), gid.as_uuid(), did.as_uuid(), merged_brand, target_id, tenant.as_uuid()
    ).execute(&pool).await.expect("insert merged product");

    let read_back = queries::product(&db, tenant, ProductId::from_uuid(merged_id))
        .await
        .expect("read merged product");

    assert_eq!(read_back.status, RecordStatus::Merged);
    assert_eq!(read_back.merged_into_id, Some(ProductId::from_uuid(target_id)));
}

#[tokio::test]
async fn no_list_response_carries_monograph_text() {
    let pool = owner_pool().await;
    let db = get_db().await;
    let tenant = TenantId::new_v7();

    let uid = Uuid::now_v7().to_string();
    let suffix = &uid[..8];
    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let did = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;

    let secret_text = format!("SecretIndicationsText_{suffix}");
    sqlx::query!(
        r#"
        INSERT INTO generic_monographs (generic_id, indications, pharmacology)
        VALUES ($1, $2, 'Pharmacology text')
        ON CONFLICT (generic_id) DO UPDATE SET indications = EXCLUDED.indications
        "#,
        gid.as_uuid(),
        secret_text
    ).execute(&pool).await.expect("insert monograph");

    let pid = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status, is_discontinued)
        VALUES ($1, $2, $3, $4, $5, '500 mg', 'allopathic', 'approved', false)
        "#,
        pid, cid.as_uuid(), gid.as_uuid(), did.as_uuid(), format!("MonoProduct {suffix}")
    ).execute(&pool).await.expect("insert product");

    let list = queries::products(&db, tenant, Some(cid), None, None, Pagination { limit: 10, cursor: None })
        .await
        .expect("list products");

    let json = serde_json::to_string(&list).expect("serialize list");
    assert!(!json.contains(&secret_text), "List response should not carry monograph indications text");
    assert!(!json.contains("\"indications\""), "List response should not carry indications field");
    assert!(!json.contains("\"contraindications\""), "List response should not carry contraindications field");
}

#[tokio::test]
async fn a_generic_without_a_monograph_is_204() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let gid = seed_generic(&pool, &format!("GenericWithoutMono {}", Uuid::now_v7())).await;

    // 1. Direct query returns Ok(None)
    let mono = queries::monograph(&db, gid).await.expect("queries::monograph");
    assert!(mono.is_none(), "Generic without a monograph should return Ok(None)");

    // 2. HTTP endpoint returns 204 No Content
    let auth = api::AuthUser {
        user_id: pos_core::ids::UserId::new_v7(),
        tenant_id: TenantId::new_v7(),
        permissions: vec![catalog::perms::VIEW.to_string()],
        branch_permissions: std::collections::BTreeMap::new(),
    };

    let app = Router::from(rest::router())
        .layer(axum::extract::Extension(db))
        .layer(axum::extract::Extension(auth));

    let req = Request::builder()
        .uri(format!("/catalog/generics/{}/monograph", gid.as_uuid()))
        .method("GET")
        .body(Body::empty())
        .expect("build request");

    let response = app.oneshot(req).await.expect("execute request");
    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "Monograph read for generic with no text must return 204 No Content"
    );
}

#[tokio::test]
async fn catalog_lists_are_cursor_paginated() {
    let pool = owner_pool().await;
    let db = get_db().await;
    let tenant = TenantId::new_v7();

    let uid = Uuid::now_v7().to_string();
    let suffix = &uid[..8];
    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let did = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;

    let mut pids = [Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7()];
    pids.sort();

    for (i, pid) in pids.iter().enumerate() {
        sqlx::query!(
            r#"
            INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status, is_discontinued)
            VALUES ($1, $2, $3, $4, $5, '10 mg', 'allopathic', 'approved', false)
            "#,
            pid, cid.as_uuid(), gid.as_uuid(), did.as_uuid(), format!("PageProd{i} {suffix}")
        ).execute(&pool).await.expect("insert product");
    }

    // Page 1: limit 2
    let page1 = queries::products(&db, tenant, Some(cid), None, None, Pagination { limit: 2, cursor: None })
        .await
        .expect("page 1");
    assert_eq!(page1.data.len(), 2);
    assert_eq!(page1.data[0].id.as_uuid(), pids[0]);
    assert_eq!(page1.data[1].id.as_uuid(), pids[1]);
    assert!(page1.next_cursor.is_some(), "Page 1 should have next_cursor");

    // Page 2: limit 2, cursor = page1.next_cursor
    let page2 = queries::products(&db, tenant, Some(cid), None, None, Pagination { limit: 2, cursor: page1.next_cursor.clone() })
        .await
        .expect("page 2");
    assert_eq!(page2.data.len(), 1);
    assert_eq!(page2.data[0].id.as_uuid(), pids[2]);
    assert!(page2.next_cursor.is_none(), "Page 2 should not have next_cursor");

    // Ensure no total_count on the wire
    let json = serde_json::to_value(&page1).expect("serialize page");
    assert!(json.get("data").is_some());
    assert!(json.get("next_cursor").is_some());
    assert!(json.get("total_count").is_none());
    assert!(json.get("total").is_none());
}

#[tokio::test]
async fn lookups_is_three_queries() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let puid = Uuid::now_v7();
    sqlx::query!("INSERT INTO pack_units (id, name) VALUES ($1, $2) ON CONFLICT DO NOTHING", puid, format!("Unit {}", &puid.to_string()[..6]))
        .execute(&pool).await.expect("seed pack unit");

    let did = Uuid::now_v7();
    sqlx::query!("INSERT INTO dosage_forms (id, name, base_unit_id) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING", did, format!("Form {}", &did.to_string()[..6]), puid)
        .execute(&pool).await.expect("seed dosage form");

    let tcid = Uuid::now_v7();
    sqlx::query!("INSERT INTO therapeutic_classes (id, name) VALUES ($1, $2) ON CONFLICT DO NOTHING", tcid, format!("Class {}", &tcid.to_string()[..6]))
        .execute(&pool).await.expect("seed therapeutic class");

    let lookups = queries::lookups(&db).await.expect("queries::lookups");

    assert!(!lookups.dosage_forms.is_empty(), "dosage_forms should be populated");
    assert!(!lookups.pack_units.is_empty(), "pack_units should be populated");
    assert!(!lookups.therapeutic_classes.is_empty(), "therapeutic_classes should be populated");
}

#[tokio::test]
async fn is_sellable_follows_the_read_policy() {
    let pool = owner_pool().await;
    let db = get_db().await;
    let tenant_a = TenantId::new_v7();
    let tenant_b = TenantId::new_v7();

    let uid = Uuid::now_v7().to_string();
    let suffix = &uid[..8];
    let cid = seed_company(&pool, &format!("Company {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("Generic {}", Uuid::now_v7())).await;
    let did = seed_dosage_form(&pool, &format!("Dosage {}", Uuid::now_v7())).await;

    // 1. Approved active product -> sellable for both tenants
    let pid_approved = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status, is_discontinued)
        VALUES ($1, $2, $3, $4, $5, '500 mg', 'allopathic', 'approved', false)
        "#,
        pid_approved, cid.as_uuid(), gid.as_uuid(), did.as_uuid(), format!("Approved {suffix}")
    ).execute(&pool).await.expect("insert approved");

    assert!(sellable::is_sellable(&db, tenant_a, ProductId::from_uuid(pid_approved)).await.unwrap());
    assert!(sellable::is_sellable(&db, tenant_b, ProductId::from_uuid(pid_approved)).await.unwrap());

    // 2. Approved discontinued product -> sellable
    let pid_disc = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status, is_discontinued)
        VALUES ($1, $2, $3, $4, $5, '500 mg', 'allopathic', 'approved', true)
        "#,
        pid_disc, cid.as_uuid(), gid.as_uuid(), did.as_uuid(), format!("Disc {suffix}")
    ).execute(&pool).await.expect("insert discontinued");

    assert!(sellable::is_sellable(&db, tenant_a, ProductId::from_uuid(pid_disc)).await.unwrap());

    // 3. Merged product -> not sellable
    let pid_merged = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status, merged_into_id, is_discontinued)
        VALUES ($1, $2, $3, $4, $5, '500 mg', 'allopathic', 'merged', $6, false)
        "#,
        pid_merged, cid.as_uuid(), gid.as_uuid(), did.as_uuid(), format!("Merged {suffix}"), pid_approved
    ).execute(&pool).await.expect("insert merged");

    assert!(!sellable::is_sellable(&db, tenant_a, ProductId::from_uuid(pid_merged)).await.unwrap());

    // 4. Pending product of Tenant A -> sellable to Tenant A, NOT to Tenant B
    let pid_pend_a = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, strength, product_type, status, submitted_by_tenant_id, is_discontinued)
        VALUES ($1, $2, $3, $4, $5, '500 mg', 'allopathic', 'pending', $6, false)
        "#,
        pid_pend_a, cid.as_uuid(), gid.as_uuid(), did.as_uuid(), format!("PendingA {suffix}"), tenant_a.as_uuid()
    ).execute(&pool).await.expect("insert pending a");

    assert!(sellable::is_sellable(&db, tenant_a, ProductId::from_uuid(pid_pend_a)).await.unwrap());
    assert!(!sellable::is_sellable(&db, tenant_b, ProductId::from_uuid(pid_pend_a)).await.unwrap());

    // 5. Non-existent product -> false
    assert!(!sellable::is_sellable(&db, tenant_a, ProductId::new_v7()).await.unwrap());
}
