#![allow(clippy::unwrap_used, clippy::expect_used)]

use api::extract::AuthUser;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use catalog::{commands, perms, rest, types::*};
use db::{Db, Tx};
use pos_core::context::TenantContext;
use pos_core::ids::{CompanyId, DosageFormId, GenericId, PackUnitId, TenantId, UserId};
use sea_orm::Database;
use sqlx::postgres::PgPoolOptions;
use std::collections::BTreeMap;
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

fn make_auth(tenant_id: TenantId, user_id: UserId, perms: &[&str]) -> AuthUser {
    AuthUser {
        user_id,
        tenant_id,
        permissions: perms.iter().map(|s| s.to_string()).collect(),
        branch_permissions: BTreeMap::new(),
    }
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
async fn catalog_routes_are_mounted() {
    let pharmacy_app = Router::from(rest::pharmacy_router());
    let admin_app = Router::from(rest::admin_router());

    let test_uuid = Uuid::now_v7();

    // 1. Verify pharmacy routes are mounted on pharmacy_app (none return 404)
    let pharmacy_endpoints = [
        ("GET", "/catalog/search?q=test".to_string()),
        ("GET", "/catalog/products".to_string()),
        ("GET", format!("/catalog/products/{test_uuid}")),
        ("GET", format!("/catalog/generics/{test_uuid}")),
        ("GET", format!("/catalog/generics/{test_uuid}/monograph")),
        ("GET", "/catalog/companies".to_string()),
        ("GET", "/catalog/lookups".to_string()),
        ("POST", "/catalog/companies".to_string()),
        ("POST", "/catalog/generics".to_string()),
        ("POST", "/catalog/products".to_string()),
        ("GET", "/catalog/submissions".to_string()),
        ("POST", "/catalog/issues".to_string()),
        ("GET", "/catalog/issues".to_string()),
    ];

    for (method, path) in &pharmacy_endpoints {
        let req = Request::builder()
            .method(*method)
            .uri(path)
            .body(Body::empty())
            .unwrap();
        let resp = pharmacy_app.clone().oneshot(req).await.unwrap();
        assert_ne!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "Pharmacy route {method} {path} must be mounted, got 404"
        );
    }

    // 2. Verify pharmacy router does NOT mount admin routes (must return 404)
    let admin_on_pharmacy = [
        ("GET", "/catalog/queue".to_string()),
        ("POST", format!("/catalog/product/{test_uuid}/approve")),
    ];
    for (method, path) in &admin_on_pharmacy {
        let req = Request::builder()
            .method(*method)
            .uri(path)
            .body(Body::empty())
            .unwrap();
        let resp = pharmacy_app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "Admin route {method} {path} must NOT be mounted on pharmacy router"
        );
    }

    // 3. Verify admin routes are mounted on admin_app (none return 404)
    let admin_endpoints = [
        ("GET", "/catalog/queue".to_string()),
        ("GET", "/catalog/issues".to_string()),
        ("POST", format!("/catalog/product/{test_uuid}/approve")),
        ("POST", format!("/catalog/product/{test_uuid}/reject")),
        ("POST", format!("/catalog/product/{test_uuid}/merge")),
        ("PUT", format!("/catalog/generics/{test_uuid}/monograph")),
        ("POST", format!("/catalog/issues/{test_uuid}/resolve")),
        ("POST", format!("/catalog/issues/{test_uuid}/dismiss")),
    ];

    for (method, path) in &admin_endpoints {
        let req = Request::builder()
            .method(*method)
            .uri(path)
            .body(Body::empty())
            .unwrap();
        let resp = admin_app.clone().oneshot(req).await.unwrap();
        assert_ne!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "Admin route {method} {path} must be mounted, got 404"
        );
    }

    // 4. Verify admin router does NOT mount pharmacy routes (must return 404)
    let pharmacy_on_admin = [
        ("GET", "/catalog/search".to_string()),
        ("GET", "/catalog/products".to_string()),
        ("POST", "/catalog/products".to_string()),
    ];
    for (method, path) in &pharmacy_on_admin {
        let req = Request::builder()
            .method(*method)
            .uri(path)
            .body(Body::empty())
            .unwrap();
        let resp = admin_app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "Pharmacy route {method} {path} must NOT be mounted on admin router"
        );
    }
}

