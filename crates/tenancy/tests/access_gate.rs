use api::{access_gate_middleware, AuthUser};
use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware,
    routing::{get, post},
    Router,
};
use cache::Cache;
use chrono::Utc;
use db::{Db, Tx};
use pos_core::context::TenantContext;
use pos_core::error::ErrorCode;
use pos_core::ids::{TenantId, UserId};
use pos_core::ports::AccessModeSource;
use sea_orm::Database;
use sqlx::postgres::PgPoolOptions;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tenancy::commands::create_pharmacy;
use tenancy::platform::{approve, suspend};
use tenancy::{invalidate_access_mode, AccessMode, TenancyAccessModeSource};
use tower::ServiceExt;
use uuid::Uuid;

const APP_URL: &str = "postgres://pos_app:devpass@127.0.0.1:5432/medicine_pos";
const OWNER_URL: &str = "postgres://pos_owner:devpass@127.0.0.1:5432/medicine_pos";
const REDIS_URL: &str = "redis://127.0.0.1:6379";

async fn get_db() -> Option<Db> {
    let url = std::env::var("APP_DATABASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| APP_URL.to_string());
    let conn = Database::connect(&url).await.ok()?;
    Some(Db::from_connection(conn, url))
}

async fn owner_pool() -> Option<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| OWNER_URL.to_string());
    PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(500))
        .connect(&url)
        .await
        .ok()
}

#[tokio::test]
async fn read_only_rejects_a_write_and_allows_a_read() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_id = TenantId::new_v7();
    let platform_user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, platform_user_id, vec![]);

    // 1. Create pharmacy and approve it so status = active
    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin platform tx");
    create_pharmacy(&tx, "Read Only Test Pharmacy", None, None, None, None)
        .await
        .expect("create pharmacy");
    tx.commit().await.expect("commit create");
    println!("DEBUG: pharmacy created");

    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin platform tx");
    approve(&tx, tenant_id, platform_user_id)
        .await
        .expect("approve pharmacy");
    tx.commit().await.expect("commit approve");

    // 2. Insert expired subscription beyond grace period (ended 30 days ago, grace is 7 days)
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
    .bind(now - chrono::Duration::days(60))
    .bind(now - chrono::Duration::days(30))
    .execute(&owner)
    .await
    .expect("insert expired subscription");

    let cache = Cache::in_memory("test_read_only");
    let source: Arc<dyn AccessModeSource> = Arc::new(TenancyAccessModeSource::new(db.clone(), cache));

    // Direct AccessModeSource check:
    // Read succeeds
    let read_check = source.check_access(tenant_id, false).await;
    assert!(read_check.is_ok(), "A read in read-only mode must succeed");

    // Write fails with 403 subscription_read_only
    let write_check = source.check_access(tenant_id, true).await;
    let err = write_check.expect_err("A write in read-only mode must be rejected");
    assert_eq!(err.code, ErrorCode::SubscriptionReadOnly);
    assert_eq!(err.code.http_status(), 403);
    assert_eq!(err.code.as_str(), "subscription_read_only");

    // HTTP layer assertion:
    let app = Router::new()
        .route("/items", get(|| async { (StatusCode::OK, "read ok") }))
        .route("/items", post(|| async { (StatusCode::CREATED, "write ok") }))
        .layer(middleware::from_fn(access_gate_middleware));

    // GET request (read) succeeds
    let mut get_req = Request::builder()
        .uri("/items")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    get_req.extensions_mut().insert(ctx.clone());
    get_req.extensions_mut().insert(source.clone());

    let get_resp = app.clone().oneshot(get_req).await.unwrap();
    assert_eq!(get_resp.status(), StatusCode::OK);

    // POST request (write) rejected with 403 subscription_read_only
    let mut post_req = Request::builder()
        .uri("/items")
        .method("POST")
        .body(Body::empty())
        .unwrap();
    post_req.extensions_mut().insert(ctx.clone());
    post_req.extensions_mut().insert(source.clone());

    let post_resp = app.oneshot(post_req).await.unwrap();
    assert_eq!(post_resp.status(), StatusCode::FORBIDDEN);

    // Clean up
    let _ = sqlx::query("DELETE FROM subscriptions WHERE tenant_id = $1").bind(tenant_id.as_uuid()).execute(&owner).await;
    let _ = sqlx::query("DELETE FROM tenant_settings WHERE tenant_id = $1").bind(tenant_id.as_uuid()).execute(&owner).await;
    let _ = sqlx::query("DELETE FROM branches WHERE tenant_id = $1").bind(tenant_id.as_uuid()).execute(&owner).await;
    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1").bind(tenant_id.as_uuid()).execute(&owner).await;
}

