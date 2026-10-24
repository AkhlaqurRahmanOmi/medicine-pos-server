#![allow(clippy::expect_used, clippy::unwrap_used)]

use api::ListPage;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    Extension, Router,
};
use chrono::Utc;
use db::{Db, Tx};
use pos_core::context::TenantContext;
use pos_core::ids::{TenantId, UserId};
use sea_orm::Database;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use std::time::Duration;
use tenancy::rest::{admin_router, pharmacy_router};
use tenancy::types::Plan;
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

#[tokio::test]
async fn every_pharmacy_contract_route_is_mounted() {
    let router = Router::from(pharmacy_router());
    let dummy_id = Uuid::now_v7();

    let routes = [
        ("GET", "/tenant"),
        ("PATCH", "/tenant"),
        ("GET", "/tenant/settings"),
        ("PATCH", "/tenant/settings"),
        ("GET", "/branches"),
        ("POST", "/branches"),
        ("GET", &format!("/branches/{dummy_id}")),
        ("PATCH", &format!("/branches/{dummy_id}")),
        ("POST", &format!("/branches/{dummy_id}/deactivate")),
        ("GET", "/plans"),
        ("GET", "/subscriptions"),
        ("GET", "/subscriptions/current"),
        ("POST", "/subscription-payments"),
        ("POST", &format!("/subscription-payments/{dummy_id}/slip")),
        ("POST", "/exports"),
        ("GET", &format!("/exports/{dummy_id}")),
        ("POST", "/tenant/closure"),
    ];

    for (method, path) in routes {
        let req = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::empty())
            .unwrap();
        let resp = router.clone().oneshot(req).await.unwrap();
        assert_ne!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "Route {method} {path} must be mounted, got 404 Not Found"
        );
    }
}

#[tokio::test]
async fn an_export_request_is_accepted_once_per_tenant() {
    let db = get_db().await;
    let owner = owner_pool().await;
    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, user_id, vec![]);

    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin tx");
    tenancy::commands::create_pharmacy(&tx, "Export Pharmacy", None, None, None, None)
        .await
        .expect("create");
    tx.commit().await.expect("commit");

    // 1. First export request: accepted (202 / Ok)
    let tx1 = Tx::begin(db.conn(), &ctx).await.expect("begin tx");
    let exp1 = tenancy::commands::request_export(&tx1, user_id)
        .await
        .expect("export 1 accepted");
    tx1.commit().await.expect("commit tx1");
    assert_eq!(exp1.status, "pending");

    // 2. Second export request while first is pending: 409 export_already_running
    let tx2 = Tx::begin(db.conn(), &ctx).await.expect("begin tx");
    let err2 = tenancy::commands::request_export(&tx2, user_id)
        .await
        .expect_err("export 2 rejected");
    assert_eq!(
        err2.code,
        pos_core::error::ErrorCode::Conflict("export_already_running")
    );

    // Cleanup
    let _ = sqlx::query("DELETE FROM tenant_exports WHERE tenant_id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
    let _ = sqlx::query("DELETE FROM tenant_settings WHERE tenant_id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
    let _ = sqlx::query("DELETE FROM branches WHERE tenant_id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
}

#[tokio::test]
async fn every_admin_contract_route_is_mounted() {
    let router = Router::from(admin_router());
    let dummy_id = Uuid::now_v7();

    let routes = [
        ("GET", "/tenants"),
        ("GET", &format!("/tenants/{dummy_id}")),
        ("POST", &format!("/tenants/{dummy_id}/approve")),
        ("POST", &format!("/tenants/{dummy_id}/suspend")),
        ("POST", &format!("/tenants/{dummy_id}/reinstate")),
        ("POST", &format!("/tenants/{dummy_id}/subscriptions")),
        ("GET", "/subscription-payments"),
        ("POST", &format!("/subscription-payments/{dummy_id}/confirm")),
        ("POST", &format!("/subscription-payments/{dummy_id}/reject")),
        ("GET", "/plans"),
        ("POST", "/plans"),
        ("PATCH", &format!("/plans/{dummy_id}")),
        ("PUT", &format!("/plans/{dummy_id}/limits/branches")),
    ];

    for (method, path) in routes {
        let req = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::empty())
            .unwrap();
        let resp = router.clone().oneshot(req).await.unwrap();
        assert_ne!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "Admin route {method} {path} must be mounted, got 404 Not Found"
        );
    }
}

