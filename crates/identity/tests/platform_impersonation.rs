#![allow(clippy::expect_used, clippy::unwrap_used)]

use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware,
    response::IntoResponse,
    routing::get,
    Router,
};
use db::Tx;
use identity::password::hash_password;
use identity::platform::{
    enable_impersonation_writes, execute_impersonated_write, get_impersonation_session,
    list_impersonation_sessions_for_tenant, open_impersonation_session, platform_login_step_one,
    platform_login_step_two,
};
use identity::tokens::{decode_pharmacy_token, decode_platform_token, encode_pharmacy_token, encode_platform_token};
use identity::types::{
    PlatformLoginStepOneRequest, PlatformLoginStepTwoRequest,
};
use pos_core::context::TenantContext;
use pos_core::ids::{TenantId, UserId};
use sea_orm::{ConnectionTrait, Database, DatabaseConnection};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

const APP_URL: &str = "postgres://pos_app:devpass@127.0.0.1:5432/medicine_pos";
const OWNER_URL: &str = "postgres://pos_owner:devpass@127.0.0.1:5432/medicine_pos";
const JWT_SECRET: &str = "test_jwt_secret_key_at_least_32_bytes_long!!";

/// The lifetimes these tests assert about, stated once.
fn settings() -> identity::TokenSettings {
    identity::TokenSettings::for_test(JWT_SECRET)
}
// 32 base32 chars = 160 bits. The old 16-char secret was 80 bits, which `totp_rs`
// rejects as `SecretSize(80)` — RFC 4226 wants at least 128.
const TEST_TOTP_SECRET: &str = "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP";

async fn get_app_db() -> Option<DatabaseConnection> {
    Database::connect(APP_URL).await.ok()
}

async fn get_owner_db() -> Option<DatabaseConnection> {
    Database::connect(OWNER_URL).await.ok()
}

async fn owner_pool() -> Option<PgPool> {
    PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(2))
        .connect(OWNER_URL)
        .await
        .ok()
}

async fn seed_tenant(pool: &PgPool, tenant_id: TenantId, name: &str) {
    let _ = sqlx::query(
        "INSERT INTO tenants (id, name, status, created_at, updated_at)
         VALUES ($1, $2, 'active', now(), now())
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(tenant_id.as_uuid())
    .bind(name)
    .execute(pool)
    .await;
}

async fn seed_platform_user(
    pool: &PgPool,
    user_id: Uuid,
    email: &str,
    password: &str,
    totp_secret: &str,
    role: &str,
) {
    let pwd_hash = hash_password(password).unwrap_or_default();
    // `platform_users` has no `name` column — `[D-80]` gives it email, role, hash and TOTP.
    // This insert is asserted, not swallowed: a silent failure here leaves every test that
    // needs a platform user failing on a foreign key, which is how it went unnoticed.
    sqlx::query(
        "INSERT INTO platform_users (id, email, role, password_hash, totp_secret_encrypted, status, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, 'active', now(), now())
         ON CONFLICT (email) DO UPDATE SET password_hash = EXCLUDED.password_hash, totp_secret_encrypted = EXCLUDED.totp_secret_encrypted",
    )
    .bind(user_id)
    .bind(email.to_lowercase())
    .bind(role)
    .bind(pwd_hash)
    .bind(totp_secret)
    .execute(pool)
    .await
    .expect("seed platform user");
}

#[tokio::test]
async fn platform_login_step_one_issues_no_access_token() {
    let owner_db = match get_owner_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };

    let user_id = Uuid::now_v7();
    let email = format!("plat_s1_{}@test.com", Uuid::now_v7());
    seed_platform_user(&owner, user_id, &email, "Pass123!", TEST_TOTP_SECRET, "super_admin").await;

    // Step 1: Login with email & password
    let step1_res = platform_login_step_one(
        &owner_db,
        PlatformLoginStepOneRequest {
            email: email.clone(),
            password: "Pass123!".to_string(),
        },
        JWT_SECRET,
    )
    .await
    .expect("step 1 login");

    assert!(step1_res.requires_totp, "Step 1 must require TOTP");
    assert!(!step1_res.temp_token.is_empty(), "Step 1 issues a temp token");

    // Verify temp_token CANNOT be decoded as an access token
    let dec_res = decode_platform_token(&step1_res.temp_token, JWT_SECRET);
    assert!(dec_res.is_err(), "Step 1 temp token is not an access token");

    // Complete Step 2 with valid TOTP code
    let secret_bytes = totp_rs::Secret::Encoded(TEST_TOTP_SECRET.to_string()).to_bytes().unwrap();
    let totp = totp_rs::TOTP::new(totp_rs::Algorithm::SHA1, 6, 1, 30, secret_bytes).unwrap();
    let code = totp.generate_current().unwrap();

    let step2_res = platform_login_step_two(
        &owner_db,
        PlatformLoginStepTwoRequest {
            temp_token: step1_res.temp_token,
            totp_code: code,
        },
        &settings(),
    )
    .await
    .expect("step 2 login");

    assert!(!step2_res.access_token.is_empty(), "Step 2 issues real access token");
    let access_claims = decode_platform_token(&step2_res.access_token, JWT_SECRET).expect("valid access token");
    assert_eq!(access_claims.sub, user_id.to_string());
}