#[tokio::test]
async fn blocked_is_refused_before_the_handler() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_a = TenantId::new_v7();
    let tenant_b = TenantId::new_v7();
    let platform_user_id = UserId::new_v7();

    let ctx_a = TenantContext::new(tenant_a, platform_user_id, vec![]);
    let ctx_b = TenantContext::new(tenant_b, platform_user_id, vec![]);

    // 1. Tenant A: pending_approval
    let tx = Tx::begin_platform(db.conn(), &ctx_a).await.expect("begin platform tx");
    create_pharmacy(&tx, "Pending Approval Pharmacy", None, None, None, None)
        .await
        .expect("create pharmacy");
    tx.commit().await.expect("commit create");

    // 2. Tenant B: suspended
    let tx = Tx::begin_platform(db.conn(), &ctx_b).await.expect("begin platform tx");
    create_pharmacy(&tx, "Suspended Pharmacy", None, None, None, None)
        .await
        .expect("create pharmacy");
    tx.commit().await.expect("commit create");

    let tx = Tx::begin_platform(db.conn(), &ctx_b).await.expect("begin platform tx");
    approve(&tx, tenant_b, platform_user_id)
        .await
        .expect("approve pharmacy");
    tx.commit().await.expect("commit approve");

    let tx = Tx::begin_platform(db.conn(), &ctx_b).await.expect("begin platform tx");
    suspend(&tx, tenant_b, "Policy violation".to_string())
        .await
        .expect("suspend pharmacy");
    tx.commit().await.expect("commit suspend");

    let cache = Cache::in_memory("test_blocked");
    let source: Arc<dyn AccessModeSource> = Arc::new(TenancyAccessModeSource::new(db.clone(), cache));

    // Direct check assertions:
    let err_pending_read = source.check_access(tenant_a, false).await.expect_err("pending blocked read");
    assert_eq!(err_pending_read.code, ErrorCode::TenantPendingApproval);
    assert_eq!(err_pending_read.code.http_status(), 403);

    let err_pending_write = source.check_access(tenant_a, true).await.expect_err("pending blocked write");
    assert_eq!(err_pending_write.code, ErrorCode::TenantPendingApproval);

    let err_susp_read = source.check_access(tenant_b, false).await.expect_err("suspended blocked read");
    assert_eq!(err_susp_read.code, ErrorCode::TenantSuspended);
    assert_eq!(err_susp_read.code.http_status(), 403);

    let err_susp_write = source.check_access(tenant_b, true).await.expect_err("suspended blocked write");
    assert_eq!(err_susp_write.code, ErrorCode::TenantSuspended);

    // Axum middleware check proving refusal before handler:
    let handler_executed = Arc::new(AtomicUsize::new(0));
    let handler_flag = handler_executed.clone();

    let app = Router::new()
        .route("/guarded", get({
            let flag = handler_flag.clone();
            move || {
                let f = flag.clone();
                async move {
                    f.fetch_add(1, Ordering::SeqCst);
                    (StatusCode::OK, "handler reached")
                }
            }
        }))
        .route("/guarded", post({
            let flag = handler_flag.clone();
            move || {
                let f = flag.clone();
                async move {
                    f.fetch_add(1, Ordering::SeqCst);
                    (StatusCode::CREATED, "handler reached")
                }
            }
        }))
        .layer(middleware::from_fn(access_gate_middleware));

    // Request for Tenant A (pending approval)
    let mut req_a = Request::builder().uri("/guarded").method("GET").body(Body::empty()).unwrap();
    req_a.extensions_mut().insert(ctx_a);
    req_a.extensions_mut().insert(source.clone());

    let resp_a = app.clone().oneshot(req_a).await.unwrap();
    assert_eq!(resp_a.status(), StatusCode::FORBIDDEN);
    assert_eq!(handler_executed.load(Ordering::SeqCst), 0, "Handler must NOT have run for pending pharmacy");

    // Request for Tenant B (suspended)
    let mut req_b = Request::builder().uri("/guarded").method("POST").body(Body::empty()).unwrap();
    req_b.extensions_mut().insert(ctx_b);
    req_b.extensions_mut().insert(source.clone());

    let resp_b = app.oneshot(req_b).await.unwrap();
    assert_eq!(resp_b.status(), StatusCode::FORBIDDEN);
    assert_eq!(handler_executed.load(Ordering::SeqCst), 0, "Handler must NOT have run for suspended pharmacy");

    // Clean up
    for tid in [tenant_a, tenant_b] {
        let _ = sqlx::query("DELETE FROM subscriptions WHERE tenant_id = $1").bind(tid.as_uuid()).execute(&owner).await;
        let _ = sqlx::query("DELETE FROM tenant_settings WHERE tenant_id = $1").bind(tid.as_uuid()).execute(&owner).await;
        let _ = sqlx::query("DELETE FROM branches WHERE tenant_id = $1").bind(tid.as_uuid()).execute(&owner).await;
        let _ = sqlx::query("DELETE FROM tenants WHERE id = $1").bind(tid.as_uuid()).execute(&owner).await;
    }
}

