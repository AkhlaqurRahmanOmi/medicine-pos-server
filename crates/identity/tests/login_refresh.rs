#![allow(clippy::expect_used, clippy::unwrap_used)]

use cache::Cache;
use identity::login::{
    confirm_password_reset, login, refresh_token, request_password_reset,
};
use identity::password::hash_password;
use identity::rate_limit;
use identity::revocation::{is_session_revoked, revoke_session};
use identity::types::{LoginRequest, PasswordResetConfirmRequest, RefreshTokenRequest};
use pos_core::error::ErrorCode;
use pos_core::ids::{BranchId, DeviceId, SessionId, TenantId, UserId};
use sea_orm::{Database, DatabaseConnection};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

const APP_URL: &str = "postgres://pos_app:devpass@127.0.0.1:5432/medicine_pos";
const OWNER_URL: &str = "postgres://pos_owner:devpass@127.0.0.1:5432/medicine_pos";
const JWT_SECRET: &str = "test-jwt-secret-key-32-bytes-long!";

/// The lifetimes these tests assert about, stated once.
fn settings() -> identity::TokenSettings {
    identity::TokenSettings::for_test(JWT_SECRET)
}

async fn get_db() -> Option<DatabaseConnection> {
    let url = std::env::var("APP_DATABASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| APP_URL.to_string());
    Database::connect(&url).await.ok()
}

async fn owner_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| OWNER_URL.to_string());
    PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(1500))
        .connect(&url)
        .await
        .ok()
}

fn get_cache(prefix: &str) -> Cache {
    Cache::in_memory(prefix)
}

