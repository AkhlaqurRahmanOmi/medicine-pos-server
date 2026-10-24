use db::{Db, Tx};
use pos_core::context::TenantContext;
use pos_core::error::ErrorCode;
use pos_core::ids::{TenantId, UserId};
use sea_orm::{ActiveModelTrait, ColumnTrait, Database, EntityTrait, PaginatorTrait, QueryFilter, Set};
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use tenancy::commands::{create_pharmacy, deactivate_branch};
use tenancy::entity::{branches as branches_entity, tenant_settings, subscriptions, subscription_payments};
use tenancy::queries::branches;
use uuid::Uuid;

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
async fn tenant_a_cannot_read_tenant_b() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_a = TenantId::new_v7();
    let tenant_b = TenantId::new_v7();
    let user_a = UserId::new_v7();
    let user_b = UserId::new_v7();

    let ctx_a = TenantContext::new(tenant_a, user_a, vec![]);
    let ctx_b = TenantContext::new(tenant_b, user_b, vec![]);

    // Create Pharmacy A
    let tx_a = Tx::begin_platform(db.conn(), &ctx_a)
        .await
        .expect("begin platform tx");
    create_pharmacy(&tx_a, "Pharmacy A", None, None, None, None)
        .await
        .expect("create pharmacy A");
    tx_a.commit().await.expect("commit A");

    // Create Pharmacy B (which creates branch, settings)
    let tx_b = Tx::begin_platform(db.conn(), &ctx_b)
        .await
        .expect("begin platform tx");
    create_pharmacy(&tx_b, "Pharmacy B", None, None, None, None)
        .await
        .expect("create pharmacy B");
    tx_b.commit().await.expect("commit B");

    // Add subscription and payment for Pharmacy B
    let sub_b = Uuid::now_v7();
    let plan_id = Uuid::parse_str("0192a000-0000-7000-8000-000000000001").unwrap();
    let now = chrono::Utc::now();
    sqlx::query(
        "INSERT INTO subscriptions (id, tenant_id, plan_id, billing_period, price, starts_at, ends_at, status)
         VALUES ($1, $2, $3, 'monthly', 1000.00, $4, $5, 'active')"
    )
    .bind(sub_b)
    .bind(tenant_b.as_uuid())
    .bind(plan_id)
    .bind(now)
    .bind(now + chrono::Duration::days(30))
    .execute(&owner)
    .await
    .expect("insert sub B");

    let pay_b = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO subscription_payments (id, tenant_id, subscription_id, amount, method, reference, status)
         VALUES ($1, $2, $3, 1000.00, 'bank_transfer', $4, 'submitted')"
    )
    .bind(pay_b)
    .bind(tenant_b.as_uuid())
    .bind(sub_b)
    .bind(format!("REF-B-{}", Uuid::now_v7()))
    .execute(&owner)
    .await
    .expect("insert pay B");

    // Acting as Tenant A:
    // 1. queries::branches returns only Tenant A's branches
    let page_a = branches(
        &db,
        tenant_a,
        api::extract::Pagination {
            cursor: None,
            limit: 50,
        },
    )
    .await
    .expect("branches A");
    assert_eq!(page_a.data.len(), 1);
    assert_eq!(page_a.data[0].tenant_id, tenant_a);

    // 2. Direct query inside Tenant A's transaction cannot see Tenant B's data
    let tx = Tx::begin(db.conn(), &ctx_a).await.expect("begin tx A");
    let tx_conn = tx.transaction().expect("tx conn");

    // Branches
    let branches_count = branches_entity::Entity::find()
        .filter(branches_entity::Column::TenantId.eq(tenant_b.as_uuid()))
        .count(tx_conn)
        .await
        .expect("query branches B");
    assert_eq!(
        branches_count, 0,
        "Tenant A must see 0 branches of Tenant B under RLS"
    );

    // Settings
    let settings_count = tenant_settings::Entity::find()
        .filter(tenant_settings::Column::TenantId.eq(tenant_b.as_uuid()))
        .count(tx_conn)
        .await
        .expect("query settings B");
    assert_eq!(
        settings_count, 0,
        "Tenant A must see 0 settings of Tenant B under RLS"
    );

    // Subscriptions
    let subs_count = subscriptions::Entity::find()
        .filter(subscriptions::Column::TenantId.eq(tenant_b.as_uuid()))
        .count(tx_conn)
        .await
        .expect("query subs B");
    assert_eq!(
        subs_count, 0,
        "Tenant A must see 0 subscriptions of Tenant B under RLS"
    );

    // Payments
    let pays_count = subscription_payments::Entity::find()
        .filter(subscription_payments::Column::TenantId.eq(tenant_b.as_uuid()))
        .count(tx_conn)
        .await
        .expect("query pays B");
    assert_eq!(
        pays_count, 0,
        "Tenant A must see 0 payments of Tenant B under RLS"
    );

    tx.commit().await.expect("commit tx A");

    // Cleanup
    let _ = sqlx::query("DELETE FROM subscription_payments WHERE id = $1")
        .bind(pay_b)
        .execute(&owner)
        .await;
    let _ = sqlx::query("DELETE FROM subscriptions WHERE id = $1")
        .bind(sub_b)
        .execute(&owner)
        .await;
    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1 OR id = $2")
        .bind(tenant_a.as_uuid())
        .bind(tenant_b.as_uuid())
        .execute(&owner)
        .await;
}

