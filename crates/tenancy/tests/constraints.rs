use chrono::{Duration as ChronoDuration, Utc};
use sqlx::postgres::PgPoolOptions;
use std::fs;
use std::path::Path;
use std::time::Duration;
use uuid::Uuid;

const OWNER_URL: &str = "postgres://pos_owner:devpass@127.0.0.1:5432/medicine_pos";
const APP_URL: &str = "postgres://pos_app:devpass@127.0.0.1:5432/medicine_pos";

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

async fn app_pool() -> Option<sqlx::PgPool> {
    let url = std::env::var("APP_DATABASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| APP_URL.to_string());
    PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(500))
        .connect(&url)
        .await
        .ok()
}

fn read_migration_sql() -> String {
    let candidates = [
        "../migration/src/m0002_tenancy.rs",
        "crates/migration/src/m0002_tenancy.rs",
        "../../crates/migration/src/m0002_tenancy.rs",
    ];
    for c in &candidates {
        if let Ok(content) = fs::read_to_string(Path::new(c)) {
            return content;
        }
    }
    panic!("Could not locate m0002_tenancy.rs");
}

#[tokio::test]
async fn a_second_default_branch_is_rejected_by_the_database() {
    let ddl = read_migration_sql();
    assert!(
        ddl.contains("CREATE UNIQUE INDEX")
            && ddl.contains("idx_branches_tenant_default")
            && ddl.contains("(tenant_id) WHERE is_default"),
        "Migration must define partial unique index idx_branches_tenant_default on (tenant_id) WHERE is_default"
    );

    if let Some(pool) = owner_pool().await {
        let tenant_id = Uuid::now_v7();
        let _ = sqlx::query("INSERT INTO tenants (id, name, status) VALUES ($1, 'Test Pharmacy 1', 'active')")
            .bind(tenant_id)
            .execute(&pool)
            .await;

        let b1 = Uuid::now_v7();
        let b2 = Uuid::now_v7();

        let res1 = sqlx::query("INSERT INTO branches (id, tenant_id, name, is_default, is_active) VALUES ($1, $2, 'Branch 1', true, true)")
            .bind(b1)
            .bind(tenant_id)
            .execute(&pool)
            .await;
        assert!(res1.is_ok(), "First default branch must insert successfully");

        let res2 = sqlx::query("INSERT INTO branches (id, tenant_id, name, is_default, is_active) VALUES ($1, $2, 'Branch 2', true, true)")
            .bind(b2)
            .bind(tenant_id)
            .execute(&pool)
            .await;

        assert!(
            res2.is_err(),
            "Second default branch for same tenant must be rejected by partial unique index"
        );

        let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
            .bind(tenant_id)
            .execute(&pool)
            .await;
    }
}

#[tokio::test]
async fn two_tenants_may_both_have_a_branch_called_main() {
    let ddl = read_migration_sql();
    assert!(
        ddl.contains("UNIQUE (tenant_id, name)"),
        "Migration must scope branch name uniqueness to tenant_id: UNIQUE (tenant_id, name)"
    );

    if let Some(pool) = owner_pool().await {
        let t1 = Uuid::now_v7();
        let t2 = Uuid::now_v7();

        let _ = sqlx::query("INSERT INTO tenants (id, name, status) VALUES ($1, 'Pharmacy Alpha', 'active'), ($2, 'Pharmacy Beta', 'active')")
            .bind(t1)
            .bind(t2)
            .execute(&pool)
            .await;

        let b1 = Uuid::now_v7();
        let b2 = Uuid::now_v7();

        let res1 = sqlx::query("INSERT INTO branches (id, tenant_id, name, is_default, is_active) VALUES ($1, $2, 'Main', true, true)")
            .bind(b1)
            .bind(t1)
            .execute(&pool)
            .await;
        assert!(res1.is_ok(), "First pharmacy can have a branch named Main");

        let res2 = sqlx::query("INSERT INTO branches (id, tenant_id, name, is_default, is_active) VALUES ($1, $2, 'Main', true, true)")
            .bind(b2)
            .bind(t2)
            .execute(&pool)
            .await;
        assert!(res2.is_ok(), "Second pharmacy can also have a branch named Main");

        let _ = sqlx::query("DELETE FROM tenants WHERE id IN ($1, $2)")
            .bind(t1)
            .bind(t2)
            .execute(&pool)
            .await;
    }
}

