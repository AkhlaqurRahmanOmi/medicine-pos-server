use api::{AuthUser, PlatformToken};
use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use chrono::Utc;
use db::{Db, Tx};
use pos_core::context::TenantContext;
use pos_core::ids::{TenantId, UserId};
use rust_decimal::Decimal;
use sea_orm::Database;
use sqlx::postgres::PgPoolOptions;
use std::str::FromStr;
use std::time::Duration;
use tenancy::commands::create_pharmacy;
use tenancy::platform::{approve, suspend};
use tenancy::queries;
use tenancy::rest::admin_router;
use tenancy::types::{Payment, PlanPrice, Subscription, TenantSettings};
use tower::ServiceExt;
use uuid::Uuid;

const APP_URL: &str = "postgres://pos_app:devpass@127.0.0.1:5432/medicine_pos";
const OWNER_URL: &str = "postgres://pos_owner:devpass@127.0.0.1:5432/medicine_pos";

async fn get_db() -> Option<Db> {
    let url = std::env::var("APP_DATABASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| APP_URL.to_string())
        .replace("localhost", "127.0.0.1");
    let conn = Database::connect(&url).await.ok()?;
    Some(Db::from_connection(conn, url))
}

async fn owner_pool() -> Option<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| OWNER_URL.to_string())
        .replace("localhost", "127.0.0.1");
    PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(500))
        .connect(&url)
        .await
        .ok()
}


#[tokio::test]
async fn a_pharmacy_token_cannot_reach_an_admin_route() {
    // `admin_router` now carries its OpenAPI paths; the routes are the same.
    let app = Router::from(admin_router());

    let tenant_id = TenantId::new_v7();
    let pharmacy_user_id = UserId::new_v7();

    // A pharmacy token carries pharmacy permissions, never platform token
    let pharmacy_auth = AuthUser {
        user_id: pharmacy_user_id,
        tenant_id,
        permissions: vec![
            "tenant.view".to_string(),
            "tenant.update".to_string(),
            "branch.view".to_string(),
            "subscription.pay".to_string(),
        ],
        branch_permissions: std::collections::BTreeMap::new(),
    };

    let target_uuid = Uuid::now_v7();

    // 1. POST /tenants/{id}/approve with pharmacy token -> 403 Forbidden
    let mut req_approve = Request::builder()
        .uri(format!("/tenants/{target_uuid}/approve"))
        .method("POST")
        .body(Body::empty())
        .unwrap();
    req_approve.extensions_mut().insert(pharmacy_auth.clone());

    let resp_approve = app.clone().oneshot(req_approve).await.unwrap();
    assert_eq!(
        resp_approve.status(),
        StatusCode::FORBIDDEN,
        "Pharmacy token must be rejected from admin approve route"
    );

    // 2. POST /tenants/{id}/suspend with pharmacy token -> 403 Forbidden
    let mut req_suspend = Request::builder()
        .uri(format!("/tenants/{target_uuid}/suspend"))
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"reason":"breach"}"#))
        .unwrap();
    req_suspend.extensions_mut().insert(pharmacy_auth.clone());

    let resp_suspend = app.clone().oneshot(req_suspend).await.unwrap();
    assert_eq!(
        resp_suspend.status(),
        StatusCode::FORBIDDEN,
        "Pharmacy token must be rejected from admin suspend route"
    );

    // 3. GET /subscription-payments with pharmacy token -> 403 Forbidden
    let mut req_payments = Request::builder()
        .uri("/subscription-payments")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    req_payments.extensions_mut().insert(pharmacy_auth);

    let resp_payments = app.clone().oneshot(req_payments).await.unwrap();
    assert_eq!(
        resp_payments.status(),
        StatusCode::FORBIDDEN,
        "Pharmacy token must be rejected from admin payments queue route"
    );

    // 4. Contrast: PlatformToken passes the admin gate
    let mut req_platform = Request::builder()
        .uri("/subscription-payments")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    req_platform
        .extensions_mut()
        .insert(PlatformToken { user_id: UserId::new_v7() });

    let resp_platform = app.oneshot(req_platform).await.unwrap();
    assert_ne!(
        resp_platform.status(),
        StatusCode::FORBIDDEN,
        "Platform token must not be rejected by require_platform gate"
    );
}