#[tokio::test]
async fn each_route_requires_its_contract_permission() {
    let pharmacy_app = Router::from(rest::pharmacy_router());
    let admin_app = Router::from(rest::admin_router());

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();

    // 1. Pharmacy route requiring catalog.view
    // With only catalog.submit -> 403 Forbidden
    let mut req = Request::builder()
        .method("GET")
        .uri("/catalog/products")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(make_auth(tenant_id, user_id, &[perms::SUBMIT]));
    let resp = pharmacy_app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "GET /catalog/products without catalog.view must return 403"
    );

    // With catalog.view -> passes permission gate (not 403)
    let mut req = Request::builder()
        .method("GET")
        .uri("/catalog/products")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(make_auth(tenant_id, user_id, &[perms::VIEW]));
    let resp = pharmacy_app.clone().oneshot(req).await.unwrap();
    assert_ne!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "GET /catalog/products with catalog.view must pass permission gate"
    );

    // 2. Pharmacy route requiring catalog.submit
    // With only catalog.view -> 403 Forbidden
    let mut req = Request::builder()
        .method("POST")
        .uri("/catalog/products")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(make_auth(tenant_id, user_id, &[perms::VIEW]));
    let resp = pharmacy_app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "POST /catalog/products without catalog.submit must return 403"
    );

    // With catalog.submit -> passes permission gate (not 403)
    let mut req = Request::builder()
        .method("POST")
        .uri("/catalog/products")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(make_auth(tenant_id, user_id, &[perms::SUBMIT]));
    let resp = pharmacy_app.clone().oneshot(req).await.unwrap();
    assert_ne!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "POST /catalog/products with catalog.submit must pass permission gate"
    );

    // GET /catalog/submissions requires catalog.submit
    let mut req = Request::builder()
        .method("GET")
        .uri("/catalog/submissions")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(make_auth(tenant_id, user_id, &[perms::VIEW]));
    let resp = pharmacy_app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "GET /catalog/submissions without catalog.submit must return 403"
    );

    // 3. Pharmacy route requiring catalog.issue.report
    // With only catalog.view -> 403 Forbidden
    let mut req = Request::builder()
        .method("POST")
        .uri("/catalog/issues")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(make_auth(tenant_id, user_id, &[perms::VIEW]));
    let resp = pharmacy_app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "POST /catalog/issues without catalog.issue.report must return 403"
    );

    // With catalog.issue.report -> passes permission gate (not 403)
    let mut req = Request::builder()
        .method("POST")
        .uri("/catalog/issues")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(make_auth(tenant_id, user_id, &[perms::ISSUE_REPORT]));
    let resp = pharmacy_app.clone().oneshot(req).await.unwrap();
    assert_ne!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "POST /catalog/issues with catalog.issue.report must pass permission gate"
    );

    // 4. Admin route requiring catalog.review
    // With only pharmacy permissions -> 403 Forbidden
    let mut req = Request::builder()
        .method("GET")
        .uri("/catalog/queue")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(make_auth(tenant_id, user_id, &[perms::VIEW, perms::SUBMIT]));
    let resp = admin_app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "GET /catalog/queue without catalog.review must return 403"
    );

    // With catalog.review -> passes permission gate (not 403)
    let mut req = Request::builder()
        .method("GET")
        .uri("/catalog/queue")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(make_auth(tenant_id, user_id, &[perms::REVIEW]));
    let resp = admin_app.clone().oneshot(req).await.unwrap();
    assert_ne!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "GET /catalog/queue with catalog.review must pass permission gate"
    );
}