#[tokio::test]
async fn overlapping_subscriptions_are_rejected_by_the_database() {
    let ddl = read_migration_sql();
    assert!(
        ddl.contains("ex_subscriptions_no_overlap")
            && ddl.contains("EXCLUDE USING gist")
            && ddl.contains("tstzrange(starts_at, ends_at)"),
        "Migration must define exclusion constraint ex_subscriptions_no_overlap using gist"
    );

    if let Some(pool) = owner_pool().await {
        let tenant_id = Uuid::now_v7();
        let plan_id = Uuid::parse_str("0192a000-0000-7000-8000-000000000001").unwrap();

        let _ = sqlx::query("INSERT INTO tenants (id, name, status) VALUES ($1, 'Sub Pharmacy', 'active')")
            .bind(tenant_id)
            .execute(&pool)
            .await;

        let s1 = Uuid::now_v7();
        let s2 = Uuid::now_v7();
        let now = Utc::now();
        let t1_start = now;
        let t1_end = now + ChronoDuration::days(30);

        let res1 = sqlx::query(
            r#"INSERT INTO subscriptions (id, tenant_id, plan_id, billing_period, price, starts_at, ends_at, status, is_complimentary)
               VALUES ($1, $2, $3, 'monthly', 1000.00, $4, $5, 'active', false)"#,
        )
        .bind(s1)
        .bind(tenant_id)
        .bind(plan_id)
        .bind(t1_start)
        .bind(t1_end)
        .execute(&pool)
        .await;
        assert!(res1.is_ok(), "Initial subscription should be accepted");

        // Overlapping date range
        let t2_start = now + ChronoDuration::days(15);
        let t2_end = now + ChronoDuration::days(45);

        let res2 = sqlx::query(
            r#"INSERT INTO subscriptions (id, tenant_id, plan_id, billing_period, price, starts_at, ends_at, status, is_complimentary)
               VALUES ($1, $2, $3, 'monthly', 1000.00, $4, $5, 'active', false)"#,
        )
        .bind(s2)
        .bind(tenant_id)
        .bind(plan_id)
        .bind(t2_start)
        .bind(t2_end)
        .execute(&pool)
        .await;

        assert!(
            res2.is_err(),
            "Overlapping active subscription must be rejected by exclusion constraint ex_subscriptions_no_overlap"
        );

        let _ = sqlx::query("DELETE FROM subscriptions WHERE tenant_id = $1")
            .bind(tenant_id)
            .execute(&pool)
            .await;
        let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
            .bind(tenant_id)
            .execute(&pool)
            .await;
    }
}

#[tokio::test]
async fn a_subscription_at_a_nonexistent_price_is_rejected() {
    let ddl = read_migration_sql();
    assert!(
        ddl.contains("fk_subscriptions_plan_price")
            && ddl.contains("FOREIGN KEY (plan_id, billing_period)")
            && ddl.contains("REFERENCES plan_prices(plan_id, billing_period)"),
        "Migration must define composite foreign key fk_subscriptions_plan_price"
    );

    if let Some(pool) = owner_pool().await {
        let tenant_id = Uuid::now_v7();
        let plan_id = Uuid::parse_str("0192a000-0000-7000-8000-000000000001").unwrap();

        let _ = sqlx::query("INSERT INTO tenants (id, name, status) VALUES ($1, 'Price Pharmacy', 'active')")
            .bind(tenant_id)
            .execute(&pool)
            .await;

        let s1 = Uuid::now_v7();
        let now = Utc::now();

        // Non-existent billing period 'decade'
        let res = sqlx::query(
            r#"INSERT INTO subscriptions (id, tenant_id, plan_id, billing_period, price, starts_at, ends_at, status, is_complimentary)
               VALUES ($1, $2, $3, 'decade', 9999.00, $4, $5, 'active', false)"#,
        )
        .bind(s1)
        .bind(tenant_id)
        .bind(plan_id)
        .bind(now)
        .bind(now + ChronoDuration::days(3650))
        .execute(&pool)
        .await;

        assert!(
            res.is_err(),
            "Subscription at non-existent plan price must fail foreign key constraint"
        );

        let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
            .bind(tenant_id)
            .execute(&pool)
            .await;
    }
}

