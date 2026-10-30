use cache::Cache;
use db::{Db, Tx};
use pos_core::context::TenantContext;
use pos_core::error::ErrorCode;
use pos_core::ids::{BranchId, DeviceId, RoleId, TenantId, UserId};
use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

use identity::commands;
use identity::queries;

const APP_URL: &str = "postgres://pos_app:devpass@127.0.0.1:5432/medicine_pos";
const OWNER_URL: &str = "postgres://pos_owner:devpass@127.0.0.1:5432/medicine_pos";

async fn get_db() -> Option<Db> {
    let url = std::env::var("APP_DATABASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| APP_URL.to_string());
    let conn = Database::connect(&url).await.ok()?;
    Some(Db::from_connection(conn, url))
}

async fn owner_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| OWNER_URL.to_string());
    PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_millis(1000))
        .connect(&url)
        .await
        .ok()
}

fn get_cache(prefix: &str) -> Cache {
    Cache::in_memory(prefix)
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

async fn seed_branch(pool: &PgPool, tenant_id: TenantId, branch_id: BranchId, name: &str) {
    seed_tenant(pool, tenant_id, "Test Pharmacy").await;
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
async fn the_suite_refuses_to_pass_without_a_database() {
    use sea_orm::ConnectOptions;
    let mut opt = ConnectOptions::new("postgres://pos_app:devpass@127.0.0.1:54329/nonexistent");
    opt.connect_timeout(Duration::from_millis(200));
    let conn = Database::connect(opt).await;
    assert!(
        conn.is_err(),
        "Attempting to connect without a database must return an error and fail the test"
    );
}

#[tokio::test]
async fn rls_blocks_cross_tenant_read() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner pool required");

    let tenant_a = TenantId::new_v7();
    let tenant_b = TenantId::new_v7();
    let user_a = UserId::new_v7();
    let user_b = UserId::new_v7();
    let branch_b = BranchId::new_v7();

    seed_tenant(&owner, tenant_a, "Tenant A Pharmacy").await;
    seed_tenant(&owner, tenant_b, "Tenant B Pharmacy").await;
    seed_branch(&owner, tenant_b, branch_b, "Branch B").await;

    // Seed user_b in tenant_b
    let _ = sqlx::query(
        "INSERT INTO users (id, tenant_id, name, email, password_hash, status, created_at, updated_at)
         VALUES ($1, $2, 'User B', $3, 'hash_b', 'active', now(), now())",
    )
    .bind(user_b.as_uuid())
    .bind(tenant_b.as_uuid())
    .bind(format!("user_b_{}@test.com", Uuid::now_v7()))
    .execute(&owner)
    .await
    .expect("seed user b");

    // Seed role_b in tenant_b
    let role_b_id = Uuid::now_v7();
    let _ = sqlx::query(
        "INSERT INTO roles (id, tenant_id, name, is_system, is_owner, created_at, updated_at)
         VALUES ($1, $2, 'Role B', false, false, now(), now())",
    )
    .bind(role_b_id)
    .bind(tenant_b.as_uuid())
    .execute(&owner)
    .await
    .expect("seed role b");

    // Seed device_b in tenant_b
    let device_b_id = Uuid::now_v7();
    let _ = sqlx::query(
        "INSERT INTO devices (id, tenant_id, branch_id, name, invoice_prefix, status, platform, created_at, updated_at)
         VALUES ($1, $2, $3, 'Device B', $4, 'active', 'android', now(), now())",
    )
    .bind(device_b_id)
    .bind(tenant_b.as_uuid())
    .bind(branch_b.as_uuid())
    .bind(format!("B{}", &Uuid::now_v7().to_string()[..6]))
    .execute(&owner)
    .await
    .expect("seed device b");

    // Seed session_b in tenant_b
    let session_b_id = Uuid::now_v7();
    let _ = sqlx::query(
        "INSERT INTO sessions (id, tenant_id, user_id, device_id, refresh_token_hash, expires_at, created_at)
         VALUES ($1, $2, $3, $4, $5, now() + interval '1 day', now())",
    )
    .bind(session_b_id)
    .bind(tenant_b.as_uuid())
    .bind(user_b.as_uuid())
    .bind(device_b_id)
    .bind(format!("hash_{}", Uuid::now_v7()))
    .execute(&owner)
    .await
    .expect("seed session b");

    // Acting as Tenant A inside an RLS-enforced transaction:
    let ctx_a = TenantContext::new(tenant_a, user_a, vec![]);
    let tx = Tx::begin(db.conn(), &ctx_a).await.expect("begin tx A");
    let tx_conn = tx.transaction().expect("tx conn");

    // 1. Users: Tenant A sees 0 users of Tenant B
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT count(*) FROM users WHERE tenant_id = $1",
        [tenant_b.as_uuid().into()],
    );
    let row = tx_conn.query_one_raw(stmt).await.expect("query users B").expect("row");
    let users_count: i64 = row.try_get_by_index(0).expect("count");
    assert_eq!(users_count, 0, "RLS must hide Tenant B users from Tenant A");

    // 2. Roles: Tenant A sees 0 roles of Tenant B
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT count(*) FROM roles WHERE tenant_id = $1",
        [tenant_b.as_uuid().into()],
    );
    let row = tx_conn.query_one_raw(stmt).await.expect("query roles B").expect("row");
    let roles_count: i64 = row.try_get_by_index(0).expect("count");
    assert_eq!(roles_count, 0, "RLS must hide Tenant B roles from Tenant A");

    // 3. Devices: Tenant A sees 0 devices of Tenant B
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT count(*) FROM devices WHERE tenant_id = $1",
        [tenant_b.as_uuid().into()],
    );
    let row = tx_conn.query_one_raw(stmt).await.expect("query devices B").expect("row");
    let devices_count: i64 = row.try_get_by_index(0).expect("count");
    assert_eq!(devices_count, 0, "RLS must hide Tenant B devices from Tenant A");

    // 4. Sessions: Tenant A sees 0 sessions of Tenant B
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT count(*) FROM sessions WHERE tenant_id = $1",
        [tenant_b.as_uuid().into()],
    );
    let row = tx_conn.query_one_raw(stmt).await.expect("query sessions B").expect("row");
    let sessions_count: i64 = row.try_get_by_index(0).expect("count");
    assert_eq!(sessions_count, 0, "RLS must hide Tenant B sessions from Tenant A");

    // 5. Query helpers return None for Tenant B resources
    let user_res = queries::get_user(tx_conn, tenant_a, user_b)
        .await
        .expect("get_user");
    assert!(user_res.is_none(), "Tenant B user must not be readable by Tenant A");

    let role_res = queries::get_role(tx_conn, tenant_a, RoleId::from(role_b_id))
        .await
        .expect("get_role");
    assert!(role_res.is_none(), "Tenant B role must not be readable by Tenant A");

    tx.commit().await.expect("commit tx A");

    // Cleanup
    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1 OR id = $2")
        .bind(tenant_a.as_uuid())
        .bind(tenant_b.as_uuid())
        .execute(&owner)
        .await;
}