#[tokio::test]
async fn suspension_invalidates_the_cached_access_mode() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_id = TenantId::new_v7();
    let platform_user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, platform_user_id, vec![]);

    // 1. Create and approve pharmacy
    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin platform tx");
    create_pharmacy(&tx, "Cache Invalidation Pharmacy", None, None, None, None)
        .await
        .expect("create pharmacy");
    tx.commit().await.expect("commit create");

    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin platform tx");
    approve(&tx, tenant_id, platform_user_id)
        .await
        .expect("approve pharmacy");
    tx.commit().await.expect("commit approve");

    // 2. Insert active subscription covering now
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
    .bind(now - chrono::Duration::days(5))
    .bind(now + chrono::Duration::days(25))
    .execute(&owner)
    .await
    .expect("insert active subscription");

    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| REDIS_URL.to_string());
    let cache = Cache::new(&redis_url, "test_cache_invalidation").expect("connect redis cache");
    let source = TenancyAccessModeSource::new(db.clone(), cache.clone());

    // 3. Populate cache
    let access_mode = source.resolve_access_mode(tenant_id).await.expect("resolve access mode");
    assert_eq!(access_mode, AccessMode::Full);

    // Verify Redis has the key
    let cache_key = format!("tenant:{}:access", tenant_id.as_uuid());
    let cached_val = cache.get(&cache_key).await.expect("get from redis");
    assert!(cached_val.is_some(), "Access mode must be cached in Redis");

    // 4. Suspend pharmacy and invalidate cache
    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin platform tx");
    suspend(&tx, tenant_id, "Terms violation".to_string())
        .await
        .expect("suspend pharmacy");
    tx.commit().await.expect("commit suspend");

    // Explicit invalidation
    invalidate_access_mode(&cache, tenant_id).await.expect("invalidate cache");

    // 5. Verify the cached value is removed from Redis immediately
    let after_inv = cache.get(&cache_key).await.expect("get from redis");
    assert!(after_inv.is_none(), "Cached key must be deleted upon suspension");

    // 6. Next call must immediately observe suspended without waiting for TTL
    let check = source.check_access(tenant_id, false).await;
    let err = check.expect_err("Access must be rejected after suspension");
    assert_eq!(err.code, ErrorCode::TenantSuspended);

    // Clean up
    let _ = invalidate_access_mode(&cache, tenant_id).await;
    let _ = sqlx::query("DELETE FROM subscriptions WHERE tenant_id = $1").bind(tenant_id.as_uuid()).execute(&owner).await;
    let _ = sqlx::query("DELETE FROM tenant_settings WHERE tenant_id = $1").bind(tenant_id.as_uuid()).execute(&owner).await;
    let _ = sqlx::query("DELETE FROM branches WHERE tenant_id = $1").bind(tenant_id.as_uuid()).execute(&owner).await;
    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1").bind(tenant_id.as_uuid()).execute(&owner).await;
}