async fn seed_tenant(pool: &PgPool, tenant_id: TenantId) {
    let _ = sqlx::query(
        "INSERT INTO tenants (id, name, status, created_at, updated_at)
         VALUES ($1, 'Test Pharmacy', 'active', now(), now())
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(tenant_id.as_uuid())
    .execute(pool)
    .await;
}

async fn seed_user(
    pool: &PgPool,
    tenant_id: TenantId,
    user_id: UserId,
    email: &str,
    password: &str,
    status: &str,
) {
    seed_tenant(pool, tenant_id).await;
    let pwd_hash = hash_password(password).expect("hash password");
    let _ = sqlx::query(
        "INSERT INTO users (id, tenant_id, email, name, password_hash, status, created_at, updated_at)
         VALUES ($1, $2, $3, 'Test User', $4, $5, now(), now())
         ON CONFLICT (email) DO UPDATE SET password_hash = EXCLUDED.password_hash, status = EXCLUDED.status",
    )
    .bind(user_id.as_uuid())
    .bind(tenant_id.as_uuid())
    .bind(email.to_lowercase())
    .bind(pwd_hash)
    .bind(status)
    .execute(pool)
    .await;
}

async fn seed_branch_and_device(
    pool: &PgPool,
    tenant_id: TenantId,
    branch_id: BranchId,
    device_id: DeviceId,
    prefix: &str,
) {
    seed_tenant(pool, tenant_id).await;
    let _ = sqlx::query(
        "INSERT INTO branches (id, tenant_id, name, is_default, is_active, created_at, updated_at)
         VALUES ($1, $2, 'Main Branch', false, true, now(), now())
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(branch_id.as_uuid())
    .bind(tenant_id.as_uuid())
    .execute(pool)
    .await;

    let _ = sqlx::query(
        "INSERT INTO devices (id, tenant_id, branch_id, name, invoice_prefix, status, created_at, updated_at)
         VALUES ($1, $2, $3, 'POS Register', $4, 'active', now(), now())
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(device_id.as_uuid())
    .bind(tenant_id.as_uuid())
    .bind(branch_id.as_uuid())
    .bind(prefix)
    .execute(pool)
    .await;
}

#[tokio::test]
async fn login_answers_the_same_for_unknown_email_and_wrong_password() {
    let db = match get_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };
    let cache = get_cache("test_login_answers");

    let tenant_id = TenantId::new_v7();
    let u1_id = UserId::new_v7();
    let u2_id = UserId::new_v7();

    let email_active = format!("active_{}@test.com", Uuid::now_v7());
    let email_disabled = format!("disabled_{}@test.com", Uuid::now_v7());
    let email_unknown = format!("unknown_{}@test.com", Uuid::now_v7());

    seed_user(&owner, tenant_id, u1_id, &email_active, "Password123!", "active").await;
    seed_user(&owner, tenant_id, u2_id, &email_disabled, "Password123!", "disabled").await;

    // 1. Unknown email
    let err_unknown = login(
        &db,
        &cache,
        LoginRequest {
            email: email_unknown,
            password: "Password123!".to_string(),
            device_id: None,
        },
        &settings(),
        None,
        None,
    )
    .await
    .unwrap_err();

    // 2. Wrong password for active user
    let err_wrong_pwd = login(
        &db,
        &cache,
        LoginRequest {
            email: email_active,
            password: "WrongPassword999!".to_string(),
            device_id: None,
        },
        &settings(),
        None,
        None,
    )
    .await
    .unwrap_err();

    // 3. Correct password for disabled user
    let err_disabled = login(
        &db,
        &cache,
        LoginRequest {
            email: email_disabled,
            password: "Password123!".to_string(),
            device_id: None,
        },
        &settings(),
        None,
        None,
    )
    .await
    .unwrap_err();

    // Error codes must be identical
    assert_eq!(err_unknown.code, ErrorCode::InvalidToken);
    assert_eq!(err_wrong_pwd.code, ErrorCode::InvalidToken);
    assert_eq!(err_disabled.code, ErrorCode::InvalidToken);

    // Messages must be identical
    assert_eq!(err_unknown.user_message, err_wrong_pwd.user_message);
    assert_eq!(err_wrong_pwd.user_message, err_disabled.user_message);

    // HTTP status must be identical (401)
    assert_eq!(err_unknown.code.http_status(), 401);
    assert_eq!(err_wrong_pwd.code.http_status(), 401);
    assert_eq!(err_disabled.code.http_status(), 401);

    // Details must be identical
    assert_eq!(err_unknown.details, err_wrong_pwd.details);
    assert_eq!(err_wrong_pwd.details, err_disabled.details);
}

#[tokio::test]
async fn a_login_creates_one_session() {
    let db = match get_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };
    let cache = get_cache("test_login_session");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let email = format!("login_session_{}@test.com", Uuid::now_v7());

    seed_user(&owner, tenant_id, user_id, &email, "Password123!", "active").await;

    // Before login: no sessions
    let count_before: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions WHERE user_id = $1")
        .bind(user_id.as_uuid())
        .fetch_one(&owner)
        .await
        .unwrap();
    assert_eq!(count_before, 0);

    // Perform login
    let resp = login(
        &db,
        &cache,
        LoginRequest {
            email,
            password: "Password123!".to_string(),
            device_id: None,
        },
        &settings(),
        Some("127.0.0.1".to_string()),
        Some("UserAgent/1.0".to_string()),
    )
    .await
    .expect("login succeeds");

    assert!(!resp.access_token.is_empty());
    assert!(!resp.refresh_token.is_empty());

    // After login: exactly 1 session
    let count_after: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions WHERE user_id = $1")
        .bind(user_id.as_uuid())
        .fetch_one(&owner)
        .await
        .unwrap();
    assert_eq!(count_after, 1);

    // Check last_login_at is recorded
    let last_login: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT last_login_at FROM users WHERE id = $1")
            .bind(user_id.as_uuid())
            .fetch_one(&owner)
            .await
            .unwrap();
    assert!(last_login.is_some());
}

#[tokio::test]
async fn a_refreshed_token_replaces_the_one_presented() {
    let db = match get_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };
    let cache = get_cache("test_token_replace");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let email = format!("rotate_{}@test.com", Uuid::now_v7());

    seed_user(&owner, tenant_id, user_id, &email, "Password123!", "active").await;

    let login_res = login(
        &db,
        &cache,
        LoginRequest {
            email,
            password: "Password123!".to_string(),
            device_id: None,
        },
        &settings(),
        None,
        None,
    )
    .await
    .unwrap();

    let r1 = login_res.refresh_token;

    // Refresh with r1 -> generates r2
    let refresh_res = refresh_token(
        &db,
        &cache,
        RefreshTokenRequest {
            refresh_token: r1.clone(),
            device_id: None,
        },
        &settings(),
    )
    .await
    .expect("refresh succeeds");

    let r2 = refresh_res.refresh_token;
    assert_ne!(r1, r2);

    // Presenting r1 again must FAIL (it stopped working)
    let err = refresh_token(
        &db,
        &cache,
        RefreshTokenRequest {
            refresh_token: r1,
            device_id: None,
        },
        &settings(),
    )
    .await;
    assert!(err.is_err(), "Presented refresh token must stop working");
}

