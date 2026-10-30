#![allow(clippy::expect_used, clippy::unwrap_used)]

use cache::Cache;
use db::Tx;
use identity::commands::{
    create_role, delete_role, disable_user, grant_role, seed_roles_for_tenant, set_role_permissions,
    update_role,
};
use identity::password::hash_password;
use identity::queries::{
    assignable_permissions, branch_access, branch_access_cached, permissions_at, role_permissions,
};
use pos_core::context::TenantContext;
use pos_core::ids::{BranchId, TenantId, UserId};
use sea_orm::{Database, DatabaseConnection};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

const APP_URL: &str = "postgres://pos_app:devpass@127.0.0.1:5432/medicine_pos";
const OWNER_URL: &str = "postgres://pos_owner:devpass@127.0.0.1:5432/medicine_pos";
const REDIS_URL: &str = "redis://127.0.0.1:6379";

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
    let url = std::env::var("REDIS_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| REDIS_URL.to_string());
    Cache::new(&url, prefix).unwrap_or_else(|_| Cache::in_memory(prefix))
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
    status: &str,
) {
    seed_tenant(pool, tenant_id).await;
    let pwd_hash = hash_password("Password123!").unwrap_or_default();
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

async fn seed_session(
    pool: &PgPool,
    tenant_id: TenantId,
    user_id: UserId,
    session_id: pos_core::ids::SessionId,
) {
    let _ = sqlx::query(
        "INSERT INTO sessions (id, tenant_id, user_id, refresh_token_hash, expires_at, created_at)
         VALUES ($1, $2, $3, $4, now() + interval '1 day', now())
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(session_id.as_uuid())
    .bind(tenant_id.as_uuid())
    .bind(user_id.as_uuid())
    .bind(format!("hash_{}", session_id.as_uuid()))
    .execute(pool)
    .await;
}

#[tokio::test]
async fn a_new_pharmacy_gets_its_roles_from_the_templates() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner pool required");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    seed_user(&owner, tenant_id, user_id, &format!("seed_{}@test.com", Uuid::now_v7()), "active").await;

    let ctx = TenantContext::new(tenant_id, user_id, vec![]);
    let tx = Tx::begin_platform(&db, &ctx).await.expect("begin platform tx");

    let (owner_id, _mgr_id, cash_id) = seed_roles_for_tenant(&tx).await.expect("seed roles");
    tx.commit().await.expect("commit seed tx");

    // Verify 3 roles created with is_system = true
    let roles_count: i64 = sqlx::query_scalar("SELECT count(*) FROM roles WHERE tenant_id = $1 AND is_system = true")
        .bind(tenant_id.as_uuid())
        .fetch_one(&owner)
        .await
        .unwrap();
    assert_eq!(roles_count, 3);

    // Verify owner has is_owner = true
    let is_owner: bool = sqlx::query_scalar("SELECT is_owner FROM roles WHERE id = $1 AND tenant_id = $2")
        .bind(owner_id.as_uuid())
        .bind(tenant_id.as_uuid())
        .fetch_one(&owner)
        .await
        .unwrap();
    assert!(is_owner);

    // Verify cashier has role_permissions populated
    let cashier_perms: i64 = sqlx::query_scalar("SELECT count(*) FROM role_permissions WHERE role_id = $1 AND tenant_id = $2")
        .bind(cash_id.as_uuid())
        .bind(tenant_id.as_uuid())
        .fetch_one(&owner)
        .await
        .unwrap();
    assert!(cashier_perms > 0);
}

#[tokio::test]
async fn a_system_role_cannot_be_renamed_or_deleted() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner pool required");
    let cache = get_cache("test_sys_roles");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    seed_user(&owner, tenant_id, user_id, &format!("sys_{}@test.com", Uuid::now_v7()), "active").await;

    let ctx = TenantContext::new(tenant_id, user_id, vec![]);
    let tx = Tx::begin_platform(&db, &ctx).await.unwrap();
    let (owner_id, _, _) = seed_roles_for_tenant(&tx).await.unwrap();
    tx.commit().await.unwrap();

    // 1. Cannot rename system role
    let tx2 = Tx::begin_platform(&db, &ctx).await.unwrap();
    let rename_err = update_role(&tx2, owner_id, "Boss").await;
    assert!(rename_err.is_err(), "Rename system role must be rejected");
    let _ = tx2.rollback().await;

    // 2. Cannot delete system role
    let tx3 = Tx::begin_platform(&db, &ctx).await.unwrap();
    let delete_err = delete_role(&tx3, owner_id).await;
    assert!(delete_err.is_err(), "Delete system role must be rejected");
    let _ = tx3.rollback().await;

    // 3. Cannot change permissions of system role
    let tx4 = Tx::begin_platform(&db, &ctx).await.unwrap();
    let perm_err = set_role_permissions(&tx4, &cache, owner_id, &[]).await;
    assert!(perm_err.is_err(), "Change permissions of system role must be rejected");
    let _ = tx4.rollback().await;
}