#[tokio::test]
async fn a_cache_outage_never_grants_full_access() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_id = TenantId::new_v7();
    let platform_user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, platform_user_id, vec![]);

    // 1. Create pharmacy and suspend it
    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin platform tx");
    create_pharmacy(&tx, "Outage Test Pharmacy", None, None, None, None)
        .await
        .expect("create pharmacy");
    tx.commit().await.expect("commit create");

    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin platform tx");
    approve(&tx, tenant_id, platform_user_id)
        .await
        .expect("approve pharmacy");
    tx.commit().await.expect("commit approve");

    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin platform tx");
    suspend(&tx, tenant_id, "Fraudulent activity".to_string())
        .await
        .expect("suspend pharmacy");
    tx.commit().await.expect("commit suspend");

    // 2. Configure cache with unreachable Redis port (cache outage)
    let broken_cache = Cache::new("redis://127.0.0.1:9999", "outage").expect("broken cache pool created");
    let source = TenancyAccessModeSource::new(db.clone(), broken_cache);

    // 3. Fallback to database must reflect suspended status and NOT grant full access
    let access_res = source.check_access(tenant_id, false).await;
    let err = access_res.expect_err("Cache outage must fall back to DB and reject suspended tenant");
    assert_eq!(err.code, ErrorCode::TenantSuspended);
    assert_ne!(err.code.http_status(), 200);

    // 4. When database lookup also fails (e.g. unknown tenant), never grant full access
    let non_existent_tenant = TenantId::new_v7();
    let fail_res = source.check_access(non_existent_tenant, false).await;
    assert!(fail_res.is_err(), "Cache outage + DB failure must never grant full access");

    // Clean up
    let _ = sqlx::query("DELETE FROM tenant_settings WHERE tenant_id = $1").bind(tenant_id.as_uuid()).execute(&owner).await;
    let _ = sqlx::query("DELETE FROM branches WHERE tenant_id = $1").bind(tenant_id.as_uuid()).execute(&owner).await;
    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1").bind(tenant_id.as_uuid()).execute(&owner).await;
}