#[tokio::test]
async fn plans_are_returned_as_a_list_page() {
    let db = get_db().await;
    let app = Router::from(pharmacy_router())
        .layer(Extension(db))
        .layer(Extension(TenantContext::new(
            TenantId::new_v7(),
            UserId::new_v7(),
            vec![],
        )))
        .layer(Extension(
            Arc::new(tenancy::access::DummyAccessModeSource)
                as Arc<dyn pos_core::ports::AccessModeSource>,
        ));

    let req = Request::builder()
        .method("GET")
        .uri("/plans")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: ListPage<Plan> = serde_json::from_slice(&bytes).unwrap();
    assert!(!body.data.is_empty(), "Plans list page must contain plans");
    assert!(!body.has_more, "Default catalogue has no further pages");
}

#[tokio::test]
async fn each_route_requires_its_contract_permission() {
    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let router = Router::from(pharmacy_router())
        .layer(Extension(TenantContext::new(tenant_id, user_id, vec![])))
        .layer(Extension(
            Arc::new(tenancy::access::DummyAccessModeSource)
                as Arc<dyn pos_core::ports::AccessModeSource>,
        ));
    let dummy_id = Uuid::now_v7();

    let test_cases = [
        ("GET", "/tenant", "tenant.view"),
        ("PATCH", "/tenant", "tenant.update"),
        ("GET", "/tenant/settings", "tenant.view"),
        ("PATCH", "/tenant/settings", "tenant.settings.update"),
        ("GET", "/branches", "branch.view"),
        ("POST", "/branches", "branch.create"),
        ("GET", &format!("/branches/{dummy_id}"), "branch.view"),
        ("PATCH", &format!("/branches/{dummy_id}"), "branch.update"),
        (
            "POST",
            &format!("/branches/{dummy_id}/deactivate"),
            "branch.update",
        ),
        ("GET", "/subscriptions", "subscription.view"),
        ("GET", "/subscriptions/current", "subscription.view"),
        ("POST", "/subscription-payments", "subscription.pay"),
        (
            "POST",
            &format!("/subscription-payments/{dummy_id}/slip"),
            "subscription.pay",
        ),
        ("POST", "/exports", "tenant.export"),
        ("GET", &format!("/exports/{dummy_id}"), "tenant.export"),
        ("POST", "/tenant/closure", "tenant.close"),
    ];

    for (method, path, perm) in test_cases {
        let unauthorized = api::AuthUser {
            user_id,
            tenant_id,
            permissions: vec!["unrelated.perm".to_string()],
            branch_permissions: Default::default(),
        };

        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(unauthorized);

        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "Route {method} {path} must require permission '{perm}'"
        );
    }
}