#[tokio::test]
async fn rls_blocks_cross_tenant_write() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner pool required");

    let tenant_a = TenantId::new_v7();
    let tenant_b = TenantId::new_v7();
    let user_a = UserId::new_v7();

    seed_tenant(&owner, tenant_a, "Tenant A Pharmacy").await;
    seed_tenant(&owner, tenant_b, "Tenant B Pharmacy").await;

    let ctx_a = TenantContext::new(tenant_a, user_a, vec![]);
    let tx = Tx::begin(db.conn(), &ctx_a).await.expect("begin tx A");
    let tx_conn = tx.transaction().expect("tx conn");

    // Attempting to INSERT a user naming Tenant B's id while session is Tenant A
    let bad_user_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"INSERT INTO users (id, tenant_id, name, email, password_hash, status, created_at, updated_at)
           VALUES ($1, $2, 'Illegal User', $3, 'some_hash', 'active', now(), now())"#,
        [
            Uuid::now_v7().into(),
            tenant_b.as_uuid().into(),
            format!("illegal_{}@test.com", Uuid::now_v7()).into(),
        ],
    );

    let res_user = tx_conn.execute_raw(bad_user_stmt).await;
    assert!(
        res_user.is_err(),
        "RLS policy WITH CHECK must reject inserting a user for another tenant"
    );
    let err_str = res_user.unwrap_err().to_string();
    assert!(
        err_str.contains("row-level security") || err_str.contains("policy"),
        "Error must be an RLS policy violation: {err_str}"
    );

    // Attempting to INSERT a role naming Tenant B's id while session is Tenant A
    let bad_role_stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        r#"INSERT INTO roles (id, tenant_id, name, is_system, is_owner, created_at, updated_at)
           VALUES ($1, $2, 'Illegal Role', false, false, now(), now())"#,
        [
            Uuid::now_v7().into(),
            tenant_b.as_uuid().into(),
        ],
    );

    let res_role = tx_conn.execute_raw(bad_role_stmt).await;
    assert!(
        res_role.is_err(),
        "RLS policy WITH CHECK must reject inserting a role for another tenant"
    );

    tx.rollback().await.expect("rollback");

    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1 OR id = $2")
        .bind(tenant_a.as_uuid())
        .bind(tenant_b.as_uuid())
        .execute(&owner)
        .await;
}