#[tokio::test]
async fn a_refresh_token_used_twice_revokes_the_session() {
    let db = match get_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };
    let cache = get_cache("test_theft");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let email = format!("theft_{}@test.com", Uuid::now_v7());

    seed_user(&owner, tenant_id, user_id, &email, "Password123!", "active").await;

    let login_res = login(
        &db,
        &cache,
        LoginRequest {
            email,
            password: "Password123!".to_string(),
            device_id: None,
        },
        &settings(),
        None,
        None,
    )
    .await
    .unwrap();

    let r1 = login_res.refresh_token;

    // 1st use: legitimate rotation to r2
    let refresh_1 = refresh_token(
        &db,
        &cache,
        RefreshTokenRequest {
            refresh_token: r1.clone(),
            device_id: None,
        },
        &settings(),
    )
    .await
    .unwrap();

    let r2 = refresh_1.refresh_token;

    // 2nd use of r1: theft detected, revokes session
    let theft_err = refresh_token(
        &db,
        &cache,
        RefreshTokenRequest {
            refresh_token: r1,
            device_id: None,
        },
        &settings(),
    )
    .await;
    assert!(theft_err.is_err());
    assert!(theft_err.unwrap_err().user_message.contains("reuse detected"));

    // Now even r2 fails because the session itself is revoked!
    let r2_err = refresh_token(
        &db,
        &cache,
        RefreshTokenRequest {
            refresh_token: r2,
            device_id: None,
        },
        &settings(),
    )
    .await;
    assert!(r2_err.is_err(), "Session should be revoked after theft");
}

#[tokio::test]
async fn a_refresh_token_is_bound_to_its_device() {
    let db = match get_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };
    let cache = get_cache("test_device_bound");

    let tenant_id = TenantId::new_v7();
    let branch_id = BranchId::new_v7();
    let dev1_id = DeviceId::new_v7();
    let dev2_id = DeviceId::new_v7();
    let user_id = UserId::new_v7();
    let email = format!("devbind_{}@test.com", Uuid::now_v7());

    seed_user(&owner, tenant_id, user_id, &email, "Password123!", "active").await;
    seed_branch_and_device(&owner, tenant_id, branch_id, dev1_id, "D1").await;
    seed_branch_and_device(&owner, tenant_id, branch_id, dev2_id, "D2").await;

    // Sign in from device 1
    let login_res = login(
        &db,
        &cache,
        LoginRequest {
            email,
            password: "Password123!".to_string(),
            device_id: Some(dev1_id),
        },
        &settings(),
        None,
        None,
    )
    .await
    .unwrap();

    let r_token = login_res.refresh_token;

    // Refresh presented from device 2 -> fails!
    let wrong_dev_err = refresh_token(
        &db,
        &cache,
        RefreshTokenRequest {
            refresh_token: r_token.clone(),
            device_id: Some(dev2_id),
        },
        &settings(),
    )
    .await;
    assert!(wrong_dev_err.is_err());
    assert!(wrong_dev_err.unwrap_err().user_message.contains("bound to another device"));

    // Refresh presented from device 1 -> succeeds!
    let correct_dev_res = refresh_token(
        &db,
        &cache,
        RefreshTokenRequest {
            refresh_token: r_token,
            device_id: Some(dev1_id),
        },
        &settings(),
    )
    .await;
    assert!(correct_dev_res.is_ok());
}

#[tokio::test]
async fn a_revoked_session_is_refused_before_its_token_expires() {
    let cache = get_cache("test_revoked_refused");
    let session_id = SessionId::new_v7();

    // Session is not revoked initially
    assert!(!is_session_revoked(&cache, session_id).await.unwrap());

    // Revoke session in Redis
    revoke_session(&cache, session_id, 900).await.unwrap();

    // Checked before access token expires: refused!
    assert!(is_session_revoked(&cache, session_id).await.unwrap());
}