#[tokio::test]
async fn a_platform_user_without_totp_cannot_sign_in() {
    let owner_db = match get_owner_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };

    let user_id = Uuid::now_v7();
    let email = format!("plat_no_totp_{}@test.com", Uuid::now_v7());
    // Seed platform user with empty TOTP secret
    seed_platform_user(&owner, user_id, &email, "Pass123!", "", "support").await;

    let res = platform_login_step_one(
        &owner_db,
        PlatformLoginStepOneRequest {
            email,
            password: "Pass123!".to_string(),
        },
        JWT_SECRET,
    )
    .await;

    assert!(res.is_err(), "Platform user without TOTP must be refused");
    let err = res.err().unwrap();
    assert_eq!(err.code.http_status(), 403);
}

#[tokio::test]
async fn a_pharmacy_token_is_rejected_by_a_platform_route() {
    // Platform route middleware validates audience pos:platform
    async fn platform_auth_middleware(
        mut req: Request<Body>,
        next: axum::middleware::Next,
    ) -> axum::response::Response {
        let auth_header = req
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok());
        let token = match auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
            Some(t) => t.trim(),
            None => return StatusCode::UNAUTHORIZED.into_response(),
        };

        match decode_platform_token(token, JWT_SECRET) {
            Ok(claims) => {
                req.extensions_mut().insert(claims);
                next.run(req).await
            }
            Err(_) => StatusCode::UNAUTHORIZED.into_response(),
        }
    }

    let app = Router::new()
        .route("/api/v1/platform/test", get(|| async { StatusCode::OK }))
        .layer(middleware::from_fn(platform_auth_middleware));

    // Create pharmacy token with audience pos:pharmacy
    let pharmacy_token = encode_pharmacy_token(UserId::new_v7(), TenantId::new_v7(), JWT_SECRET, 900)
        .expect("encode pharmacy token");

    let req = Request::builder()
        .uri("/api/v1/platform/test")
        .header("Authorization", format!("Bearer {pharmacy_token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_platform_token_is_rejected_by_a_pharmacy_route() {
    // Pharmacy route middleware validates audience pos:pharmacy
    async fn pharmacy_auth_middleware(
        mut req: Request<Body>,
        next: axum::middleware::Next,
    ) -> axum::response::Response {
        let auth_header = req
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok());
        let token = match auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
            Some(t) => t.trim(),
            None => return StatusCode::UNAUTHORIZED.into_response(),
        };

        match decode_pharmacy_token(token, JWT_SECRET) {
            Ok(claims) => {
                req.extensions_mut().insert(claims);
                next.run(req).await
            }
            Err(_) => StatusCode::UNAUTHORIZED.into_response(),
        }
    }

    let app = Router::new()
        .route("/api/v1/pharmacy/test", get(|| async { StatusCode::OK }))
        .layer(middleware::from_fn(pharmacy_auth_middleware));

    // Create platform token with audience pos:platform
    let platform_token = encode_platform_token(Uuid::now_v7(), "reviewer", JWT_SECRET, 900)
        .expect("encode platform token");

    let req = Request::builder()
        .uri("/api/v1/pharmacy/test")
        .header("Authorization", format!("Bearer {platform_token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn platform_users_is_unreadable_with_a_tenant_context() {
    let app_db = match get_app_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    seed_tenant(&owner, tenant_id, "Tenant Context Test").await;

    let ctx = TenantContext::new(tenant_id, user_id, vec![]);
    let tx = Tx::begin(&app_db, &ctx).await.expect("begin tenant tx");
    let conn = tx.transaction().expect("get tx conn");

    // Attempt to query platform_users table from tenant context connection
    let stmt = sea_orm::Statement::from_sql_and_values(
        sea_orm::DbBackend::Postgres,
        "SELECT count(*) FROM platform_users",
        [],
    );
    let res = conn.query_one_raw(stmt).await;

    // Must fail due to REVOKE ALL ON platform_users FROM pos_app
    assert!(res.is_err(), "Tenant context connection must not have permission to read platform_users");
}

#[tokio::test]
async fn an_impersonation_session_is_read_only_by_default() {
    let owner_db = match get_owner_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };

    let platform_user_id = Uuid::now_v7();
    let tenant_id = TenantId::new_v7();
    seed_platform_user(&owner, platform_user_id, &format!("imp_{}@test.com", Uuid::now_v7()), "Pass123!", TEST_TOTP_SECRET, "support").await;
    seed_tenant(&owner, tenant_id, "Impersonation Target").await;

    // 1. Stated reason is required
    let empty_reason_res = open_impersonation_session(&owner_db, platform_user_id, tenant_id, "   ", 3600).await;
    assert!(empty_reason_res.is_err(), "Empty reason must be rejected");

    // 2. Open session with stated reason
    let session_id = open_impersonation_session(
        &owner_db,
        platform_user_id,
        tenant_id,
        "Customer reported prescription sync failure",
        3600,
    )
    .await
    .expect("open impersonation");

    // 3. Inspect session -> writes_enabled_at is NULL (read-only by default)
    let session_item = get_impersonation_session(&owner_db, session_id)
        .await
        .expect("get session")
        .expect("session exists");
    assert!(session_item.writes_enabled_at.is_none(), "Must be read-only by default");

    // 4. Attempting a write during read-only impersonation fails
    let write_res = execute_impersonated_write(&owner_db, session_id, platform_user_id, |_| Ok(())).await;
    assert!(write_res.is_err(), "Write must fail while impersonation is read-only");
    assert_eq!(write_res.err().unwrap().code.http_status(), 403);

    // 5. Explicitly enable writes
    enable_impersonation_writes(&owner_db, session_id, platform_user_id)
        .await
        .expect("enable writes");

    // 6. Write now succeeds
    let write_res2 = execute_impersonated_write(&owner_db, session_id, platform_user_id, |_| Ok("write_done")).await;
    assert!(write_res2.is_ok(), "Write must succeed when writes are enabled");
    assert_eq!(write_res2.unwrap().0, "write_done");
}