#[tokio::test]
async fn a_platform_only_permission_is_refused_on_direct_submission() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner pool required");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    seed_user(&owner, tenant_id, user_id, &format!("plat_{}@test.com", Uuid::now_v7()), "active").await;

    let ctx = TenantContext::new(tenant_id, user_id, vec![]);
    let tx = Tx::begin_platform(&db, &ctx).await.unwrap();

    // Direct submission of platform permission 'platform.users'
    let create_err = create_role(&tx, "Hacker Role", &["platform.users".to_string()]).await;
    assert!(create_err.is_err());
    assert_eq!(create_err.unwrap_err().code.http_status(), 403);

    let _ = tx.rollback().await;
}

#[tokio::test]
async fn a_platform_only_permission_is_absent_from_the_assignable_list() {
    let db = get_db().await.expect("database required");

    let perms = assignable_permissions(&db).await.expect("query assignable permissions");
    assert!(!perms.is_empty());

    // platform-only permissions must be ABSENT
    for p in &perms {
        assert_ne!(p.code, "platform.user.manage");
        assert!(!p.code.starts_with("platform."));
    }
}

#[tokio::test]
async fn the_owner_role_holds_every_assignable_permission() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner pool required");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    seed_user(&owner, tenant_id, user_id, &format!("owner_perms_{}@test.com", Uuid::now_v7()), "active").await;

    let ctx = TenantContext::new(tenant_id, user_id, vec![]);
    let tx = Tx::begin_platform(&db, &ctx).await.unwrap();
    let (owner_role_id, _, _) = seed_roles_for_tenant(&tx).await.unwrap();
    tx.commit().await.unwrap();

    let assignable = assignable_permissions(&db).await.unwrap();
    // `roles` and `role_permissions` are tenant-owned: read them with the context set.
    let tx_read = Tx::begin(&db, &ctx).await.unwrap();
    let owner_perms = role_permissions(tx_read.transaction().unwrap(), tenant_id, owner_role_id)
        .await
        .unwrap();
    tx_read.commit().await.unwrap();

    // Owner role must hold every single assignable permission
    for ap in assignable {
        assert!(
            owner_perms.contains(&ap.code),
            "Owner role missing assignable permission '{}'",
            ap.code
        );
    }
}