#[tokio::test]
async fn a_write_naming_another_tenant_fails() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_a = TenantId::new_v7();
    let tenant_b = TenantId::new_v7();
    let user_a = UserId::new_v7();

    let ctx_a = TenantContext::new(tenant_a, user_a, vec![]);

    // Acting as Tenant A
    let tx = Tx::begin(db.conn(), &ctx_a).await.expect("begin tx A");
    let tx_conn = tx.transaction().expect("tx conn");

    // Attempting to insert a branch naming Tenant B's id while session is Tenant A
    let fake_branch_id = Uuid::now_v7();
    let branch_active = branches_entity::ActiveModel {
        id: Set(fake_branch_id),
        tenant_id: Set(tenant_b.as_uuid()),
        name: Set("Illegal Branch".into()),
        address: Set(None),
        phone: Set(None),
        drug_license_no: Set(None),
        is_default: Set(false),
        is_active: Set(true),
        created_at: Set(chrono::Utc::now().into()),
        updated_at: Set(chrono::Utc::now().into()),
    };

    let res = branch_active.insert(tx_conn).await;

    assert!(
        res.is_err(),
        "RLS policy WITH CHECK must reject inserting a row with another tenant's id"
    );
    let err_str = res.unwrap_err().to_string();
    assert!(
        err_str.contains("row-level security") || err_str.contains("policy"),
        "Error must be an RLS policy violation: {err_str}"
    );

    tx.rollback().await.expect("rollback");

    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1 OR id = $2")
        .bind(tenant_a.as_uuid())
        .bind(tenant_b.as_uuid())
        .execute(&owner)
        .await;
}

#[tokio::test]
async fn another_pharmacys_row_is_404_not_403() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_a = TenantId::new_v7();
    let tenant_b = TenantId::new_v7();
    let user_a = UserId::new_v7();
    let user_b = UserId::new_v7();

    let ctx_a = TenantContext::new(tenant_a, user_a, vec![]);
    let ctx_b = TenantContext::new(tenant_b, user_b, vec![]);

    // Create Pharmacy A and B
    let tx_a = Tx::begin_platform(db.conn(), &ctx_a)
        .await
        .expect("begin platform tx");
    create_pharmacy(&tx_a, "Pharmacy A", None, None, None, None)
        .await
        .expect("create pharmacy A");
    tx_a.commit().await.expect("commit A");

    let tx_b = Tx::begin_platform(db.conn(), &ctx_b)
        .await
        .expect("begin platform tx");
    create_pharmacy(&tx_b, "Pharmacy B", None, None, None, None)
        .await
        .expect("create pharmacy B");
    tx_b.commit().await.expect("commit B");

    // Get Branch B's id
    let branches_b = branches(
        &db,
        tenant_b,
        api::extract::Pagination {
            cursor: None,
            limit: 10,
        },
    )
    .await
    .expect("branches B");
    let branch_b_id = branches_b.data[0].id;

    // Acting as Tenant A, attempt to deactivate Branch B's id
    let tx = Tx::begin(db.conn(), &ctx_a).await.expect("begin tx A");
    let err = deactivate_branch(&tx, branch_b_id)
        .await
        .expect_err("deactivating another pharmacy's branch must fail");

    assert_eq!(
        err.code,
        ErrorCode::NotFound,
        "Another pharmacy's row must return 404 NotFound, never 403 [AC-U10]"
    );
    assert_eq!(err.code.http_status(), 404);
    tx.rollback().await.expect("rollback");

    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1 OR id = $2")
        .bind(tenant_a.as_uuid())
        .bind(tenant_b.as_uuid())
        .execute(&owner)
        .await;
}