#[tokio::test]
async fn an_owner_reads_impersonation_of_their_own_pharmacy() {
    let app_db = match get_app_db().await {
        Some(d) => d,
        None => return,
    };
    let owner_db = match get_owner_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };

    let platform_user_id = Uuid::now_v7();
    let tenant_a = TenantId::new_v7();
    let tenant_b = TenantId::new_v7();
    let owner_a = UserId::new_v7();

    seed_platform_user(&owner, platform_user_id, &format!("imp_own_{}@test.com", Uuid::now_v7()), "Pass123!", TEST_TOTP_SECRET, "reviewer").await;
    seed_tenant(&owner, tenant_a, "Pharmacy Alpha").await;
    seed_tenant(&owner, tenant_b, "Pharmacy Beta").await;

    // Platform opens sessions for Tenant A and Tenant B
    let s_a_id = open_impersonation_session(&owner_db, platform_user_id, tenant_a, "Alpha Support", 3600)
        .await
        .expect("open A");
    let s_b_id = open_impersonation_session(&owner_db, platform_user_id, tenant_b, "Beta Support", 3600)
        .await
        .expect("open B");

    // Owner of Tenant A queries impersonations
    let ctx_a = TenantContext::new(tenant_a, owner_a, vec![]);
    let tx_a = Tx::begin(&app_db, &ctx_a).await.expect("begin tx A");

    let sessions_a = list_impersonation_sessions_for_tenant(&tx_a)
        .await
        .expect("list impersonation sessions");

    // Verified: Owner of A sees session for A, but NOT session for B
    let has_a = sessions_a.iter().any(|s| s.id == s_a_id && s.reason == "Alpha Support");
    let has_b = sessions_a.iter().any(|s| s.id == s_b_id);

    assert!(has_a, "Pharmacy A owner must see impersonations on Pharmacy A");
    assert!(!has_b, "Pharmacy A owner must not see impersonations on Pharmacy B");

    tx_a.commit().await.expect("commit tx A");
}