#[tokio::test]
async fn the_last_active_owner_cannot_be_disabled() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner pool required");
    let cache = get_cache("test_last_owner");

    let tenant_id = TenantId::new_v7();
    let u1_id = UserId::new_v7();
    let u2_id = UserId::new_v7();

    seed_user(&owner, tenant_id, u1_id, &format!("u1_owner_{}@test.com", Uuid::now_v7()), "active").await;
    seed_user(&owner, tenant_id, u2_id, &format!("u2_owner_{}@test.com", Uuid::now_v7()), "active").await;

    let ctx = TenantContext::new(tenant_id, u1_id, vec![]);
    let tx = Tx::begin_platform(&db, &ctx).await.unwrap();
    let (owner_role_id, _, _) = seed_roles_for_tenant(&tx).await.unwrap();

    // Grant owner role to u1
    grant_role(&tx, &cache, u1_id, owner_role_id, None).await.unwrap();
    tx.commit().await.unwrap();

    // 1. Try to disable u1 (the only active owner) -> FAILS!
    let tx2 = Tx::begin_platform(&db, &ctx).await.unwrap();
    let err_disable = disable_user(&tx2, &cache, u1_id).await;
    assert!(err_disable.is_err(), "Last active owner cannot be disabled");
    let _ = tx2.rollback().await;

    // 2. Grant owner role to u2 as well
    let tx3 = Tx::begin_platform(&db, &ctx).await.unwrap();
    grant_role(&tx3, &cache, u2_id, owner_role_id, None).await.unwrap();
    tx3.commit().await.unwrap();

    // 3. Now u1 can be disabled because u2 is also an active owner
    let tx4 = Tx::begin_platform(&db, &ctx).await.unwrap();
    let res_disable = disable_user(&tx4, &cache, u1_id).await;
    assert!(res_disable.is_ok(), "u1 can be disabled when u2 is active owner");
    tx4.commit().await.unwrap();

    // 4. Now u2 is the last active owner: attempting to disable u2 -> FAILS!
    let tx5 = Tx::begin_platform(&db, &ctx).await.unwrap();
    let err_u2 = disable_user(&tx5, &cache, u2_id).await;
    assert!(err_u2.is_err(), "u2 is now the last active owner and cannot be disabled");
    let _ = tx5.rollback().await;
}

#[tokio::test]
async fn permissions_at_a_branch_union_the_wide_and_the_local() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner pool required");
    let cache = get_cache("test_perm_union");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let b1_id = BranchId::new_v7();
    let b2_id = BranchId::new_v7();

    seed_user(&owner, tenant_id, user_id, &format!("union_{}@test.com", Uuid::now_v7()), "active").await;
    seed_branch(&owner, tenant_id, b1_id, "Branch A").await;
    seed_branch(&owner, tenant_id, b2_id, "Branch B").await;

    let ctx = TenantContext::new(tenant_id, user_id, vec![b1_id, b2_id]);
    let tx = Tx::begin_platform(&db, &ctx).await.unwrap();

    // Role wide: "user.view"
    let r_wide = create_role(&tx, "Wide Role", &["user.view".to_string()]).await.unwrap();
    // Role local to B1: "device.view"
    let r_b1 = create_role(&tx, "B1 Role", &["device.view".to_string()]).await.unwrap();
    // Role local to B2: "device.register"
    let r_b2 = create_role(&tx, "B2 Role", &["device.register".to_string()]).await.unwrap();

    // Grant wide role (branch_id = None)
    grant_role(&tx, &cache, user_id, r_wide, None).await.unwrap();
    // Grant B1 role
    grant_role(&tx, &cache, user_id, r_b1, Some(b1_id)).await.unwrap();
    // Grant B2 role
    grant_role(&tx, &cache, user_id, r_b2, Some(b2_id)).await.unwrap();

    tx.commit().await.unwrap();

    // Permissions at B1: union of Wide ("user.view") and B1 ("device.view")
    let tx_read = Tx::begin(&db, &ctx).await.unwrap();
    let conn = tx_read.transaction().unwrap();
    let b1_perms = permissions_at(conn, tenant_id, user_id, b1_id).await.unwrap();
    assert!(b1_perms.contains(&"user.view".to_string()));
    assert!(b1_perms.contains(&"device.view".to_string()));
    assert!(!b1_perms.contains(&"device.register".to_string()));

    // Permissions at B2: union of Wide ("user.view") and B2 ("device.register")
    let b2_perms = permissions_at(conn, tenant_id, user_id, b2_id).await.unwrap();
    assert!(b2_perms.contains(&"user.view".to_string()));
    assert!(!b2_perms.contains(&"device.view".to_string()));
    assert!(b2_perms.contains(&"device.register".to_string()));
}