#[tokio::test]
async fn each_record_kind_has_its_own_review_handler() {
    let pool = owner_pool().await;
    let db = get_db().await;

    let admin_user = UserId::new_v7();
    let admin_ctx = TenantContext::new(TenantId::from_uuid(Uuid::nil()), admin_user, vec![]);

    let tenant_id = TenantId::new_v7();
    let tenant_user = UserId::new_v7();
    let tenant_ctx = TenantContext::new(tenant_id, tenant_user, vec![]);

    // 1. Company review handlers
    let company_name = format!("HandlerComp {}", Uuid::now_v7());
    let tx_sub = Tx::begin(db.conn(), &tenant_ctx).await.expect("begin tx");
    let comp = commands::submit_company(&tx_sub, NewCompany {
        name: company_name.clone(),
        medex_id: None,
    })
    .await
    .expect("submit company");
    tx_sub.commit().await.expect("commit tx");

    let tx_admin = Tx::begin_platform(db.conn(), &admin_ctx).await.expect("begin platform tx");
    let approved_comp = rest::approve_company_handler(&tx_admin, comp.id.as_uuid(), admin_user)
        .await
        .expect("approve_company_handler");
    tx_admin.commit().await.expect("commit platform tx");
    assert_eq!(approved_comp.status, RecordStatus::Approved);

    // Calling approve again returns already_reviewed
    let tx_admin2 = Tx::begin_platform(db.conn(), &admin_ctx).await.expect("begin platform tx");
    let duplicate_review = rest::approve_company_handler(&tx_admin2, comp.id.as_uuid(), admin_user).await;
    assert!(duplicate_review.is_err(), "already approved company cannot be approved again");

    // reject_company_handler
    let tx_sub = Tx::begin(db.conn(), &tenant_ctx).await.expect("begin tx");
    let comp_reject = commands::submit_company(
        &tx_sub,
        NewCompany {
            name: format!("CompReject {}", Uuid::now_v7()),
            medex_id: None,
        },
    )
    .await
    .expect("submit company");
    tx_sub.commit().await.expect("commit tx");

    let tx_admin = Tx::begin_platform(db.conn(), &admin_ctx).await.expect("begin platform tx");
    let rejected_comp = rest::reject_company_handler(&tx_admin, comp_reject.id.as_uuid(), admin_user, "Rejected".into())
        .await
        .expect("reject_company_handler");
    tx_admin.commit().await.expect("commit platform tx");
    assert_eq!(rejected_comp.status, RecordStatus::Rejected);

    // merge_company_handler
    let tx_sub = Tx::begin(db.conn(), &tenant_ctx).await.expect("begin tx");
    let comp_merge = commands::submit_company(
        &tx_sub,
        NewCompany {
            name: format!("CompMerge {}", Uuid::now_v7()),
            medex_id: None,
        },
    )
    .await
    .expect("submit company");
    tx_sub.commit().await.expect("commit tx");

    let tx_admin = Tx::begin_platform(db.conn(), &admin_ctx).await.expect("begin platform tx");
    let merged_comp = rest::merge_company_handler(&tx_admin, comp_merge.id.as_uuid(), comp.id.as_uuid(), admin_user)
        .await
        .expect("merge_company_handler");
    tx_admin.commit().await.expect("commit platform tx");
    assert_eq!(merged_comp.status, RecordStatus::Merged);

    // 2. Generic review handlers
    let generic_name = format!("HandlerGen {}", Uuid::now_v7());
    let tx_sub = Tx::begin(db.conn(), &tenant_ctx).await.expect("begin tx");
    let gen = commands::submit_generic(&tx_sub, NewGeneric {
        name: generic_name.clone(),
        medex_id: None,
        ingredient_ids: vec![],
        therapeutic_class_ids: vec![],
    })
    .await
    .expect("submit generic");
    tx_sub.commit().await.expect("commit tx");

    let tx_admin = Tx::begin_platform(db.conn(), &admin_ctx).await.expect("begin platform tx");
    let approved_gen = rest::approve_generic_handler(&tx_admin, gen.id.as_uuid(), admin_user)
        .await
        .expect("approve_generic_handler");
    tx_admin.commit().await.expect("commit platform tx");
    assert_eq!(approved_gen.status, RecordStatus::Approved);

    // reject_generic_handler
    let tx_sub = Tx::begin(db.conn(), &tenant_ctx).await.expect("begin tx");
    let gen_reject = commands::submit_generic(
        &tx_sub,
        NewGeneric {
            name: format!("GenReject {}", Uuid::now_v7()),
            medex_id: None,
            ingredient_ids: vec![],
            therapeutic_class_ids: vec![],
        },
    )
    .await
    .expect("submit generic");
    tx_sub.commit().await.expect("commit tx");

    let tx_admin = Tx::begin_platform(db.conn(), &admin_ctx).await.expect("begin platform tx");
    let rejected_gen = rest::reject_generic_handler(
        &tx_admin,
        gen_reject.id.as_uuid(),
        admin_user,
        "Duplicate formulation".to_string(),
    )
    .await
    .expect("reject_generic_handler");
    tx_admin.commit().await.expect("commit platform tx");
    assert_eq!(rejected_gen.status, RecordStatus::Rejected);

    // merge_generic_handler
    let tx_sub = Tx::begin(db.conn(), &tenant_ctx).await.expect("begin tx");
    let gen_merge = commands::submit_generic(
        &tx_sub,
        NewGeneric {
            name: format!("GenMerge {}", Uuid::now_v7()),
            medex_id: None,
            ingredient_ids: vec![],
            therapeutic_class_ids: vec![],
        },
    )
    .await
    .expect("submit generic");
    tx_sub.commit().await.expect("commit tx");

    let tx_admin = Tx::begin_platform(db.conn(), &admin_ctx).await.expect("begin platform tx");
    let merged_gen = rest::merge_generic_handler(&tx_admin, gen_merge.id.as_uuid(), gen.id.as_uuid(), admin_user)
        .await
        .expect("merge_generic_handler");
    tx_admin.commit().await.expect("commit platform tx");
    assert_eq!(merged_gen.status, RecordStatus::Merged);

    // 3. Product review handlers
    let cid = seed_company(&pool, &format!("CompTarget {}", Uuid::now_v7())).await;
    let gid = seed_generic(&pool, &format!("GenTarget {}", Uuid::now_v7())).await;
    let dfid = seed_dosage_form(&pool, &format!("DosageTarget {}", Uuid::now_v7())).await;

    // Seed target approved product
    let target_prod_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, product_type, status)
         VALUES ($1, $2, $3, $4, $5, 'allopathic', 'approved')",
        target_prod_id,
        cid.as_uuid(),
        gid.as_uuid(),
        dfid.as_uuid(),
        format!("BrandTarget {}", Uuid::now_v7()),
    )
    .execute(&pool)
    .await
    .expect("seed target product");

    // approve_product_handler
    let tx_sub = Tx::begin(db.conn(), &tenant_ctx).await.expect("begin tx");
    let prod_approve = commands::submit_product(&tx_sub, NewProduct {
        company_id: cid,
        generic_id: Some(gid),
        dosage_form_id: Some(dfid),
        brand_name: format!("BrandApprove {}", Uuid::now_v7()),
        strength: Some("500 mg".into()),
        product_type: ProductType::Allopathic,
        reference_unit_mrp: None,
        pack: None,
    })
    .await
    .expect("submit product");
    tx_sub.commit().await.expect("commit tx");

    let tx_admin = Tx::begin_platform(db.conn(), &admin_ctx).await.expect("begin platform tx");
    let approved_prod = rest::approve_product_handler(&tx_admin, prod_approve.id.as_uuid(), admin_user)
        .await
        .expect("approve_product_handler");
    tx_admin.commit().await.expect("commit platform tx");
    assert_eq!(approved_prod.status, RecordStatus::Approved);

    // reject_product_handler
    let tx_sub = Tx::begin(db.conn(), &tenant_ctx).await.expect("begin tx");
    let prod_reject = commands::submit_product(&tx_sub, NewProduct {
        company_id: cid,
        generic_id: Some(gid),
        dosage_form_id: Some(dfid),
        brand_name: format!("BrandReject {}", Uuid::now_v7()),
        strength: Some("500 mg".into()),
        product_type: ProductType::Allopathic,
        reference_unit_mrp: None,
        pack: None,
    })
    .await
    .expect("submit product");
    tx_sub.commit().await.expect("commit tx");

    let tx_admin = Tx::begin_platform(db.conn(), &admin_ctx).await.expect("begin platform tx");
    let rejected_prod = rest::reject_product_handler(&tx_admin, prod_reject.id.as_uuid(), admin_user, "Rejected".into())
        .await
        .expect("reject_product_handler");
    tx_admin.commit().await.expect("commit platform tx");
    assert_eq!(rejected_prod.status, RecordStatus::Rejected);

    // merge_product_handler
    let tx_sub = Tx::begin(db.conn(), &tenant_ctx).await.expect("begin tx");
    let prod = commands::submit_product(&tx_sub, NewProduct {
        company_id: cid,
        generic_id: Some(gid),
        dosage_form_id: Some(dfid),
        brand_name: format!("BrandSource {}", Uuid::now_v7()),
        strength: Some("500 mg".into()),
        product_type: ProductType::Allopathic,
        reference_unit_mrp: None,
        pack: None,
    })
    .await
    .expect("submit product");
    tx_sub.commit().await.expect("commit tx");

    let tx_admin = Tx::begin_platform(db.conn(), &admin_ctx).await.expect("begin platform tx");
    let merged_prod = rest::merge_product_handler(
        &tx_admin,
        prod.id.as_uuid(),
        target_prod_id,
        admin_user,
    )
    .await
    .expect("merge_product_handler");
    tx_admin.commit().await.expect("commit platform tx");
    assert_eq!(merged_prod.status, RecordStatus::Merged);
    assert_eq!(merged_prod.merged_into_id, Some(target_prod_id));
}