#[tokio::test]
async fn a_role_cannot_be_granted_to_another_tenants_user() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner pool required");
    let cache = get_cache("test_isolation_role");

    let tenant_a = TenantId::new_v7();
    let tenant_b = TenantId::new_v7();
    let user_a = UserId::new_v7();
    let user_b = UserId::new_v7();
    let branch_a = BranchId::new_v7();
    let branch_b = BranchId::new_v7();

    seed_tenant(&owner, tenant_a, "Tenant A Pharmacy").await;
    seed_tenant(&owner, tenant_b, "Tenant B Pharmacy").await;
    seed_branch(&owner, tenant_a, branch_a, "Branch A").await;
    seed_branch(&owner, tenant_b, branch_b, "Branch B").await;

    // Seed user_a in tenant_a
    let _ = sqlx::query(
        "INSERT INTO users (id, tenant_id, name, email, password_hash, status, created_at, updated_at)
         VALUES ($1, $2, 'User A', $3, 'hash_a', 'active', now(), now())",
    )
    .bind(user_a.as_uuid())
    .bind(tenant_a.as_uuid())
    .bind(format!("user_a_{}@test.com", Uuid::now_v7()))
    .execute(&owner)
    .await
    .expect("seed user a");

    // Seed user_b in tenant_b
    let _ = sqlx::query(
        "INSERT INTO users (id, tenant_id, name, email, password_hash, status, created_at, updated_at)
         VALUES ($1, $2, 'User B', $3, 'hash_b', 'active', now(), now())",
    )
    .bind(user_b.as_uuid())
    .bind(tenant_b.as_uuid())
    .bind(format!("user_b_{}@test.com", Uuid::now_v7()))
    .execute(&owner)
    .await
    .expect("seed user b");

    // Seed role_a in tenant_a
    let role_a_id = RoleId::from_uuid(Uuid::now_v7());
    let _ = sqlx::query(
        "INSERT INTO roles (id, tenant_id, name, is_system, is_owner, created_at, updated_at)
         VALUES ($1, $2, 'Role A', false, false, now(), now())",
    )
    .bind(role_a_id.as_uuid())
    .bind(tenant_a.as_uuid())
    .execute(&owner)
    .await
    .expect("seed role a");

    // Seed role_b in tenant_b
    let role_b_id = RoleId::from_uuid(Uuid::now_v7());
    let _ = sqlx::query(
        "INSERT INTO roles (id, tenant_id, name, is_system, is_owner, created_at, updated_at)
         VALUES ($1, $2, 'Role B', false, false, now(), now())",
    )
    .bind(role_b_id.as_uuid())
    .bind(tenant_b.as_uuid())
    .execute(&owner)
    .await
    .expect("seed role b");

    // Acting as Tenant A:
    let ctx_a = TenantContext::new(tenant_a, user_a, vec![branch_a]);
    let tx = Tx::begin(db.conn(), &ctx_a).await.expect("begin tx A");

    // 1. Granting role_a to user_b (Tenant B's user) must fail with NotFound
    let res_cross_user = commands::grant_role(&tx, &cache, user_b, role_a_id, None).await;
    assert!(res_cross_user.is_err(), "Cannot grant role to another tenant's user");
    let err_u = res_cross_user.unwrap_err();
    assert_eq!(err_u.code, ErrorCode::NotFound);
    assert_eq!(err_u.code.http_status(), 404);

    // 2. Granting role_b (Tenant B's role) to user_a must fail with NotFound
    let res_cross_role = commands::grant_role(&tx, &cache, user_a, role_b_id, None).await;
    assert!(res_cross_role.is_err(), "Cannot grant another tenant's role");
    let err_r = res_cross_role.unwrap_err();
    assert_eq!(err_r.code, ErrorCode::NotFound);
    assert_eq!(err_r.code.http_status(), 404);

    // 3. Granting role_a at branch_b (Tenant B's branch) must fail with NotFound
    let res_cross_branch = commands::grant_role(&tx, &cache, user_a, role_a_id, Some(branch_b)).await;
    assert!(res_cross_branch.is_err(), "Cannot grant role scoped to another tenant's branch");
    let err_b = res_cross_branch.unwrap_err();
    assert_eq!(err_b.code, ErrorCode::NotFound);
    assert_eq!(err_b.code.http_status(), 404);

    tx.rollback().await.expect("rollback");

    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1 OR id = $2")
        .bind(tenant_a.as_uuid())
        .bind(tenant_b.as_uuid())
        .execute(&owner)
        .await;
}