#[tokio::test]
async fn a_null_branch_grant_includes_a_branch_created_later() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner pool required");
    let cache = get_cache("test_branch_later");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let b1_id = BranchId::new_v7();
    let b2_id = BranchId::new_v7();

    seed_user(&owner, tenant_id, user_id, &format!("null_grant_{}@test.com", Uuid::now_v7()), "active").await;
    seed_branch(&owner, tenant_id, b1_id, "Existing Branch 1").await;

    let ctx = TenantContext::new(tenant_id, user_id, vec![b1_id]);
    let tx = Tx::begin_platform(&db, &ctx).await.unwrap();
    let r_wide = create_role(&tx, "Pharmacy-Wide Role", &["user.view".to_string()]).await.unwrap();

    // Grant role with branch_id = None
    grant_role(&tx, &cache, user_id, r_wide, None).await.unwrap();
    tx.commit().await.unwrap();

    // Initial check: has access to b1. Resolved inside a tenant transaction — `branches`
    // and `user_branch_roles` are RLS-forced, so a bare connection reads nothing.
    let tx_read = Tx::begin(&db, &ctx).await.unwrap();
    let branches_initial = branch_access(tx_read.transaction().unwrap(), tenant_id, user_id)
        .await
        .unwrap();
    tx_read.commit().await.unwrap();
    assert!(branches_initial.contains(&b1_id));
    assert!(!branches_initial.contains(&b2_id));

    // Now, create branch 2 LATER
    seed_branch(&owner, tenant_id, b2_id, "New Branch 2 Created Later").await;

    // Second check: automatically includes branch 2 without re-granting!
    let tx_read2 = Tx::begin(&db, &ctx).await.unwrap();
    let branches_after = branch_access(tx_read2.transaction().unwrap(), tenant_id, user_id)
        .await
        .unwrap();
    tx_read2.commit().await.unwrap();
    assert!(branches_after.contains(&b1_id));
    assert!(branches_after.contains(&b2_id));
}

#[tokio::test]
async fn granting_a_role_clears_the_cached_branch_access() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner pool required");
    let cache = get_cache("test_cache_clear");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let b1_id = BranchId::new_v7();

    seed_user(&owner, tenant_id, user_id, &format!("cache_clear_{}@test.com", Uuid::now_v7()), "active").await;
    seed_branch(&owner, tenant_id, b1_id, "Branch 1").await;

    let ctx = TenantContext::new(tenant_id, user_id, vec![b1_id]);
    let tx = Tx::begin_platform(&db, &ctx).await.unwrap();
    let role = create_role(&tx, "Some Role", &["user.view".to_string()]).await.unwrap();
    tx.commit().await.unwrap();

    // Pre-populate cached branch access key in Redis
    let cache_key = format!("t:{tenant_id}:u:{user_id}:branches");
    cache.set_ex(&cache_key, "[\"cached_branch_id\"]", 300).await.unwrap();
    assert!(cache.get(&cache_key).await.unwrap().is_some());

    // Grant a role: must clear the cached key in the same operation
    let tx2 = Tx::begin_platform(&db, &ctx).await.unwrap();
    grant_role(&tx2, &cache, user_id, role, Some(b1_id)).await.unwrap();
    tx2.commit().await.unwrap();

    // Assert cache key is cleared
    assert!(
        cache.get(&cache_key).await.unwrap().is_none(),
        "Cached branch access must be cleared on granting a role"
    );
}

#[tokio::test]
async fn branch_access_denies_when_redis_is_unreachable() {
    let db = get_db().await.expect("database required");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();

    // Create an unreachable redis cache instance
    let bad_cache = Cache::new("redis://127.0.0.1:19999", "bad_redis").unwrap();

    // Lookup must FAIL / DENY access, never fallback to all branches!
    let res = branch_access_cached(&db, &bad_cache, tenant_id, user_id).await;
    assert!(
        res.is_err(),
        "Branch access must deny on unreachable cache, not fallback"
    );
}

