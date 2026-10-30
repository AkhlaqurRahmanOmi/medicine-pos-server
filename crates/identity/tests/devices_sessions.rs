#![allow(clippy::expect_used, clippy::unwrap_used)]

use cache::Cache;
use db::Tx;
use identity::commands::{
    disable_user, grant_role, register_device, revoke_device, seed_roles_for_tenant, set_user_pin,
    unlock_session_with_pin, verify_manager_override,
};
use identity::login::login;
use identity::password::hash_password;
use identity::revocation;
use identity::types::LoginRequest;
use pos_core::context::TenantContext;
use pos_core::ids::{BranchId, SessionId, TenantId, UserId};
use sea_orm::{Database, DatabaseConnection};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

const APP_URL: &str = "postgres://pos_app:devpass@127.0.0.1:5432/medicine_pos";
const OWNER_URL: &str = "postgres://pos_owner:devpass@127.0.0.1:5432/medicine_pos";
const JWT_SECRET: &str = "test_jwt_secret_key_at_least_32_bytes_long!!";

/// The lifetimes these tests assert about, stated once.
fn settings() -> identity::TokenSettings {
    identity::TokenSettings::for_test(JWT_SECRET)
}

async fn get_db() -> Option<DatabaseConnection> {
    Database::connect(APP_URL).await.ok()
}

async fn owner_pool() -> Option<PgPool> {
    PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(2))
        .connect(OWNER_URL)
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

async fn seed_branch(pool: &PgPool, tenant_id: TenantId, branch_id: BranchId, name: &str) {
    seed_tenant(pool, tenant_id).await;
    let _ = sqlx::query(
        "INSERT INTO branches (id, tenant_id, name, is_default, is_active, created_at, updated_at)
         VALUES ($1, $2, $3, false, true, now(), now())
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(branch_id.as_uuid())
    .bind(tenant_id.as_uuid())
    .bind(name)
    .execute(pool)
    .await;
}

async fn seed_user(
    pool: &PgPool,
    tenant_id: TenantId,
    user_id: UserId,
    email: &str,
    status: &str,
    password: &str,
) {
    seed_tenant(pool, tenant_id).await;
    let pwd_hash = hash_password(password).unwrap_or_default();
    let _ = sqlx::query(
        "INSERT INTO users (id, tenant_id, email, name, password_hash, status, created_at, updated_at)
         VALUES ($1, $2, $3, 'Test User', $4, $5, now(), now())
         ON CONFLICT (email) DO UPDATE SET status = EXCLUDED.status",
    )
    .bind(user_id.as_uuid())
    .bind(tenant_id.as_uuid())
    .bind(email.to_lowercase())
    .bind(pwd_hash)
    .bind(status)
    .execute(pool)
    .await;
}

#[tokio::test]
async fn a_second_device_cannot_take_a_used_prefix() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner pool required");

    let tenant_a = TenantId::new_v7();
    let branch_a = BranchId::new_v7();
    let user_a = UserId::new_v7();
    seed_branch(&owner, tenant_a, branch_a, "Branch A").await;
    seed_user(&owner, tenant_a, user_a, &format!("dev_a_{}@test.com", Uuid::now_v7()), "active", "Pass123!").await;

    let ctx_a = TenantContext::new(tenant_a, user_a, vec![]);
    let tx_a = Tx::begin(&db, &ctx_a).await.expect("begin tenant A tx");

    let prefix = format!("PFX{}", &Uuid::now_v7().to_string()[..5]);

    // 1. Register first device in Tenant A -> Succeeded
    let dev1_res = register_device(&tx_a, branch_a, "POS-1", &prefix, Some("android"), Some("1.0.0")).await;
    assert!(dev1_res.is_ok(), "First device registration should succeed");

    // 2. Register second device with SAME prefix in Tenant A -> Fails with conflict
    let dev2_res = register_device(&tx_a, branch_a, "POS-2", &prefix, Some("android"), Some("1.0.0")).await;
    assert!(dev2_res.is_err(), "Second device with same prefix must fail");
    let err = dev2_res.err().unwrap();
    assert_eq!(err.code.http_status(), 409);

    tx_a.commit().await.expect("commit tenant A tx");

    // 3. Register device with SAME prefix in Tenant B -> Succeeded (unique per tenant)
    let tenant_b = TenantId::new_v7();
    let branch_b = BranchId::new_v7();
    let user_b = UserId::new_v7();
    seed_branch(&owner, tenant_b, branch_b, "Branch B").await;
    seed_user(&owner, tenant_b, user_b, &format!("dev_b_{}@test.com", Uuid::now_v7()), "active", "Pass123!").await;

    let ctx_b = TenantContext::new(tenant_b, user_b, vec![]);
    let tx_b = Tx::begin(&db, &ctx_b).await.expect("begin tenant B tx");

    let dev3_res = register_device(&tx_b, branch_b, "POS-1-B", &prefix, Some("android"), Some("1.0.0")).await;
    assert!(dev3_res.is_ok(), "Same prefix in different tenant must succeed");

    tx_b.commit().await.expect("commit tenant B tx");
}