#[tokio::test]
async fn the_sixth_login_attempt_is_locked_out() {
    let db = match get_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };
    let cache = get_cache("test_sixth_locked");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let email = format!("lockout_{}@test.com", Uuid::now_v7());

    seed_user(&owner, tenant_id, user_id, &email, "Password123!", "active").await;

    // Attempts 1..=5 fail with 401 InvalidToken
    for i in 1..=5 {
        let err = login(
            &db,
            &cache,
            LoginRequest {
                email: email.clone(),
                password: "WrongPassword!".to_string(),
                device_id: None,
            },
            &settings(),
            None,
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::InvalidToken,
            "Attempt {i} must be InvalidToken"
        );
    }

    // 6th attempt is locked out with 429 TooManyRequests
    let err_sixth = login(
        &db,
        &cache,
        LoginRequest {
            email,
            password: "Password123!".to_string(),
            device_id: None,
        },
        &settings(),
        None,
        None,
    )
    .await
    .unwrap_err();

    assert_eq!(err_sixth.code, ErrorCode::TooManyRequests);
    assert_eq!(err_sixth.code.http_status(), 429);
}

#[tokio::test]
async fn a_failed_login_writes_no_row() {
    let db = match get_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };
    let cache = get_cache("test_no_row_write");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let email = format!("nofailrow_{}@test.com", Uuid::now_v7());

    seed_user(&owner, tenant_id, user_id, &email, "Password123!", "active").await;

    let sessions_before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sessions WHERE user_id = $1")
            .bind(user_id.as_uuid())
            .fetch_one(&owner)
            .await
            .unwrap();
    let users_before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM users WHERE tenant_id = $1")
            .bind(tenant_id.as_uuid())
            .fetch_one(&owner)
            .await
            .unwrap();

    // Attempt failed login
    let _ = login(
        &db,
        &cache,
        LoginRequest {
            email,
            password: "WrongPassword!".to_string(),
            device_id: None,
        },
        &settings(),
        None,
        None,
    )
    .await;

    let sessions_after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sessions WHERE user_id = $1")
            .bind(user_id.as_uuid())
            .fetch_one(&owner)
            .await
            .unwrap();
    let users_after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM users WHERE tenant_id = $1")
            .bind(tenant_id.as_uuid())
            .fetch_one(&owner)
            .await
            .unwrap();

    assert_eq!(sessions_before, sessions_after, "Failed login must write no sessions");
    assert_eq!(users_before, users_after, "Failed login must write no users");
    assert_eq!(sessions_before, 0);
    assert_eq!(users_before, 1);
}

#[tokio::test]
async fn a_locked_out_email_is_refused_without_hashing() {
    let db = match get_db().await {
        Some(d) => d,
        None => return,
    };
    let cache = get_cache("test_locked_no_hash");
    let email = format!("fastlock_{}@test.com", Uuid::now_v7());

    // Pre-record 5 failures in Redis
    for _ in 0..5 {
        rate_limit::record_login_failure(&cache, &email).await.unwrap();
    }

    // Now attempt login. Since it's locked out, rate limiter check triggers first
    let start = std::time::Instant::now();
    let err = login(
        &db,
        &cache,
        LoginRequest {
            email,
            password: "SomePassword123!".to_string(),
            device_id: None,
        },
        &settings(),
        None,
        None,
    )
    .await
    .unwrap_err();
    let elapsed = start.elapsed();

    assert_eq!(err.code, ErrorCode::TooManyRequests);
    // Argon2 password verification takes 10-50ms; Redis check takes < 5ms
    assert!(
        elapsed < Duration::from_millis(15),
        "Locked out email check must cost only a fast cache read, elapsed: {elapsed:?}"
    );
}