#[tokio::test]
async fn a_payment_reference_cannot_be_reused() {
    let ddl = read_migration_sql();
    assert!(
        ddl.contains("idx_subscription_payments_method_ref")
            && ddl.contains("(method, reference) WHERE status <> 'rejected'"),
        "Migration must define partial unique index idx_subscription_payments_method_ref on (method, reference) WHERE status <> 'rejected'"
    );

    if let Some(pool) = owner_pool().await {
        let tenant_id = Uuid::now_v7();
        let _ = sqlx::query("INSERT INTO tenants (id, name, status) VALUES ($1, 'Pay Pharmacy', 'active')")
            .bind(tenant_id)
            .execute(&pool)
            .await;

        let p1 = Uuid::now_v7();
        let p2 = Uuid::now_v7();
        let reference = format!("BANK-{}", Uuid::now_v7());

        let res1 = sqlx::query(
            r#"INSERT INTO subscription_payments (id, tenant_id, amount, method, reference, status, paid_at)
               VALUES ($1, $2, 1000.00, 'bank_transfer', $3, 'submitted', now())"#,
        )
        .bind(p1)
        .bind(tenant_id)
        .bind(&reference)
        .execute(&pool)
        .await;
        assert!(res1.is_ok(), "First payment reference should succeed");

        let res2 = sqlx::query(
            r#"INSERT INTO subscription_payments (id, tenant_id, amount, method, reference, status, paid_at)
               VALUES ($1, $2, 1000.00, 'bank_transfer', $3, 'submitted', now())"#,
        )
        .bind(p2)
        .bind(tenant_id)
        .bind(&reference)
        .execute(&pool)
        .await;

        assert!(
            res2.is_err(),
            "Reusing reference across non-rejected payments must be rejected by unique index"
        );

        let _ = sqlx::query("DELETE FROM subscription_payments WHERE tenant_id = $1")
            .bind(tenant_id)
            .execute(&pool)
            .await;
        let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
            .bind(tenant_id)
            .execute(&pool)
            .await;
    }
}

#[tokio::test]
async fn a_purge_keeps_the_payment_row_and_nulls_its_tenant() {
    let ddl = read_migration_sql();
    assert!(
        ddl.contains("tenant_id UUID REFERENCES tenants(id) ON DELETE SET NULL"),
        "Migration must set subscription_payments.tenant_id to ON DELETE SET NULL"
    );

    if let Some(pool) = owner_pool().await {
        let tenant_id = Uuid::now_v7();
        let _ = sqlx::query("INSERT INTO tenants (id, name, status) VALUES ($1, 'Purge Pharmacy', 'active')")
            .bind(tenant_id)
            .execute(&pool)
            .await;

        let p1 = Uuid::now_v7();
        let reference = format!("PURGE-{}", Uuid::now_v7());
        let _ = sqlx::query(
            r#"INSERT INTO subscription_payments (id, tenant_id, amount, method, reference, status, paid_at)
               VALUES ($1, $2, 500.00, 'bank_transfer', $3, 'submitted', now())"#,
        )
        .bind(p1)
        .bind(tenant_id)
        .bind(&reference)
        .execute(&pool)
        .await;

        // Delete the tenant (simulating purge)
        let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
            .bind(tenant_id)
            .execute(&pool)
            .await;

        // Verify payment row still exists and tenant_id is NULL
        let payment_tenant_id: Option<Uuid> = sqlx::query_scalar("SELECT tenant_id FROM subscription_payments WHERE id = $1")
            .bind(p1)
            .fetch_one(&pool)
            .await
            .unwrap_or(Some(Uuid::nil()));

        assert!(
            payment_tenant_id.is_none(),
            "subscription_payments.tenant_id must be NULL after tenant is purged"
        );

        let _ = sqlx::query("DELETE FROM subscription_payments WHERE id = $1")
            .bind(p1)
            .execute(&pool)
            .await;
    }
}