#[tokio::test]
async fn revoking_a_device_revokes_its_sessions() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner pool required");
    let cache = get_cache("test_revoke_device");

    let tenant_id = TenantId::new_v7();
    let branch_id = BranchId::new_v7();
    let user_id = UserId::new_v7();
    seed_branch(&owner, tenant_id, branch_id, "Main Branch").await;
    seed_user(&owner, tenant_id, user_id, &format!("dev_rev_{}@test.com", Uuid::now_v7()), "active", "Pass123!").await;

    let ctx = TenantContext::new(tenant_id, user_id, vec![]);
    let tx = Tx::begin(&db, &ctx).await.expect("begin tx");

    let prefix = format!("RV{}", &Uuid::now_v7().to_string()[..4]);
    let device_id = register_device(&tx, branch_id, "Counter 1", &prefix, Some("android"), None)
        .await
        .expect("register device");
    tx.commit().await.expect("commit register");

    // Create 2 sessions attached to this device
    let sess1_id = SessionId::new_v7();
    let sess2_id = SessionId::new_v7();
    let token1_hash = format!("hash_{}", Uuid::now_v7());
    let token2_hash = format!("hash_{}", Uuid::now_v7());

    sqlx::query(
        "INSERT INTO sessions (id, tenant_id, user_id, device_id, refresh_token_hash, expires_at, created_at)
         VALUES ($1, $2, $3, $4, $5, now() + interval '1 day', now()),
                ($6, $2, $3, $4, $7, now() + interval '1 day', now())",
    )
    .bind(sess1_id.as_uuid())
    .bind(tenant_id.as_uuid())
    .bind(user_id.as_uuid())
    .bind(device_id.as_uuid())
    .bind(token1_hash)
    .bind(sess2_id.as_uuid())
    .bind(token2_hash)
    .execute(&owner)
    .await
    .expect("insert sessions");

    // Revoke device
    let tx2 = Tx::begin(&db, &ctx).await.expect("begin tx2");
    revoke_device(&tx2, &cache, device_id).await.expect("revoke device");
    tx2.commit().await.expect("commit revoke");

    // Verify device status in DB
    let dev_status: String = sqlx::query_scalar("SELECT status FROM devices WHERE id = $1 AND tenant_id = $2")
        .bind(device_id.as_uuid())
        .bind(tenant_id.as_uuid())
        .fetch_one(&owner)
        .await
        .unwrap();
    assert_eq!(dev_status, "revoked");

    // Verify both sessions have revoked_at IS NOT NULL in DB
    let revoked_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sessions WHERE device_id = $1 AND tenant_id = $2 AND revoked_at IS NOT NULL",
    )
    .bind(device_id.as_uuid())
    .bind(tenant_id.as_uuid())
    .fetch_one(&owner)
    .await
    .unwrap();
    assert_eq!(revoked_count, 2);

    // Verify Redis revocation list
    let r1 = revocation::is_session_revoked(&cache, sess1_id).await.unwrap();
    let r2 = revocation::is_session_revoked(&cache, sess2_id).await.unwrap();
    assert!(r1, "Session 1 must be marked revoked in Redis");
    assert!(r2, "Session 2 must be marked revoked in Redis");
}