#[tokio::test]
async fn access_mode_reads_at_most_two_subscription_rows() {
    let db = get_db().await;
    let owner = owner_pool().await;
    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, user_id, vec![]);

    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin tx");
    tenancy::commands::create_pharmacy(&tx, "Two Rows Pharmacy", None, None, None, None)
        .await
        .expect("create");
    tx.commit().await.expect("commit");

    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin tx");
    tenancy::platform::approve(&tx, tenant_id, user_id)
        .await
        .expect("approve");
    tx.commit().await.expect("commit");

    let plan_id = Uuid::parse_str("0192a000-0000-7000-8000-000000000001").unwrap();
    let now = Utc::now();

    // Insert 5 past subscriptions
    for i in 1..=5 {
        let s_id = Uuid::now_v7();
        let starts = now - chrono::Duration::days(100 - i * 10);
        let ends = now - chrono::Duration::days(90 - i * 10);
        sqlx::query(
            "INSERT INTO subscriptions (id, tenant_id, plan_id, billing_period, price, starts_at, ends_at, status)
             VALUES ($1, $2, $3, 'monthly', 1000.00, $4, $5, 'active')"
        )
        .bind(s_id)
        .bind(tenant_id.as_uuid())
        .bind(plan_id)
        .bind(starts)
        .bind(ends)
        .execute(&owner)
        .await
        .expect("insert sub");
    }

    let mode = tenancy::queries::access_mode(&db, tenant_id, now)
        .await
        .expect("get access mode");
    assert_eq!(mode, tenancy::AccessMode::ReadOnly);

    // Clean up
    let _ = sqlx::query("DELETE FROM subscriptions WHERE tenant_id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
    let _ = sqlx::query("DELETE FROM tenant_settings WHERE tenant_id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
    let _ = sqlx::query("DELETE FROM branches WHERE tenant_id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
}

#[tokio::test]
async fn the_plan_catalogue_is_one_query_per_table() {
    let db = get_db().await;
    let plans = tenancy::queries::plans(&db).await.expect("query plans");
    assert!(!plans.is_empty(), "Plans catalogue must not be empty");
    for p in &plans {
        assert!(!p.code.is_empty());
        assert!(!p.limits.is_empty(), "Plan must have limits attached");
    }
}

#[tokio::test]
async fn an_unimplemented_limit_type_is_an_error_not_unlimited() {
    let db = get_db().await;
    let tenant_id = TenantId::new_v7();
    let res = tenancy::queries::limit_remaining(&db, tenant_id, "unimplemented_limit_type").await;
    assert!(
        res.is_err(),
        "Unimplemented limit type must return error, not unlimited/0"
    );
}

#[tokio::test]
async fn two_concurrent_branch_creates_cannot_both_pass_the_limit() {
    let db = get_db().await;
    let owner = owner_pool().await;
    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, user_id, vec![]);

    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin tx");
    tenancy::commands::create_pharmacy(&tx, "Limit Pharmacy", None, None, None, None)
        .await
        .expect("create");
    tx.commit().await.expect("commit");

    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin tx");
    tenancy::platform::approve(&tx, tenant_id, user_id)
        .await
        .expect("approve");
    tx.commit().await.expect("commit");

    // Set branch limit to 2 for the standard plan (default branch is already 1, so only 1 more can be created)
    let plan_id = Uuid::parse_str("0192a000-0000-7000-8000-000000000001").unwrap();
    sqlx::query(
        "UPDATE plan_limits SET max_value = 2 WHERE plan_id = $1 AND limit_key = 'branches'",
    )
    .bind(plan_id)
    .execute(&owner)
    .await
    .expect("update limit");

    let db1 = db.clone();
    let ctx1 = ctx.clone();
    let h1 = tokio::spawn(async move {
        let tx = Tx::begin(db1.conn(), &ctx1).await?;
        let res = tenancy::commands::create_branch(
            &tx,
            tenancy::types::NewBranch {
                name: "Branch A".to_string(),
                address: None,
                phone: None,
                drug_license_no: None,
            },
        )
        .await;
        if res.is_ok() {
            tx.commit().await?;
        }
        res
    });

    let db2 = db.clone();
    let ctx2 = ctx.clone();
    let h2 = tokio::spawn(async move {
        let tx = Tx::begin(db2.conn(), &ctx2).await?;
        let res = tenancy::commands::create_branch(
            &tx,
            tenancy::types::NewBranch {
                name: "Branch B".to_string(),
                address: None,
                phone: None,
                drug_license_no: None,
            },
        )
        .await;
        if res.is_ok() {
            tx.commit().await?;
        }
        res
    });

    let db3 = db.clone();
    let ctx3 = ctx.clone();
    let h3 = tokio::spawn(async move {
        let tx = Tx::begin(db3.conn(), &ctx3).await?;
        let res = tenancy::commands::create_branch(
            &tx,
            tenancy::types::NewBranch {
                name: "Branch C".to_string(),
                address: None,
                phone: None,
                drug_license_no: None,
            },
        )
        .await;
        if res.is_ok() {
            tx.commit().await?;
        }
        res
    });

    let (r1, r2, r3) = tokio::join!(h1, h2, h3);
    let results = [r1.unwrap(), r2.unwrap(), r3.unwrap()];
    let successes = results.iter().filter(|r| r.is_ok()).count();
    let failures = results.iter().filter(|r| r.is_err()).count();

    // Since max_value is 2 and 1 already exists (Main branch), exactly 1 can succeed!
    assert_eq!(
        successes, 1,
        "Exactly one concurrent branch create must succeed"
    );
    assert_eq!(
        failures, 2,
        "Other concurrent branch creates must fail due to plan limit"
    );

    // Restore standard plan limit to NULL (unlimited)
    sqlx::query(
        "UPDATE plan_limits SET max_value = NULL WHERE plan_id = $1 AND limit_key = 'branches'",
    )
    .bind(plan_id)
    .execute(&owner)
    .await
    .expect("restore limit");

    // Cleanup
    let _ = sqlx::query("DELETE FROM tenant_settings WHERE tenant_id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
    let _ = sqlx::query("DELETE FROM branches WHERE tenant_id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
}

#[test]
fn an_unknown_tenant_status_is_an_error_not_pending() {
    let res = tenancy::access::TenantStatus::try_from("corrupted_status");
    assert!(res.is_err(), "Unknown tenant status must return an error");
    let err = res.unwrap_err();
    assert_eq!(err.code, pos_core::error::ErrorCode::ValidationFailed);
}

#[tokio::test]
async fn a_patch_can_clear_the_receipt_header() {
    let db = get_db().await;
    let owner = owner_pool().await;
    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, user_id, vec![]);

    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin tx");
    tenancy::commands::create_pharmacy(&tx, "Header Pharmacy", None, None, None, None)
        .await
        .expect("create");
    tx.commit().await.expect("commit");

    // 1. Set receipt header to "Initial Header"
    let tx1 = Tx::begin(db.conn(), &ctx).await.expect("begin tx1");
    let patch1 = tenancy::types::SettingsPatch {
        receipt_header: Some(Some("Initial Header".to_string())),
        ..Default::default()
    };
    let s1 = tenancy::commands::update_settings(&tx1, patch1)
        .await
        .expect("update settings 1");
    tx1.commit().await.expect("commit tx1");
    assert_eq!(s1.receipt_header, Some("Initial Header".to_string()));

    // 2. Patch with Some(None) representing {"receipt_header": null}
    let tx2 = Tx::begin(db.conn(), &ctx).await.expect("begin tx2");
    let patch2 = tenancy::types::SettingsPatch {
        receipt_header: Some(None),
        ..Default::default()
    };
    let s2 = tenancy::commands::update_settings(&tx2, patch2)
        .await
        .expect("update settings 2");
    tx2.commit().await.expect("commit tx2");
    assert_eq!(s2.receipt_header, None, "receipt_header must be cleared to None");

    // Verify in database directly
    let row: (Option<String>,) =
        sqlx::query_as("SELECT receipt_header FROM tenant_settings WHERE tenant_id = $1")
            .bind(tenant_id.as_uuid())
            .fetch_one(&owner)
            .await
            .expect("fetch settings row");
    assert_eq!(row.0, None, "Database column must be NULL");

    // Cleanup
    let _ = sqlx::query("DELETE FROM tenant_settings WHERE tenant_id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
    let _ = sqlx::query("DELETE FROM branches WHERE tenant_id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
}