#[tokio::test]
async fn a_read_only_tenant_can_still_submit_a_payment() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_id = TenantId::new_v7();
    let platform_user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, platform_user_id, vec![]);

    // 1. Create pharmacy and approve it so status = active
    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin platform tx");
    create_pharmacy(&tx, "Read Only Payment Pharmacy", None, None, None, None)
        .await
        .expect("create pharmacy");
    tx.commit().await.expect("commit create");

    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin platform tx");
    approve(&tx, tenant_id, platform_user_id)
        .await
        .expect("approve pharmacy");
    tx.commit().await.expect("commit approve");

    // 2. Insert expired subscription beyond grace period (ended 30 days ago, grace is 7 days)
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
    .bind(now - chrono::Duration::days(60))
    .bind(now - chrono::Duration::days(30))
    .execute(&owner)
    .await
    .expect("insert expired subscription");

    let cache = Cache::in_memory("test_read_only_payment");
    let source: Arc<dyn AccessModeSource> = Arc::new(TenancyAccessModeSource::new(db.clone(), cache));

    let app = Router::from(tenancy::rest::pharmacy_router())
        .layer(axum::Extension(db.clone()))
        .layer(axum::Extension(source.clone()));

    // Submit payment via POST /subscription-payments
    let payment_body = serde_json::json!({
        "subscription_id": sub_id,
        "amount": "500.00",
        "method": "bank_transfer",
        "reference": format!("REF-{}", Uuid::now_v7()),
        "proof_file_key": null
    });

    let auth = AuthUser {
        user_id: platform_user_id,
        tenant_id,
        permissions: vec!["subscription.pay".to_string(), "branch.create".to_string()],
        branch_permissions: Default::default(),
    };

    let mut post_req = Request::builder()
        .uri("/subscription-payments")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(payment_body.to_string()))
        .unwrap();
    post_req.extensions_mut().insert(ctx.clone());
    post_req.extensions_mut().insert(source.clone());
    post_req.extensions_mut().insert(db.clone());
    post_req.extensions_mut().insert(auth.clone());

    let post_resp = app.clone().oneshot(post_req).await.unwrap();
    assert_eq!(
        post_resp.status(),
        StatusCode::CREATED,
        "A read-only tenant must be allowed to submit a renewal payment"
    );

    // In contrast, a gated write route (e.g. creating a branch) must still be rejected with 403
    let branch_body = serde_json::json!({
        "name": "New Gated Branch"
    });
    let mut branch_req = Request::builder()
        .uri("/branches")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(branch_body.to_string()))
        .unwrap();
    branch_req.extensions_mut().insert(ctx.clone());
    branch_req.extensions_mut().insert(source.clone());
    branch_req.extensions_mut().insert(db.clone());
    branch_req.extensions_mut().insert(auth);

    let branch_resp = app.oneshot(branch_req).await.unwrap();
    assert_eq!(
        branch_resp.status(),
        StatusCode::FORBIDDEN,
        "Gated write route must still be rejected with 403 for read-only tenant"
    );

    // Clean up
    let _ = sqlx::query("DELETE FROM subscription_payments WHERE tenant_id = $1").bind(tenant_id.as_uuid()).execute(&owner).await;
    let _ = sqlx::query("DELETE FROM subscriptions WHERE tenant_id = $1").bind(tenant_id.as_uuid()).execute(&owner).await;
    let _ = sqlx::query("DELETE FROM tenant_settings WHERE tenant_id = $1").bind(tenant_id.as_uuid()).execute(&owner).await;
    let _ = sqlx::query("DELETE FROM branches WHERE tenant_id = $1").bind(tenant_id.as_uuid()).execute(&owner).await;
    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1").bind(tenant_id.as_uuid()).execute(&owner).await;
}

#[tokio::test]
async fn a_suspension_survives_an_unreachable_cache() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_id = TenantId::new_v7();
    let platform_user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, platform_user_id, vec![]);

    // 1. Create pharmacy and approve it
    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin platform tx");
    create_pharmacy(&tx, "Suspension Outage Pharmacy", None, None, None, None)
        .await
        .expect("create pharmacy");
    tx.commit().await.expect("commit create");

    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin platform tx");
    approve(&tx, tenant_id, platform_user_id)
        .await
        .expect("approve pharmacy");
    tx.commit().await.expect("commit approve");

    // 2. Suspend pharmacy in DB
    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin platform tx");
    suspend(&tx, tenant_id, "Security breach".to_string())
        .await
        .expect("suspend pharmacy");
    tx.commit().await.expect("commit suspend");

    // 3. Configure cache with unreachable Redis port
    let broken_cache = Cache::new("redis://127.0.0.1:9999", "unreachable").expect("broken cache pool created");
    let source = TenancyAccessModeSource::new(db.clone(), broken_cache);

    // 4. Verify access is rejected with TenantSuspended
    let access_res = source.check_access(tenant_id, false).await;
    let err = access_res.expect_err("Suspended pharmacy must be blocked even when cache is unreachable");
    assert_eq!(err.code, ErrorCode::TenantSuspended);

    // Clean up
    let _ = sqlx::query("DELETE FROM tenant_settings WHERE tenant_id = $1").bind(tenant_id.as_uuid()).execute(&owner).await;
    let _ = sqlx::query("DELETE FROM branches WHERE tenant_id = $1").bind(tenant_id.as_uuid()).execute(&owner).await;
    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1").bind(tenant_id.as_uuid()).execute(&owner).await;
}