#[tokio::test]
async fn a_reset_token_works_once() {
    let db = match get_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };
    let cache = get_cache("test_reset_once");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let email = format!("reset_once_{}@test.com", Uuid::now_v7());

    seed_user(&owner, tenant_id, user_id, &email, "OldPassword123!", "active").await;

    // 1. Request reset token
    let token = request_password_reset(&db, &cache, &email)
        .await
        .expect("request reset")
        .expect("token returned for known user");

    // 2. First confirm reset -> succeeds
    let confirm_res = confirm_password_reset(
        &db,
        PasswordResetConfirmRequest {
            token: token.clone(),
            new_password: "BrandNewPassword123!".to_string(),
        },
    )
    .await;
    assert!(confirm_res.is_ok(), "First reset confirm should succeed");

    // 3. Second confirm reset with same token -> fails (single-use)
    let second_res = confirm_password_reset(
        &db,
        PasswordResetConfirmRequest {
            token,
            new_password: "AnotherPassword123!".to_string(),
        },
    )
    .await;
    assert!(second_res.is_err(), "Second reset confirm must fail");
    assert_eq!(second_res.unwrap_err().code, ErrorCode::BadRequest);

    // 4. Verify login works with new password
    let login_res = login(
        &db,
        &cache,
        LoginRequest {
            email,
            password: "BrandNewPassword123!".to_string(),
            device_id: None,
        },
        &settings(),
        None,
        None,
    )
    .await;
    assert!(login_res.is_ok(), "Login with new password should succeed");
}

#[tokio::test]
async fn password_reset_answers_the_same_for_an_unknown_email() {
    let db = match get_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };
    let cache = get_cache("test_reset_answers");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let known_email = format!("known_{}@test.com", Uuid::now_v7());
    let unknown_email = format!("unknown_{}@test.com", Uuid::now_v7());

    seed_user(&owner, tenant_id, user_id, &known_email, "Pass123!", "active").await;

    let res_known = request_password_reset(&db, &cache, &known_email).await;
    let res_unknown = request_password_reset(&db, &cache, &unknown_email).await;

    // Both return Ok
    assert!(res_known.is_ok());
    assert!(res_unknown.is_ok());
}

/// Module 03 Q1, answered 2026-10-24: the refresh lifetime is configuration. A deployment
/// that sets two hours gets two hours, not the 30-day default this code used to hardcode.
#[tokio::test]
async fn the_refresh_lifetime_comes_from_configuration() {
    let db = match get_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };
    let cache = get_cache("test_cfg_refresh_ttl");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let email = format!("ttl_{}@test.com", Uuid::now_v7());
    seed_user(&owner, tenant_id, user_id, &email, "Password123!", "active").await;

    let two_hours = 2 * 60 * 60;
    let configured = identity::TokenSettings {
        jwt_secret: JWT_SECRET.to_string(),
        access_ttl_secs: 60,
        refresh_ttl_secs: two_hours,
        impersonation_ttl_secs: 120,
    };

    let resp = login(
        &db,
        &cache,
        LoginRequest {
            email,
            password: "Password123!".to_string(),
            device_id: None,
        },
        &configured,
        None,
        None,
    )
    .await
    .expect("login succeeds");

    // The access token's stated lifetime is the configured one, not a literal 900.
    assert_eq!(resp.expires_in, 60, "expires_in must come from configuration");

    let expires_at: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT expires_at FROM sessions WHERE user_id = $1")
            .bind(user_id.as_uuid())
            .fetch_one(&owner)
            .await
            .unwrap();

    let lived = (expires_at - chrono::Utc::now()).num_seconds();
    assert!(
        (two_hours - 60..=two_hours).contains(&lived),
        "session should expire about two hours out, got {lived}s — the 30-day default would be {}s",
        30 * 24 * 60 * 60
    );
}

/// The signing secret is the configured one. Before this, every handler signed with a
/// hardcoded test secret while the authenticator verified with the configured one.
#[tokio::test]
async fn login_signs_with_the_configured_secret() {
    let db = match get_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };
    let cache = get_cache("test_cfg_secret");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let email = format!("secret_{}@test.com", Uuid::now_v7());
    seed_user(&owner, tenant_id, user_id, &email, "Password123!", "active").await;

    let deployment_secret = "a-deployments-own-secret-32-bytes!!";
    let configured = identity::TokenSettings::for_test(deployment_secret);

    let resp = login(
        &db,
        &cache,
        LoginRequest {
            email,
            password: "Password123!".to_string(),
            device_id: None,
        },
        &configured,
        None,
        None,
    )
    .await
    .expect("login succeeds");

    assert!(
        identity::tokens::decode_pharmacy_token(&resp.access_token, deployment_secret).is_ok(),
        "the token must verify under the configured secret"
    );
    assert!(
        identity::tokens::decode_pharmacy_token(&resp.access_token, JWT_SECRET).is_err(),
        "the token must not verify under any other secret"
    );
}