#[tokio::test]
async fn app_role_cannot_delete_in_tenancy() {
    let ddl = read_migration_sql();
    assert!(
        !ddl.contains("GRANT DELETE ON") && !ddl.contains("GRANT ALL ON"),
        "Migration must never grant DELETE to pos_app"
    );

    if let Some(pool) = app_pool().await {
        let res = sqlx::query("DELETE FROM branches WHERE false")
            .execute(&pool)
            .await;
        assert!(
            res.is_err(),
            "pos_app must be refused DELETE privilege on tenancy tables"
        );
    }
}

#[tokio::test]
async fn the_seed_plan_has_a_row_for_every_limit_type() {
    let ddl = read_migration_sql();
    assert!(
        ddl.contains("INSERT INTO plans")
            && ddl.contains("INSERT INTO plan_limit_types")
            && ddl.contains("INSERT INTO plan_limits"),
        "Migration must contain seed statements for plan, limit types and plan limits"
    );

    if let Some(pool) = owner_pool().await {
        let plan_count: i64 = sqlx::query_scalar("SELECT count(*) FROM plans WHERE code = 'standard'")
            .fetch_one(&pool)
            .await
            .unwrap_or(0);
        assert_eq!(plan_count, 1, "Seed standard plan must exist in database");

        let limit_types_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM plan_limit_types WHERE key IN ('branches', 'users', 'devices')",
        )
        .fetch_one(&pool)
        .await
        .unwrap_or(0);
        assert_eq!(
            limit_types_count, 3,
            "Must have seed limit types for branches, users, and devices"
        );

        let limits_count: i64 = sqlx::query_scalar(
            r#"SELECT count(*) FROM plan_limits
               WHERE plan_id = '0192a000-0000-7000-8000-000000000001' AND max_value IS NULL"#,
        )
        .fetch_one(&pool)
        .await
        .unwrap_or(0);
        assert_eq!(
            limits_count, 3,
            "Seed plan must have every limit type set to unlimited (NULL)"
        );
    }
}

#[tokio::test]
async fn a_payment_cannot_name_another_tenants_subscription() {
    let owner = owner_pool().await.expect("owner database required");

    let tenant_a = Uuid::now_v7();
    let tenant_b = Uuid::now_v7();

    sqlx::query("INSERT INTO tenants (id, name, status) VALUES ($1, 'Tenant A', 'active'), ($2, 'Tenant B', 'active')")
        .bind(tenant_a)
        .bind(tenant_b)
        .execute(&owner)
        .await
        .expect("insert tenants");

    let plan_id = Uuid::parse_str("0192a000-0000-7000-8000-000000000001").unwrap();
    let sub_b = Uuid::now_v7();
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO subscriptions (id, tenant_id, plan_id, billing_period, price, starts_at, ends_at, status)
         VALUES ($1, $2, $3, 'monthly', 1000.00, $4, $5, 'active')"
    )
    .bind(sub_b)
    .bind(tenant_b)
    .bind(plan_id)
    .bind(now)
    .bind(now + ChronoDuration::days(30))
    .execute(&owner)
    .await
    .expect("insert sub B");

    // Tenant A attempts to insert a payment naming Tenant B's subscription
    let payment_id = Uuid::now_v7();
    let ref_code = format!("REF-{}", Uuid::now_v7());
    let res = sqlx::query(
        "INSERT INTO subscription_payments (id, tenant_id, subscription_id, amount, method, reference, status)
         VALUES ($1, $2, $3, 1000.00, 'bank_transfer', $4, 'submitted')"
    )
    .bind(payment_id)
    .bind(tenant_a)
    .bind(sub_b)
    .bind(&ref_code)
    .execute(&owner)
    .await;

    assert!(
        res.is_err(),
        "Database must reject payment referencing another tenant's subscription via composite FK"
    );
    let err_msg = res.unwrap_err().to_string();
    assert!(
        err_msg.contains("fk_subscription_payments_subscription_composite") || err_msg.contains("foreign key"),
        "Error must be a foreign key constraint violation on composite key: {err_msg}"
    );

    // Clean up
    let _ = sqlx::query("DELETE FROM subscriptions WHERE tenant_id = $1").bind(tenant_b).execute(&owner).await;
    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1 OR id = $2").bind(tenant_a).bind(tenant_b).execute(&owner).await;
}

