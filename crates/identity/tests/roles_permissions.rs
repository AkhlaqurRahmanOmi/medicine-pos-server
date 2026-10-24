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

#[tokio::test]
async fn a_new_pharmacy_gets_its_roles_from_the_templates() {
    let db = match get_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };

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
    let db = match get_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };
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
    let db = match get_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };

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
    let db = match get_db().await {
        Some(d) => d,
        None => return,
    };

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
    let db = match get_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    seed_user(&owner, tenant_id, user_id, &format!("owner_perms_{}@test.com", Uuid::now_v7()), "active").await;

    let ctx = TenantContext::new(tenant_id, user_id, vec![]);
    let tx = Tx::begin_platform(&db, &ctx).await.unwrap();
    let (owner_role_id, _, _) = seed_roles_for_tenant(&tx).await.unwrap();
    tx.commit().await.unwrap();

    let assignable = assignable_permissions(&db).await.unwrap();
    let owner_perms = role_permissions(&db, tenant_id, owner_role_id).await.unwrap();

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
    let db = match get_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };
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
    let db = match get_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };
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
    let b1_perms = permissions_at(&db, tenant_id, user_id, b1_id).await.unwrap();
    assert!(b1_perms.contains(&"user.view".to_string()));
    assert!(b1_perms.contains(&"device.view".to_string()));
    assert!(!b1_perms.contains(&"device.register".to_string()));

    // Permissions at B2: union of Wide ("user.view") and B2 ("device.register")
    let b2_perms = permissions_at(&db, tenant_id, user_id, b2_id).await.unwrap();
    assert!(b2_perms.contains(&"user.view".to_string()));
    assert!(!b2_perms.contains(&"device.view".to_string()));
    assert!(b2_perms.contains(&"device.register".to_string()));
}

#[tokio::test]
async fn a_null_branch_grant_includes_a_branch_created_later() {
    let db = match get_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };
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

    // Initial check: has access to b1
    let branches_initial = branch_access(&db, tenant_id, user_id).await.unwrap();
    assert!(branches_initial.contains(&b1_id));
    assert!(!branches_initial.contains(&b2_id));

    // Now, create branch 2 LATER
    seed_branch(&owner, tenant_id, b2_id, "New Branch 2 Created Later").await;

    // Second check: automatically includes branch 2 without re-granting!
    let branches_after = branch_access(&db, tenant_id, user_id).await.unwrap();
    assert!(branches_after.contains(&b1_id));
    assert!(branches_after.contains(&b2_id));
}

#[tokio::test]
async fn granting_a_role_clears_the_cached_branch_access() {
    let db = match get_db().await {
        Some(d) => d,
        None => return,
    };
    let owner = match owner_pool().await {
        Some(o) => o,
        None => return,
    };
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
    let db = match get_db().await {
        Some(d) => d,
        None => return,
    };

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