#[tokio::test]
async fn another_pharmacys_user_is_a_404() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner pool required");
    let cache = get_cache("test_isolation_404");

    let tenant_a = TenantId::new_v7();
    let tenant_b = TenantId::new_v7();
    let user_a = UserId::new_v7();
    let user_b = UserId::new_v7();
    let branch_b = BranchId::new_v7();

    seed_tenant(&owner, tenant_a, "Tenant A Pharmacy").await;
    seed_tenant(&owner, tenant_b, "Tenant B Pharmacy").await;
    seed_branch(&owner, tenant_b, branch_b, "Branch B").await;

    // Seed user_b in tenant_b
    let _ = sqlx::query(
        "INSERT INTO users (id, tenant_id, name, email, password_hash, status, created_at, updated_at)
         VALUES ($1, $2, 'User B', $3, 'hash_b', 'active', now(), now())",
    )
    .bind(user_b.as_uuid())
    .bind(tenant_b.as_uuid())
    .bind(format!("user_b_404_{}@test.com", Uuid::now_v7()))
    .execute(&owner)
    .await
    .expect("seed user b");

    // Seed role_b in tenant_b
    let role_b_id = RoleId::from_uuid(Uuid::now_v7());
    let _ = sqlx::query(
        "INSERT INTO roles (id, tenant_id, name, is_system, is_owner, created_at, updated_at)
         VALUES ($1, $2, 'Role B', false, false, now(), now())",
    )
    .bind(role_b_id.as_uuid())
    .bind(tenant_b.as_uuid())
    .execute(&owner)
    .await
    .expect("seed role b");

    // Seed device_b in tenant_b
    let device_b_id = DeviceId::from_uuid(Uuid::now_v7());
    let _ = sqlx::query(
        "INSERT INTO devices (id, tenant_id, branch_id, name, invoice_prefix, status, platform, created_at, updated_at)
         VALUES ($1, $2, $3, 'Device B', $4, 'active', 'android', now(), now())",
    )
    .bind(device_b_id.as_uuid())
    .bind(tenant_b.as_uuid())
    .bind(branch_b.as_uuid())
    .bind(format!("D{}", &Uuid::now_v7().to_string()[..6]))
    .execute(&owner)
    .await
    .expect("seed device b");

    // Acting as Tenant A:
    let ctx_a = TenantContext::new(tenant_a, user_a, vec![]);
    let tx = Tx::begin(db.conn(), &ctx_a).await.expect("begin tx A");
    let tx_conn = tx.transaction().expect("tx conn");

    // 1. queries::get_user returns None (which REST handler turns into 404, never 403)
    let user_opt = queries::get_user(tx_conn, tenant_a, user_b).await.expect("get_user");
    assert!(user_opt.is_none(), "Tenant B's user must return None to Tenant A");

    // 2. commands::update_user returns 404 NotFound, never 403
    let err_update = commands::update_user(&tx, user_b, Some("New Name"), None)
        .await
        .expect_err("update user B should fail");
    assert_eq!(err_update.code, ErrorCode::NotFound, "Must be NotFound, never Forbidden");
    assert_eq!(err_update.code.http_status(), 404);

    // 3. commands::disable_user returns 404 NotFound, never 403
    let err_disable = commands::disable_user(&tx, &cache, user_b)
        .await
        .expect_err("disable user B should fail");
    assert_eq!(err_disable.code, ErrorCode::NotFound, "Must be NotFound, never Forbidden");
    assert_eq!(err_disable.code.http_status(), 404);

    // 4. commands::set_user_pin returns 404 NotFound, never 403
    let err_pin = commands::set_user_pin(&tx, user_b, "1234")
        .await
        .expect_err("set user pin for B should fail");
    assert_eq!(err_pin.code, ErrorCode::NotFound, "Must be NotFound, never Forbidden");
    assert_eq!(err_pin.code.http_status(), 404);

    // 5. commands::revoke_device returns 404 NotFound, never 403
    let err_dev = commands::revoke_device(&tx, &cache, device_b_id)
        .await
        .expect_err("revoke device B should fail");
    assert_eq!(err_dev.code, ErrorCode::NotFound, "Must be NotFound, never Forbidden");
    assert_eq!(err_dev.code.http_status(), 404);

    // 6. commands::delete_role returns 404 NotFound, never 403
    let err_role = commands::delete_role(&tx, role_b_id)
        .await
        .expect_err("delete role B should fail");
    assert_eq!(err_role.code, ErrorCode::NotFound, "Must be NotFound, never Forbidden");
    assert_eq!(err_role.code.http_status(), 404);

    tx.rollback().await.expect("rollback");

    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1 OR id = $2")
        .bind(tenant_a.as_uuid())
        .bind(tenant_b.as_uuid())
        .execute(&owner)
        .await;
}