#[tokio::test]
async fn pos_app_cannot_update_a_payment_or_insert_a_subscription() {
    let owner = owner_pool().await.expect("owner database required");
    let app = app_pool().await.expect("app database required");

    let tenant_id = Uuid::now_v7();
    sqlx::query("INSERT INTO tenants (id, name, status) VALUES ($1, 'RLS Test Tenant', 'active')")
        .bind(tenant_id)
        .execute(&owner)
        .await
        .expect("insert tenant");

    let plan_id = Uuid::parse_str("0192a000-0000-7000-8000-000000000001").unwrap();
    let payment_id = Uuid::now_v7();
    let ref_code = format!("REF-{}", Uuid::now_v7());

    // 1. pos_app acting as tenant: insert submitted payment (allowed by subscription_payments_tenant_insert)
    let mut app_tx = app.begin().await.expect("begin app tx");
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *app_tx)
        .await
        .expect("set tenant");

    sqlx::query(
        "INSERT INTO subscription_payments (id, tenant_id, amount, method, reference, status)
         VALUES ($1, $2, 1000.00, 'bank_transfer', $3, 'submitted')"
    )
    .bind(payment_id)
    .bind(tenant_id)
    .bind(&ref_code)
    .execute(&mut *app_tx)
    .await
    .expect("tenant can insert submitted payment");

    // 2. pos_app acting as tenant attempts to UPDATE payment to 'confirmed' -> blocked by RLS (0 rows affected)
    let update_res = sqlx::query(
        "UPDATE subscription_payments SET status = 'confirmed' WHERE id = $1 RETURNING id"
    )
    .bind(payment_id)
    .fetch_optional(&mut *app_tx)
    .await
    .expect("update query executes under RLS");

    assert!(
        update_res.is_none(),
        "pos_app without platform token cannot update payment (0 rows modified under RLS)"
    );

    // 3. pos_app acting as tenant attempts to INSERT a subscription -> blocked by RLS policy
    let sub_id = Uuid::now_v7();
    let now = Utc::now();
    let sub_insert_res = sqlx::query(
        "INSERT INTO subscriptions (id, tenant_id, plan_id, billing_period, price, starts_at, ends_at, status)
         VALUES ($1, $2, $3, 'monthly', 1000.00, $4, $5, 'active')"
    )
    .bind(sub_id)
    .bind(tenant_id)
    .bind(plan_id)
    .bind(now)
    .bind(now + ChronoDuration::days(30))
    .execute(&mut *app_tx)
    .await;

    assert!(
        sub_insert_res.is_err(),
        "pos_app without platform privileges cannot insert a subscription"
    );
    let sub_err = sub_insert_res.unwrap_err().to_string();
    assert!(
        sub_err.contains("row-level security") || sub_err.contains("policy"),
        "Error must be an RLS policy violation: {sub_err}"
    );

    app_tx.rollback().await.expect("rollback");

    // Clean up
    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1").bind(tenant_id).execute(&owner).await;
}