#[tokio::test]
async fn disabling_a_user_revokes_their_sessions() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner pool required");
    let cache = get_cache("test_disable_user_sess");

    let tenant_id = TenantId::new_v7();
    let owner_user_id = UserId::new_v7();
    let cashier_user_id = UserId::new_v7();
    seed_user(&owner, tenant_id, owner_user_id, &format!("own_{}@test.com", Uuid::now_v7()), "active", "Pass123!").await;
    seed_user(&owner, tenant_id, cashier_user_id, &format!("cash_{}@test.com", Uuid::now_v7()), "active", "Pass123!").await;

    // Seed roles and grant owner role to owner_user_id so owner is protected
    let ctx_admin = TenantContext::new(tenant_id, owner_user_id, vec![]);
    let tx_seed = Tx::begin(&db, &ctx_admin).await.expect("begin seed");
    let (owner_role_id, _, _) = seed_roles_for_tenant(&tx_seed).await.expect("seed roles");
    grant_role(&tx_seed, &cache, owner_user_id, owner_role_id, None).await.expect("grant owner");
    tx_seed.commit().await.expect("commit seed");

    // Create 2 sessions for cashier
    let sess1_id = SessionId::new_v7();
    let sess2_id = SessionId::new_v7();
    let token1_hash = format!("hash_{}", Uuid::now_v7());
    let token2_hash = format!("hash_{}", Uuid::now_v7());

    sqlx::query(
        "INSERT INTO sessions (id, tenant_id, user_id, refresh_token_hash, expires_at, created_at)
         VALUES ($1, $2, $3, $4, now() + interval '1 day', now()),
                ($5, $2, $3, $6, now() + interval '1 day', now())",
    )
    .bind(sess1_id.as_uuid())
    .bind(tenant_id.as_uuid())
    .bind(cashier_user_id.as_uuid())
    .bind(token1_hash)
    .bind(sess2_id.as_uuid())
    .bind(token2_hash)
    .execute(&owner)
    .await
    .expect("insert sessions");

    // Disable cashier user
    let tx = Tx::begin(&db, &ctx_admin).await.expect("begin disable tx");
    disable_user(&tx, &cache, cashier_user_id).await.expect("disable cashier");
    tx.commit().await.expect("commit disable");

    // Verify user status is disabled
    let status: String = sqlx::query_scalar("SELECT status FROM users WHERE id = $1 AND tenant_id = $2")
        .bind(cashier_user_id.as_uuid())
        .bind(tenant_id.as_uuid())
        .fetch_one(&owner)
        .await
        .unwrap();
    assert_eq!(status, "disabled");

    // Verify sessions revoked in DB
    let revoked_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sessions WHERE user_id = $1 AND tenant_id = $2 AND revoked_at IS NOT NULL",
    )
    .bind(cashier_user_id.as_uuid())
    .bind(tenant_id.as_uuid())
    .fetch_one(&owner)
    .await
    .unwrap();
    assert_eq!(revoked_count, 2);

    // Verify sessions revoked in Redis
    let r1 = revocation::is_session_revoked(&cache, sess1_id).await.unwrap();
    let r2 = revocation::is_session_revoked(&cache, sess2_id).await.unwrap();
    assert!(r1, "Session 1 must be marked revoked in Redis");
    assert!(r2, "Session 2 must be marked revoked in Redis");
}

#[tokio::test]
async fn a_pin_cannot_start_a_session_on_a_new_device() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner pool required");
    let cache = get_cache("test_pin_unlock");

    let tenant_id = TenantId::new_v7();
    let branch_id = BranchId::new_v7();
    let user_id = UserId::new_v7();
    let email = format!("pin_dev_{}@test.com", Uuid::now_v7());
    seed_branch(&owner, tenant_id, branch_id, "Pin Branch").await;
    seed_user(&owner, tenant_id, user_id, &email, "active", "Pass123!").await;

    let ctx = TenantContext::new(tenant_id, user_id, vec![]);
    let tx = Tx::begin(&db, &ctx).await.expect("begin tx");

    // Set user PIN to 1234
    set_user_pin(&tx, user_id, "1234").await.expect("set PIN");

    // Register 2 devices
    let dev1_id = register_device(&tx, branch_id, "Counter 1", &format!("D1{}", &Uuid::now_v7().to_string()[..3]), None, None)
        .await
        .expect("register dev1");
    let dev2_id = register_device(&tx, branch_id, "Counter 2", &format!("D2{}", &Uuid::now_v7().to_string()[..3]), None, None)
        .await
        .expect("register dev2");
    tx.commit().await.expect("commit registration");

    // Login on Device 1 -> creates session on Device 1
    let login_res = login(
        &db,
        &cache,
        LoginRequest {
            email: email.clone(),
            password: "Pass123!".to_string(),
            device_id: Some(dev1_id),
        },
        &settings(),
        None,
        None,
    )
    .await
    .expect("login on dev1");
    assert!(!login_res.access_token.is_empty());

    // Verify session row exists for dev1
    let sess1_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM sessions WHERE user_id = $1 AND device_id = $2 AND revoked_at IS NULL",
    )
    .bind(user_id.as_uuid())
    .bind(dev1_id.as_uuid())
    .fetch_one(&owner)
    .await
    .unwrap();

    let tx2 = Tx::begin(&db, &ctx).await.expect("begin tx2");

    // 1. Attempt to unlock or start session with PIN on Device 2 (where NO session exists for this user)
    let dev2_pin_res = unlock_session_with_pin(&tx2, dev2_id, user_id, "1234").await;
    assert!(dev2_pin_res.is_err(), "A PIN cannot start a session on a new device with no existing session");
    assert_eq!(dev2_pin_res.err().unwrap().code.http_status(), 401);

    // 2. Unlock session with correct PIN on Device 1 (where session exists) -> Succeeded!
    let dev1_pin_res = unlock_session_with_pin(&tx2, dev1_id, user_id, "1234").await;
    assert!(dev1_pin_res.is_ok(), "PIN unlock on existing device session must succeed");
    assert_eq!(dev1_pin_res.unwrap(), SessionId::from(sess1_id));

    // 3. Unlock session with WRONG PIN on Device 1 -> Fails!
    let wrong_pin_res = unlock_session_with_pin(&tx2, dev1_id, user_id, "9999").await;
    assert!(wrong_pin_res.is_err(), "Wrong PIN must fail");
    assert_eq!(wrong_pin_res.err().unwrap().code.http_status(), 401);

    tx2.commit().await.expect("commit tx2");
}