/// Phase 11: the gate is only as good as what the middleware puts on the request.
///
/// `identity::auth::authenticate` resolves the holder's grants, so a request arrives with
/// the permissions and branches the user actually holds. Before this, the authenticator
/// returned an empty set and every `require` / `require_at` check denied.
#[tokio::test]
async fn an_authenticated_request_carries_resolved_permissions() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner pool required");
    let cache = get_cache("test_authn_resolve");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let wide_branch = BranchId::new_v7();
    let local_branch = BranchId::new_v7();

    seed_user(
        &owner,
        tenant_id,
        user_id,
        &format!("authn_{}@test.com", Uuid::now_v7()),
        "active",
    )
    .await;
    seed_branch(&owner, tenant_id, wide_branch, "Wide Branch").await;
    seed_branch(&owner, tenant_id, local_branch, "Local Branch").await;

    let ctx = TenantContext::new(tenant_id, user_id, vec![wide_branch, local_branch]);
    let tx = Tx::begin_platform(&db, &ctx).await.unwrap();
    let r_wide = create_role(&tx, "Wide", &["user.view".to_string()])
        .await
        .unwrap();
    let r_local = create_role(&tx, "Local", &["device.register".to_string()])
        .await
        .unwrap();
    grant_role(&tx, &cache, user_id, r_wide, None).await.unwrap();
    grant_role(&tx, &cache, user_id, r_local, Some(local_branch))
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let session_id = pos_core::ids::SessionId::new_v7();
    seed_session(&owner, tenant_id, user_id, session_id).await;

    let secret = "an-authenticator-secret-32-bytes!!";
    let token = identity::tokens::encode_pharmacy_token(user_id, tenant_id, session_id, secret, 900)
        .expect("encode token");

    let user = identity::auth::authenticate(&db, secret, &token)
        .await
        .expect("authenticate");

    assert_eq!(user.user_id, user_id);
    assert_eq!(user.tenant_id, tenant_id);

    // The pharmacy-wide grant is held everywhere.
    assert!(
        user.has_permission("user.view"),
        "a pharmacy-wide permission must be on the request, got {:?}",
        user.permissions
    );

    // A null-branch grant reaches every active branch `[AC-U13]`, so both are present.
    assert!(user.branch_permissions.contains_key(&wide_branch));
    assert!(user.branch_permissions.contains_key(&local_branch));

    // The branch-local grant is held at its branch and nowhere else.
    assert!(user.has_permission_at("device.register", local_branch));
    assert!(
        !user.has_permission_at("device.register", wide_branch),
        "a branch-local permission must not leak to another branch"
    );

    // And nothing the user was never granted.
    assert!(!user.has_permission("role.delete"));
    assert!(!user.has_permission_at("role.delete", local_branch));
}

#[tokio::test]
async fn a_grant_on_an_inactive_branch_is_not_resolved() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner pool required");
    let cache = get_cache("test_inactive_branch");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let active_branch = BranchId::new_v7();
    let inactive_branch = BranchId::new_v7();

    seed_user(
        &owner,
        tenant_id,
        user_id,
        &format!("inact_{}@test.com", Uuid::now_v7()),
        "active",
    )
    .await;
    seed_branch(&owner, tenant_id, active_branch, "Active Branch").await;
    seed_branch(&owner, tenant_id, inactive_branch, "Inactive Branch").await;

    // Deactivate the second branch
    let _ = sqlx::query("UPDATE branches SET is_active = false WHERE id = $1")
        .bind(inactive_branch.as_uuid())
        .execute(&owner)
        .await
        .expect("deactivate branch");

    let ctx = TenantContext::new(tenant_id, user_id, vec![active_branch]);
    let tx = Tx::begin_platform(&db, &ctx).await.unwrap();
    let r_local = create_role(&tx, "CashierLocal", &["device.register".to_string()])
        .await
        .unwrap();
    // Grant role at the inactive branch
    grant_role(&tx, &cache, user_id, r_local, Some(inactive_branch))
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let session_id = pos_core::ids::SessionId::new_v7();
    seed_session(&owner, tenant_id, user_id, session_id).await;

    let secret = "an-authenticator-secret-32-bytes!!";
    let token = identity::tokens::encode_pharmacy_token(user_id, tenant_id, session_id, secret, 900)
        .expect("encode token");

    let user = identity::auth::authenticate(&db, secret, &token)
        .await
        .expect("authenticate");

    // Inactive branch must NOT be resolved into branch_permissions!
    assert!(
        !user.branch_permissions.contains_key(&inactive_branch),
        "Inactive branch must not be present in resolved permissions"
    );
    assert!(
        !user.has_permission_at("device.register", inactive_branch),
        "Permissions on inactive branch must not be resolved"
    );
}