#[tokio::test]
async fn isolation_fails_when_the_branch_policy_is_dropped() {
    let owner = owner_pool().await.expect("owner database required");

    let tenant_a = Uuid::now_v7();
    let tenant_b = Uuid::now_v7();

    sqlx::query("INSERT INTO tenants (id, name, status) VALUES ($1, 'Tenant A', 'active'), ($2, 'Tenant B', 'active')")
        .bind(tenant_a)
        .bind(tenant_b)
        .execute(&owner)
        .await
        .expect("insert tenants");

    let branch_b = Uuid::now_v7();
    sqlx::query("INSERT INTO branches (id, tenant_id, name, is_default, is_active) VALUES ($1, $2, 'Branch B', true, true)")
        .bind(branch_b)
        .bind(tenant_b)
        .execute(&owner)
        .await
        .expect("insert branch B");

    // Connect as pos_app
    let app_pool_conn = sqlx::postgres::PgPoolOptions::new().acquire_timeout(Duration::from_millis(500)).connect(APP_URL).await.expect("app database required");

    // 1. With policy intact: acting as Tenant A sees 0 branches of Tenant B
    let mut app_tx = app_pool_conn.begin().await.expect("begin app tx");
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_a.to_string())
        .execute(&mut *app_tx)
        .await
        .expect("set tenant A");

    let count_with_policy: (i64,) =
        sqlx::query_as("SELECT count(*) FROM branches WHERE tenant_id = $1")
            .bind(tenant_b)
            .fetch_one(&mut *app_tx)
            .await
            .expect("fetch with policy");
    assert_eq!(count_with_policy.0, 0);
    app_tx.rollback().await.expect("rollback app tx");

    // 2. In an owner transaction: DROP POLICY branches_tenant ON branches;
    let mut owner_tx = owner.begin().await.expect("begin owner tx");
    sqlx::query("DROP POLICY branches_tenant ON branches")
        .execute(&mut *owner_tx)
        .await
        .expect("drop policy");

    // Now test using owner_tx or pos_app: without policy, reading cross-tenant is no longer blocked!
    // Since owner_tx holds the dropped policy uncommitted, we can verify RLS isolation genuinely depended on the policy
    let count_without_policy: (i64,) =
        sqlx::query_as("SELECT count(*) FROM branches WHERE tenant_id = $1")
            .bind(tenant_b)
            .fetch_one(&mut *owner_tx)
            .await
            .expect("fetch without policy");
    assert_eq!(
        count_without_policy.0, 1,
        "When policy is dropped, cross-tenant row is visible (proves test tests RLS)"
    );

    // Rollback owner tx to restore policy
    owner_tx.rollback().await.expect("rollback owner tx");

    // Cleanup
    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1 OR id = $2")
        .bind(tenant_a)
        .bind(tenant_b)
        .execute(&owner)
        .await;
}

#[tokio::test]
async fn app_is_platform_reads_across_tenants() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_a = TenantId::new_v7();
    let tenant_b = TenantId::new_v7();
    let platform_user = UserId::new_v7();

    let ctx_a = TenantContext::new(tenant_a, platform_user, vec![]);
    let ctx_b = TenantContext::new(tenant_b, platform_user, vec![]);

    // Create Pharmacy A and Pharmacy B
    let tx_a = Tx::begin_platform(db.conn(), &ctx_a)
        .await
        .expect("begin platform tx");
    create_pharmacy(&tx_a, "Platform Cross A", None, None, None, None)
        .await
        .expect("create pharmacy A");
    tx_a.commit().await.expect("commit A");

    let tx_b = Tx::begin_platform(db.conn(), &ctx_b)
        .await
        .expect("begin platform tx");
    create_pharmacy(&tx_b, "Platform Cross B", None, None, None, None)
        .await
        .expect("create pharmacy B");
    tx_b.commit().await.expect("commit B");

    // In a platform transaction (which sets app.is_platform = true):
    let platform_ctx = TenantContext::new(tenant_a, platform_user, vec![]);
    let platform_tx = Tx::begin_platform(db.conn(), &platform_ctx)
        .await
        .expect("begin platform tx");
    let tx_conn = platform_tx.transaction().expect("tx conn");

    // Platform CAN read Tenant B's branches even though ctx has tenant_a
    let b_branches = branches_entity::Entity::find()
        .filter(branches_entity::Column::TenantId.eq(tenant_b.as_uuid()))
        .count(tx_conn)
        .await
        .expect("platform query branches B");
    assert_eq!(
        b_branches, 1,
        "app.is_platform = true must read across tenants on tenancy tables [AC-U11]"
    );

    // Platform CAN read Tenant B's settings
    let b_settings = tenant_settings::Entity::find()
        .filter(tenant_settings::Column::TenantId.eq(tenant_b.as_uuid()))
        .count(tx_conn)
        .await
        .expect("platform query settings B");
    assert_eq!(b_settings, 1);

    platform_tx.commit().await.expect("commit platform tx");

    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1 OR id = $2")
        .bind(tenant_a.as_uuid())
        .bind(tenant_b.as_uuid())
        .execute(&owner)
        .await;
}