#[tokio::test]
async fn an_impersonated_write_names_both_identities() {
    let owner_db = match get_owner_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };

    let platform_user_id = Uuid::now_v7();
    let tenant_id = TenantId::new_v7();
    seed_platform_user(&owner, platform_user_id, &format!("dual_{}@test.com", Uuid::now_v7()), "Pass123!", TEST_TOTP_SECRET, "super_admin").await;
    seed_tenant(&owner, tenant_id, "Dual Identity Pharmacy").await;

    let session_id = open_impersonation_session(
        &owner_db,
        platform_user_id,
        tenant_id,
        "Fixing inventory batch discrepancies",
        3600,
    )
    .await
    .expect("open session");

    enable_impersonation_writes(&owner_db, session_id, platform_user_id)
        .await
        .expect("enable writes");

    // Execute impersonated write
    let (_, audit) = execute_impersonated_write(&owner_db, session_id, platform_user_id, |_| {
        Ok("batch_updated")
    })
    .await
    .expect("execute impersonated write");

    // Verified: The write names BOTH the tenant identity and the platform staff identity!
    assert_eq!(audit.tenant_id, tenant_id);
    assert_eq!(audit.platform_user_id, platform_user_id);
    assert_eq!(audit.reason, "Fixing inventory batch discrepancies");
}

/// Module 03 Q3, answered 2026-10-24: the impersonation lifetime is configuration. The
/// handler passes `settings.impersonation_ttl_secs`, so a deployment that sets two minutes
/// gets two minutes rather than the hour this code used to hardcode at the call site.
#[tokio::test]
async fn the_impersonation_lifetime_comes_from_configuration() {
    let owner_db = match get_owner_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };

    let platform_user_id = Uuid::now_v7();
    let tenant_id = TenantId::new_v7();
    seed_platform_user(
        &owner,
        platform_user_id,
        &format!("imp_ttl_{}@test.com", Uuid::now_v7()),
        "Pass123!",
        TEST_TOTP_SECRET,
        "support",
    )
    .await;
    seed_tenant(&owner, tenant_id, "Impersonation TTL Target").await;

    let configured = identity::TokenSettings {
        jwt_secret: JWT_SECRET.to_string(),
        access_ttl_secs: 900,
        refresh_ttl_secs: 30 * 24 * 60 * 60,
        impersonation_ttl_secs: 120,
    };

    let session_id = open_impersonation_session(
        &owner_db,
        platform_user_id,
        tenant_id,
        "Checking a sync complaint",
        configured.impersonation_ttl_secs,
    )
    .await
    .expect("open impersonation");

    let expires_at: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT expires_at FROM impersonation_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_one(&owner)
            .await
            .unwrap();

    let lived = (expires_at - chrono::Utc::now()).num_seconds();
    assert!(
        (60..=120).contains(&lived),
        "session should expire about two minutes out, got {lived}s — the hardcoded hour would be 3600s"
    );
}