async fn seed_device(
    pool: &PgPool,
    tenant_id: TenantId,
    device_id: pos_core::ids::DeviceId,
    branch_id: BranchId,
    prefix: &str,
) {
    let _ = sqlx::query(
        "INSERT INTO devices (id, tenant_id, branch_id, name, invoice_prefix, status, created_at, updated_at)
         VALUES ($1, $2, $3, 'Register 1', $4, 'active', now(), now())
         ON CONFLICT (tenant_id, invoice_prefix) DO NOTHING",
    )
    .bind(device_id.as_uuid())
    .bind(tenant_id.as_uuid())
    .bind(branch_id.as_uuid())
    .bind(prefix)
    .execute(pool)
    .await
    .expect("seed device");
}

#[tokio::test]
async fn every_pharmacy_endpoint_refuses_a_caller_without_its_permission() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let cache = get_cache("test_perm_refusal");
    let db = get_db().await.expect("database required");
    let db_wrap = db::Db::from_connection(db, APP_URL.to_string());

    // AuthUser with NO permissions at all
    let unprivileged_user = api::AuthUser::new(
        user_id,
        tenant_id,
        vec![],
        std::collections::BTreeMap::new(),
    );
    let ctx = pos_core::context::TenantContext::new(tenant_id, user_id, vec![]);

    let (app, _) = identity::rest::pharmacy_router()
        .layer(axum::Extension(unprivileged_user))
        .layer(axum::Extension(ctx))
        .layer(axum::Extension(db_wrap))
        .layer(axum::Extension(cache))
        .split_for_parts();

    let sample_id = Uuid::now_v7();
    let sample_grant_id = Uuid::now_v7();

    // List of (method, path, body) for every endpoint guarded by a permission in CONTRACT.md §2
    let permissioned_endpoints: Vec<(&str, String, Option<String>)> = vec![
        // user.view
        ("GET", "/users".to_string(), None),
        ("GET", format!("/users/{sample_id}"), None),
        ("GET", "/impersonations".to_string(), None),
        ("GET", "/impersonation-sessions".to_string(), None),
        // user.create
        ("POST", "/users".to_string(), Some(r#"{"name":"New","email":"new@example.com"}"#.to_string())),
        // user.update
        ("PATCH", format!("/users/{sample_id}"), Some(r#"{"name":"Updated"}"#.to_string())),
        // user.disable
        ("POST", format!("/users/{sample_id}/disable"), None),
        // user.reset_password
        ("POST", format!("/users/{sample_id}/reset-password"), None),
        // role.view
        ("GET", "/roles".to_string(), None),
        ("GET", format!("/roles/{sample_id}"), None),
        ("GET", "/permissions".to_string(), None),
        ("GET", "/roles/permissions".to_string(), None),
        // role.create
        ("POST", "/roles".to_string(), Some(r#"{"name":"Custom","permissions":[]}"#.to_string())),
        // role.update
        ("PATCH", format!("/roles/{sample_id}"), Some(r#"{"name":"Renamed"}"#.to_string())),
        ("PUT", format!("/roles/{sample_id}/permissions"), Some(r#"{"permissions":[]}"#.to_string())),
        ("POST", format!("/roles/{sample_id}/permissions"), Some(r#"{"permissions":[]}"#.to_string())),
        // role.delete
        ("DELETE", format!("/roles/{sample_id}"), None),
        // role.assign
        ("POST", format!("/users/{sample_id}/roles"), Some(format!(r#"{{"role_id":"{sample_id}"}}"#))),
        ("DELETE", format!("/users/{sample_id}/roles/{sample_grant_id}"), None),
        ("DELETE", format!("/user-roles/{sample_grant_id}"), None),
        // device.view
        ("GET", "/devices".to_string(), None),
        // device.register
        ("POST", "/devices".to_string(), Some(format!(r#"{{"branch_id":"{sample_id}","name":"POS-1","invoice_prefix":"P1"}}"#))),
        // device.revoke
        ("POST", format!("/devices/{sample_id}/revoke"), None),
        // session.view
        ("GET", "/sessions".to_string(), None),
        // session.revoke
        ("DELETE", format!("/sessions/{sample_id}"), None),
    ];

    for (method, path, body) in permissioned_endpoints {
        let mut builder = Request::builder().method(method).uri(&path);
        let req = if let Some(b) = body {
            builder = builder.header("Content-Type", "application/json");
            builder.body(Body::from(b)).expect("request body")
        } else {
            builder.body(Body::empty()).expect("empty body")
        };

        let resp = app.clone().oneshot(req).await.expect("route response");
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "Endpoint {method} {path} MUST refuse an unprivileged caller with 403 Forbidden"
        );
    }
}

#[tokio::test]
async fn a_pharmacy_reads_its_own_users_roles_and_devices_over_http() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner pool required");
    let cache = get_cache("test_reads_over_http");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let branch_id = BranchId::new_v7();
    let role_id = pos_core::ids::RoleId::new_v7();
    let device_id = pos_core::ids::DeviceId::new_v7();

    seed_tenant(&owner, tenant_id).await;
    seed_branch(&owner, tenant_id, branch_id, "Main Branch").await;
    seed_user(
        &owner,
        tenant_id,
        user_id,
        &format!("reader_{}@example.com", tenant_id.as_uuid()),
        "active",
    )
    .await;

    // Seed role with permissions
    let _ = sqlx::query(
        "INSERT INTO roles (id, tenant_id, name, is_system, is_owner, created_at, updated_at)
         VALUES ($1, $2, 'Manager', false, false, now(), now())
         ON CONFLICT (tenant_id, name) DO NOTHING",
    )
    .bind(role_id.as_uuid())
    .bind(tenant_id.as_uuid())
    .execute(&owner)
    .await
    .expect("seed role");

    let _ = sqlx::query(
        "INSERT INTO role_permissions (tenant_id, role_id, permission_code, created_at)
         VALUES ($1, $2, 'user.view', now())
         ON CONFLICT DO NOTHING",
    )
    .bind(tenant_id.as_uuid())
    .bind(role_id.as_uuid())
    .execute(&owner)
    .await
    .expect("seed role perm");

    // Seed device
    let prefix = format!("D{}", &tenant_id.as_uuid().to_string()[..6]);
    seed_device(&owner, tenant_id, device_id, branch_id, &prefix).await;

    let db_wrap = db::Db::from_connection(db.clone(), APP_URL.to_string());
    let wide_perms = vec![
        "user.view".to_string(),
        "role.view".to_string(),
        "device.view".to_string(),
    ];
    let auth_user = api::AuthUser::new(user_id, tenant_id, wide_perms, std::collections::BTreeMap::new());
    let ctx = pos_core::context::TenantContext::new(tenant_id, user_id, vec![branch_id]);

    let (app, _) = identity::rest::pharmacy_router()
        .layer(axum::Extension(auth_user))
        .layer(axum::Extension(ctx))
        .layer(axum::Extension(db_wrap))
        .layer(axum::Extension(cache))
        .split_for_parts();

    // 1. GET /users
    let req = Request::builder().uri("/users").body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let users: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        !users.as_array().expect("array").is_empty(),
        "list_users must return rows inside tenant transaction"
    );

    // 2. GET /users/{id}
    let req = Request::builder().uri(format!("/users/{}", user_id.as_uuid())).body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let u: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(u["id"].as_str().unwrap(), user_id.as_uuid().to_string());

    // 3. GET /roles
    let req = Request::builder().uri("/roles").body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let roles: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        !roles.as_array().expect("array").is_empty(),
        "list_roles must return rows inside tenant transaction"
    );

    // 4. GET /roles/{id}
    let req = Request::builder().uri(format!("/roles/{}", role_id.as_uuid())).body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let r: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(r["id"].as_str().unwrap(), role_id.as_uuid().to_string());

    // 5. GET /devices
    let req = Request::builder().uri("/devices").body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let devices: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        !devices.as_array().expect("array").is_empty(),
        "list_devices must return rows inside tenant transaction"
    );
}