#[tokio::test]
async fn a_pin_is_not_a_credential_at_the_login_endpoint() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner pool required");
    let cache = get_cache("test_pin_not_cred");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let email = format!("pin_not_cred_{}@test.com", Uuid::now_v7());
    seed_user(&owner, tenant_id, user_id, &email, "active", "StrongPassword123!").await;

    let ctx = TenantContext::new(tenant_id, user_id, vec![]);
    let tx = Tx::begin(&db, &ctx).await.expect("begin tx");
    set_user_pin(&tx, user_id, "4321").await.expect("set PIN");
    tx.commit().await.expect("commit tx");

    // 1. Attempt login with the 4-digit PIN as the credential at the login endpoint
    let pin_login_res = login(
        &db,
        &cache,
        LoginRequest {
            email: email.clone(),
            password: "4321".to_string(),
            device_id: None,
        },
        &settings(),
        None,
        None,
    )
    .await;
    assert!(pin_login_res.is_err(), "A PIN must never be accepted as a login credential");
    assert_eq!(pin_login_res.err().unwrap().code.http_status(), 401);

    // 2. Login with actual password succeeds
    let full_pwd_res = login(
        &db,
        &cache,
        LoginRequest {
            email,
            password: "StrongPassword123!".to_string(),
            device_id: None,
        },
        &settings(),
        None,
        None,
    )
    .await;
    assert!(full_pwd_res.is_ok(), "Actual password login must succeed");
}

#[tokio::test]
async fn an_override_records_who_approved_it() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner pool required");
    let cache = get_cache("test_override_mgr");

    let tenant_id = TenantId::new_v7();
    let branch_id = BranchId::new_v7();
    let manager_id = UserId::new_v7();
    let cashier_id = UserId::new_v7();

    seed_branch(&owner, tenant_id, branch_id, "Override Branch").await;
    seed_user(&owner, tenant_id, manager_id, &format!("mgr_{}@test.com", Uuid::now_v7()), "active", "Pass123!").await;
    seed_user(&owner, tenant_id, cashier_id, &format!("csh_{}@test.com", Uuid::now_v7()), "active", "Pass123!").await;

    let ctx_admin = TenantContext::new(tenant_id, manager_id, vec![]);
    let tx = Tx::begin(&db, &ctx_admin).await.expect("begin tx");

    // Seed roles: Manager has device.revoke, Cashier does not
    let (_, mgr_role_id, cash_role_id) = seed_roles_for_tenant(&tx).await.expect("seed roles");
    grant_role(&tx, &cache, manager_id, mgr_role_id, Some(branch_id)).await.expect("grant manager");
    grant_role(&tx, &cache, cashier_id, cash_role_id, Some(branch_id)).await.expect("grant cashier");

    // Set PINs
    set_user_pin(&tx, manager_id, "8888").await.expect("set manager PIN");
    set_user_pin(&tx, cashier_id, "1111").await.expect("set cashier PIN");

    tx.commit().await.expect("commit tx");

    let tx2 = Tx::begin(&db, &ctx_admin).await.expect("begin tx2");

    // 1. Cashier attempts override requiring "device.revoke" -> Fails (forbidden, approver lacks permission)
    let cash_override = verify_manager_override(&tx2, cashier_id, "1111", Some(branch_id), "device.revoke").await;
    assert!(cash_override.is_err(), "Cashier without permission cannot approve override");
    assert_eq!(cash_override.err().unwrap().code.http_status(), 403);

    // 2. Manager enters wrong PIN -> Fails (unauthorized)
    let wrong_pin = verify_manager_override(&tx2, manager_id, "0000", Some(branch_id), "device.revoke").await;
    assert!(wrong_pin.is_err(), "Wrong PIN must fail");
    assert_eq!(wrong_pin.err().unwrap().code.http_status(), 401);

    // 3. Manager enters correct PIN "8888" -> Succeeded, returns manager's UserId
    let approved_by = verify_manager_override(&tx2, manager_id, "8888", Some(branch_id), "device.revoke")
        .await
        .expect("manager approval");

    // Verified: The override records the approver's user id!
    assert_eq!(approved_by, manager_id);

    tx2.commit().await.expect("commit tx2");
}
