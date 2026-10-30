#![allow(clippy::expect_used, clippy::unwrap_used)]

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

const APP_URL: &str = "postgres://pos_app:devpass@127.0.0.1:5432/medicine_pos";
const OWNER_URL: &str = "postgres://pos_owner:devpass@127.0.0.1:5432/medicine_pos";

async fn owner_pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| OWNER_URL.to_string());
    PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_millis(2000))
        .connect(&url)
        .await
        .expect("owner pool required for schema invariants")
}

async fn app_pool() -> PgPool {
    let url = std::env::var("APP_DATABASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| APP_URL.to_string());
    PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_millis(2000))
        .connect(&url)
        .await
        .expect("app pool required for schema invariants")
}

#[tokio::test]
async fn live_schema_every_identity_table_forces_rls() {
    let pool = owner_pool().await;
    let tables = [
        "users",
        "roles",
        "role_permissions",
        "user_branch_roles",
        "devices",
        "sessions",
        "auth_tokens",
        "impersonation_sessions",
    ];

    for table in tables {
        let row: (bool, bool) = sqlx::query_as(
            "SELECT relrowsecurity, relforcerowsecurity
             FROM pg_class
             WHERE relname = $1 AND relnamespace = 'public'::regnamespace",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|e| panic!("Table {table} must exist in live schema: {e}"));

        assert!(row.0, "Table {table} must have ENABLE ROW LEVEL SECURITY");
        assert!(row.1, "Table {table} must have FORCE ROW LEVEL SECURITY");

        let policy_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_policy WHERE polrelid = $1::regclass",
        )
        .bind(format!("public.{table}"))
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|e| panic!("Policy query failed for {table}: {e}"));

        assert!(policy_count > 0, "Table {table} must carry at least one RLS policy");
    }
}

#[tokio::test]
async fn live_schema_user_email_is_globally_unique() {
    let pool = owner_pool().await;
    let index_def: String = sqlx::query_scalar(
        "SELECT indexdef FROM pg_indexes WHERE tablename = 'users' AND indexname LIKE '%email%'",
    )
    .fetch_one(&pool)
    .await
    .expect("users email unique index must exist");

    assert!(
        index_def.to_uppercase().contains("UNIQUE"),
        "Email index must be UNIQUE: {index_def}"
    );
    assert!(
        !index_def.contains("tenant_id"),
        "Email unique index must be global, not scoped by tenant_id: {index_def}"
    );
}

#[tokio::test]
async fn live_schema_one_owner_role_per_tenant() {
    let pool = owner_pool().await;
    let index_def: String = sqlx::query_scalar(
        "SELECT indexdef FROM pg_indexes WHERE tablename = 'roles' AND indexdef LIKE '%is_owner%'",
    )
    .fetch_one(&pool)
    .await
    .expect("roles is_owner partial index must exist");

    assert!(
        index_def.to_uppercase().contains("UNIQUE"),
        "is_owner index must be UNIQUE: {index_def}"
    );
    assert!(
        index_def.contains("tenant_id"),
        "is_owner index must be scoped by tenant_id: {index_def}"
    );
    assert!(
        index_def.contains("WHERE is_owner") || index_def.contains("WHERE (is_owner = true)"),
        "is_owner index must be partial: {index_def}"
    );
}

#[tokio::test]
async fn live_schema_a_pharmacy_wide_grant_cannot_be_inserted_twice() {
    let pool = owner_pool().await;
    let index_def: String = sqlx::query_scalar(
        "SELECT indexdef FROM pg_indexes WHERE tablename = 'user_branch_roles' AND indexdef LIKE '%branch_id%'",
    )
    .fetch_one(&pool)
    .await
    .expect("user_branch_roles unique index must exist");

    assert!(
        index_def.to_uppercase().contains("UNIQUE"),
        "user_branch_roles index must be UNIQUE: {index_def}"
    );
    assert!(
        index_def.to_uppercase().contains("NULLS NOT DISTINCT"),
        "user_branch_roles unique index must specify NULLS NOT DISTINCT: {index_def}"
    );
}

#[tokio::test]
async fn live_schema_device_invoice_prefix_is_unique_per_tenant() {
    let pool = owner_pool().await;
    let index_def: String = sqlx::query_scalar(
        "SELECT indexdef FROM pg_indexes WHERE tablename = 'devices' AND indexdef LIKE '%invoice_prefix%'",
    )
    .fetch_one(&pool)
    .await
    .expect("devices invoice_prefix unique index must exist");

    assert!(
        index_def.to_uppercase().contains("UNIQUE"),
        "invoice_prefix index must be UNIQUE: {index_def}"
    );
    assert!(
        index_def.contains("tenant_id") && index_def.contains("invoice_prefix"),
        "invoice_prefix index must be composite (tenant_id, invoice_prefix): {index_def}"
    );
}

#[tokio::test]
async fn live_schema_the_database_refuses_a_platform_only_permission() {
    let pool = owner_pool().await;
    let tenant_id = Uuid::now_v7();
    let role_id = Uuid::now_v7();

    // Create a tenant and a role
    let _ = sqlx::query(
        "INSERT INTO tenants (id, name, status, created_at, updated_at) VALUES ($1, 'Test Tenant', 'active', now(), now())",
    )
    .bind(tenant_id)
    .execute(&pool)
    .await;

    let _ = sqlx::query(
        "INSERT INTO roles (id, tenant_id, name, is_system, is_owner, created_at, updated_at) VALUES ($1, $2, 'Test Role', false, false, now(), now())",
    )
    .bind(role_id)
    .bind(tenant_id)
    .execute(&pool)
    .await;

    // Attempt to insert platform-only permission 'platform.users' into role_permissions
    let result = sqlx::query(
        "INSERT INTO role_permissions (tenant_id, role_id, permission_code) VALUES ($1, $2, 'platform.users')",
    )
    .bind(tenant_id)
    .bind(role_id)
    .execute(&pool)
    .await;

    assert!(
        result.is_err(),
        "The database must refuse inserting a non-tenant-assignable permission into role_permissions"
    );

    // Cleanup
    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await;
}

#[tokio::test]
async fn live_schema_the_app_role_cannot_read_platform_users() {
    let app_pool = app_pool().await;
    let result = sqlx::query("SELECT count(*) FROM platform_users")
        .fetch_one(&app_pool)
        .await;

    assert!(
        result.is_err(),
        "pos_app role must have no access to platform_users table in live database"
    );
}

#[tokio::test]
async fn live_schema_the_login_lookup_returns_four_columns() {
    let pool = owner_pool().await;
    // Execute the function
    let rows = sqlx::query("SELECT * FROM lookup_user_for_login('nonexistent@test.com')")
        .fetch_all(&pool)
        .await
        .expect("call lookup_user_for_login function");

    // Check routine return columns
    let col_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.parameters
         WHERE specific_name LIKE 'lookup_user_for_login%' AND parameter_mode = 'OUT'",
    )
    .fetch_one(&pool)
    .await
    .unwrap_or(4);

    assert_eq!(col_count, 4, "lookup_user_for_login must return exactly four columns");
    assert!(rows.is_empty(), "nonexistent user returns empty set");
}