#[test]
fn money_is_a_string_on_the_wire() {
    // 1. TenantSettings: vat_rate
    let settings = TenantSettings {
        vat_rate: Decimal::from_str("0.0500").unwrap(),
        vat_included: true,
        rounding_mode: "nearest".to_string(),
        near_expiry_days: 90,
        default_payment_terms_days: 30,
        allow_zero_stock_sale: false,
        receipt_header: None,
        receipt_footer: None,
        receipt_paper_width: 80,
    };
    let settings_json = serde_json::to_string(&settings).unwrap();
    assert!(
        settings_json.contains(r#""vat_rate":"0.0500""#),
        "vat_rate must serialize as a string on the wire, got: {settings_json}"
    );
    assert!(
        !settings_json.contains(r#""vat_rate":0.05"#),
        "vat_rate must never serialize as a bare number"
    );

    // 2. Subscription: price
    let sub = Subscription {
        id: Uuid::now_v7(),
        tenant_id: TenantId::new_v7(),
        plan_code: "standard".to_string(),
        billing_period: "monthly".to_string(),
        price: Decimal::from_str("1500.00").unwrap(),
        starts_at: Utc::now(),
        ends_at: Utc::now(),
        status: "active".to_string(),
        is_complimentary: false,
    };
    let sub_json = serde_json::to_string(&sub).unwrap();
    assert!(
        sub_json.contains(r#""price":"1500.00""#),
        "Subscription price must serialize as a string on the wire, got: {sub_json}"
    );
    assert!(
        !sub_json.contains(r#""price":1500"#),
        "Subscription price must never serialize as a bare number"
    );

    // 3. Payment: amount
    let payment = Payment {
        id: Uuid::now_v7(),
        subscription_id: Some(Uuid::now_v7()),
        amount: Decimal::from_str("1500.00").unwrap(),
        method: "bank_transfer".to_string(),
        reference: "TXN123".to_string(),
        status: "submitted".to_string(),
        paid_at: Utc::now(),
    };
    let payment_json = serde_json::to_string(&payment).unwrap();
    assert!(
        payment_json.contains(r#""amount":"1500.00""#),
        "Payment amount must serialize as a string on the wire, got: {payment_json}"
    );
    assert!(
        !payment_json.contains(r#""amount":1500"#),
        "Payment amount must never serialize as a bare number"
    );

    // 4. PlanPrice: price
    let plan_price = PlanPrice {
        billing_period: "yearly".to_string(),
        price: Decimal::from_str("15000.00").unwrap(),
    };
    let price_json = serde_json::to_string(&plan_price).unwrap();
    assert!(
        price_json.contains(r#""price":"15000.00""#),
        "PlanPrice must serialize as a string on the wire, got: {price_json}"
    );
}

#[tokio::test]
async fn tenant_response_carries_the_access_summary() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_id = TenantId::new_v7();
    let platform_user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, platform_user_id, vec![]);

    // 1. Create pharmacy and approve it
    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin platform tx");
    create_pharmacy(&tx, "Access Summary Pharmacy", None, None, None, None)
        .await
        .expect("create pharmacy");
    tx.commit().await.expect("commit create");

    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin platform tx");
    approve(&tx, tenant_id, platform_user_id)
        .await
        .expect("approve pharmacy");
    tx.commit().await.expect("commit approve");

    // 2. Add active subscription
    let sub_id = Uuid::now_v7();
    let now = Utc::now();
    let plan_id = Uuid::parse_str("0192a000-0000-7000-8000-000000000001").unwrap();
    sqlx::query(
        "INSERT INTO subscriptions (id, tenant_id, plan_id, billing_period, price, starts_at, ends_at, status)
         VALUES ($1, $2, $3, 'monthly', 500.00, $4, $5, 'active')"
    )
    .bind(sub_id)
    .bind(tenant_id.as_uuid())
    .bind(plan_id)
    .bind(now - chrono::Duration::days(2))
    .bind(now + chrono::Duration::days(28))
    .execute(&owner)
    .await
    .expect("insert active subscription");

    // Query tenant: response must carry access summary with mode "full"
    let tenant_full = queries::tenant(&db, tenant_id).await.expect("query tenant");
    assert_eq!(tenant_full.access.mode, "full");
    let full_json = serde_json::to_string(&tenant_full).unwrap();
    assert!(
        full_json.contains(r#""access":{"mode":"full"}"#),
        "Tenant JSON must carry access summary mode full, got: {full_json}"
    );

    // 3. Suspend pharmacy: response must carry access summary with mode "suspended"
    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin platform tx");
    suspend(&tx, tenant_id, "Access check suspension".to_string())
        .await
        .expect("suspend pharmacy");
    tx.commit().await.expect("commit suspend");

    let tenant_suspended = queries::tenant(&db, tenant_id).await.expect("query tenant");
    assert_eq!(tenant_suspended.access.mode, "suspended");
    let susp_json = serde_json::to_string(&tenant_suspended).unwrap();
    assert!(
        susp_json.contains(r#""access":{"mode":"suspended"}"#),
        "Tenant JSON must carry access summary mode suspended, got: {susp_json}"
    );

    // Clean up
    let _ = sqlx::query("DELETE FROM subscriptions WHERE tenant_id = $1").bind(tenant_id.as_uuid()).execute(&owner).await;
    let _ = sqlx::query("DELETE FROM tenant_settings WHERE tenant_id = $1").bind(tenant_id.as_uuid()).execute(&owner).await;
    let _ = sqlx::query("DELETE FROM branches WHERE tenant_id = $1").bind(tenant_id.as_uuid()).execute(&owner).await;
    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1").bind(tenant_id.as_uuid()).execute(&owner).await;
}