#[tokio::test]
async fn app_is_platform_reads_across_tenants_in_identity() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner pool required");

    let tenant_a = TenantId::new_v7();
    let tenant_b = TenantId::new_v7();
    let platform_user = UserId::new_v7();
    let user_b = UserId::new_v7();

    seed_tenant(&owner, tenant_a, "Tenant A Pharmacy").await;
    seed_tenant(&owner, tenant_b, "Tenant B Pharmacy").await;

    let _ = sqlx::query(
        "INSERT INTO users (id, tenant_id, name, email, password_hash, status)
         VALUES ($1, $2, 'User B', $3, 'hash_b', 'active')",
    )
    .bind(user_b.as_uuid())
    .bind(tenant_b.as_uuid())
    .bind(format!("plat_read_{}@test.com", Uuid::now_v7()))
    .execute(&owner)
    .await;

    // In a platform transaction (which sets app.is_platform = true):
    let platform_ctx = TenantContext::new(tenant_a, platform_user, vec![]);
    let platform_tx = Tx::begin_platform(db.conn(), &platform_ctx)
        .await
        .expect("begin platform tx");
    let tx_conn = platform_tx.transaction().expect("tx conn");

    // Platform CAN read Tenant B's users even though ctx has tenant_a
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT count(*) FROM users WHERE tenant_id = $1",
        [tenant_b.as_uuid().into()],
    );
    let row = tx_conn.query_one_raw(stmt).await.expect("platform query users B").expect("row");
    let b_users: i64 = row.try_get_by_index(0).expect("count");
    assert_eq!(
        b_users, 1,
        "app.is_platform = true must read across tenants on identity tables"
    );

    platform_tx.commit().await.expect("commit platform tx");

    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1 OR id = $2")
        .bind(tenant_a.as_uuid())
        .bind(tenant_b.as_uuid())
        .execute(&owner)
        .await;
}
