use chrono::Utc;
use db::{Db, Tx};
use pos_core::context::TenantContext;
use pos_core::error::ErrorCode;
use pos_core::ids::{TenantId, UserId};
use rust_decimal::Decimal;
use sea_orm::Database;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use tenancy::commands::{create_pharmacy, submit_payment};
use tenancy::platform::{
    approve, confirm_payment, create_subscription, reject_payment, suspend,
};
use tenancy::types::{NewPayment, NewSubscription};
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

const STANDARD_PLAN_ID: &str = "0192a000-0000-7000-8000-000000000001";

#[tokio::test]
async fn approve_records_who_and_when() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_id = TenantId::new_v7();
    let platform_user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, platform_user_id, vec![]);

    // 1. Create pharmacy (starts as pending_approval)
    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .expect("begin tx");
    create_pharmacy(&tx, "Approval Test Pharmacy", None, None, None, None)
        .await
        .expect("create pharmacy");
    tx.commit().await.expect("commit create");

    let before = Utc::now();

    // 2. Platform staff approves pharmacy
    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .expect("begin platform tx");
    let approved = approve(&tx, tenant_id, platform_user_id)
        .await
        .expect("approve pharmacy");
    tx.commit().await.expect("commit approve");

    assert_eq!(approved.status, "active");

    // 3. Verify in database: status is active, approved_by_platform_user_id is recorded, approved_at is recorded
    let (status, approved_by, approved_at): (
        String,
        Option<Uuid>,
        Option<chrono::DateTime<Utc>>,
    ) = sqlx::query_as(
        "SELECT status, approved_by_platform_user_id, approved_at FROM tenants WHERE id = $1",
    )
    .bind(tenant_id.as_uuid())
    .fetch_one(&owner)
    .await
    .expect("fetch approved tenant");

    assert_eq!(status, "active");
    assert_eq!(approved_by, Some(platform_user_id.as_uuid()));
    assert!(approved_at.is_some());
    assert!(approved_at.unwrap() >= before);

    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
}

#[tokio::test]
async fn suspension_requires_a_reason() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_id = TenantId::new_v7();
    let platform_user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, platform_user_id, vec![]);

    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .expect("begin tx");
    create_pharmacy(&tx, "Suspension Test Pharmacy", None, None, None, None)
        .await
        .expect("create pharmacy");
    tx.commit().await.expect("commit");

    // 1. Command level: empty reason is rejected with ValidationFailed
    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin tx");
    let err = suspend(&tx, tenant_id, "   ".to_string())
        .await
        .expect_err("empty suspension reason must fail");
    assert_eq!(err.code, ErrorCode::ValidationFailed);
    tx.rollback().await.expect("rollback");

    // 2. Command level: providing a reason succeeds
    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin tx");
    let suspended = suspend(&tx, tenant_id, "Violated terms of service".to_string())
        .await
        .expect("suspension with reason succeeds");
    assert_eq!(suspended.status, "suspended");
    tx.commit().await.expect("commit");

    // 3. Database level: check constraint chk_tenants_suspension_reason forbids status='suspended' with suspension_reason=NULL
    let res = sqlx::query("UPDATE tenants SET status = 'suspended', suspension_reason = NULL WHERE id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
    assert!(
        res.is_err(),
        "Database check constraint chk_tenants_suspension_reason must reject status='suspended' without suspension_reason"
    );

    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
}

#[tokio::test]
async fn confirming_a_payment_activates_its_subscription_in_one_transaction() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_id = TenantId::new_v7();
    let platform_user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, platform_user_id, vec![]);

    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .expect("begin tx");
    create_pharmacy(&tx, "Confirm Payment Pharmacy", None, None, None, None)
        .await
        .expect("create pharmacy");
    tx.commit().await.expect("commit");

    let plan_id = Uuid::parse_str(STANDARD_PLAN_ID).unwrap();
    let sub_id = Uuid::now_v7();
    let now = Utc::now();

    // Insert an initial past_due subscription
    sqlx::query(
        "INSERT INTO subscriptions (id, tenant_id, plan_id, billing_period, price, starts_at, ends_at, status)
         VALUES ($1, $2, $3, 'monthly', 1000.00, $4, $5, 'past_due')"
    )
    .bind(sub_id)
    .bind(tenant_id.as_uuid())
    .bind(plan_id)
    .bind(now - chrono::Duration::days(1))
    .bind(now + chrono::Duration::days(29))
    .execute(&owner)
    .await
    .expect("insert subscription");

    // Pharmacy submits payment linked to subscription
    let tx = Tx::begin(db.conn(), &ctx).await.expect("begin tx");
    let payment = submit_payment(
        &tx,
        NewPayment {
            subscription_id: Some(sub_id),
            amount: Decimal::from(1000),
            method: "bank_transfer".into(),
            reference: format!("CONFIRM-{}", Uuid::now_v7()),
            proof_file_key: None,
        },
    )
    .await
    .expect("submit payment");
    tx.commit().await.expect("commit payment");

    assert_eq!(payment.status, "submitted");

    // Platform confirms payment
    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin tx");
    let (confirmed_pay, activated_sub) = confirm_payment(&tx, payment.id)
        .await
        .expect("confirm payment");
    tx.commit().await.expect("commit confirm");

    assert_eq!(confirmed_pay.status, "confirmed");
    assert_eq!(activated_sub.status, "active");

    // Verify in database: both payment is confirmed and subscription is active
    let pay_row: (String,) =
        sqlx::query_as("SELECT status FROM subscription_payments WHERE id = $1")
            .bind(payment.id)
            .fetch_one(&owner)
            .await
            .expect("fetch payment");
    assert_eq!(pay_row.0, "confirmed");

    let sub_row: (String,) =
        sqlx::query_as("SELECT status FROM subscriptions WHERE id = $1")
            .bind(sub_id)
            .fetch_one(&owner)
            .await
            .expect("fetch subscription");
    assert_eq!(sub_row.0, "active");

    let _ = sqlx::query("DELETE FROM subscription_payments WHERE id = $1")
        .bind(payment.id)
        .execute(&owner)
        .await;
    let _ = sqlx::query("DELETE FROM subscriptions WHERE id = $1")
        .bind(sub_id)
        .execute(&owner)
        .await;
    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
}

#[tokio::test]
async fn rejecting_a_payment_requires_a_note() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_id = TenantId::new_v7();
    let platform_user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, platform_user_id, vec![]);

    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .expect("begin tx");
    create_pharmacy(&tx, "Reject Payment Pharmacy", None, None, None, None)
        .await
        .expect("create pharmacy");
    tx.commit().await.expect("commit");

    let tx = Tx::begin(db.conn(), &ctx).await.expect("begin tx");
    let payment = submit_payment(
        &tx,
        NewPayment {
            subscription_id: None,
            amount: Decimal::from(1000),
            method: "bank_transfer".into(),
            reference: format!("REJECT-{}", Uuid::now_v7()),
            proof_file_key: None,
        },
    )
    .await
    .expect("submit payment");
    tx.commit().await.expect("commit payment");

    // Rejecting without a note fails with ValidationFailed
    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin tx");
    let err = reject_payment(&tx, payment.id, "   ".to_string())
        .await
        .expect_err("rejecting without a note must fail");
    assert_eq!(err.code, ErrorCode::ValidationFailed);
    tx.rollback().await.expect("rollback");

    // Rejecting with a note succeeds
    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin tx");
    let rejected = reject_payment(
        &tx,
        payment.id,
        "Transaction reference not found on bank statement".to_string(),
    )
    .await
    .expect("rejecting with note succeeds");
    assert_eq!(rejected.status, "rejected");
    tx.commit().await.expect("commit reject");

    let row: (String, Option<String>) = sqlx::query_as(
        "SELECT status, review_note FROM subscription_payments WHERE id = $1",
    )
    .bind(payment.id)
    .fetch_one(&owner)
    .await
    .expect("fetch payment");
    assert_eq!(row.0, "rejected");
    assert_eq!(
        row.1.as_deref(),
        Some("Transaction reference not found on bank statement")
    );

    let _ = sqlx::query("DELETE FROM subscription_payments WHERE id = $1")
        .bind(payment.id)
        .execute(&owner)
        .await;
    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
}

#[tokio::test]
async fn a_complimentary_subscription_without_a_reason_is_rejected() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_id = TenantId::new_v7();
    let platform_user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, platform_user_id, vec![]);

    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .expect("begin tx");
    create_pharmacy(&tx, "Complimentary Pharmacy", None, None, None, None)
        .await
        .expect("create pharmacy");
    tx.commit().await.expect("commit");

    let plan_id = Uuid::parse_str(STANDARD_PLAN_ID).unwrap();
    let now = Utc::now();

    // 1. Command level: complimentary with None grant_reason fails
    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin tx");
    let err = create_subscription(
        &tx,
        tenant_id,
        NewSubscription {
            plan_id,
            billing_period: "monthly".into(),
            starts_at: now,
            ends_at: now + chrono::Duration::days(30),
            is_complimentary: true,
            grant_reason: None,
        },
    )
    .await
    .expect_err("complimentary without grant_reason must fail");
    assert_eq!(err.code, ErrorCode::ValidationFailed);
    tx.rollback().await.expect("rollback");

    // 2. Database level: check constraint chk_subscriptions_complimentary_grant_reason
    let res = sqlx::query(
        "INSERT INTO subscriptions (id, tenant_id, plan_id, billing_period, price, starts_at, ends_at, is_complimentary, grant_reason)
         VALUES ($1, $2, $3, 'monthly', 1000.00, $4, $5, true, NULL)"
    )
    .bind(Uuid::now_v7())
    .bind(tenant_id.as_uuid())
    .bind(plan_id)
    .bind(now)
    .bind(now + chrono::Duration::days(30))
    .execute(&owner)
    .await;
    assert!(
        res.is_err(),
        "Database check constraint chk_subscriptions_complimentary_grant_reason must reject complimentary subscription without grant_reason"
    );

    // 3. Command level: complimentary with valid grant_reason succeeds
    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin tx");
    let sub = create_subscription(
        &tx,
        tenant_id,
        NewSubscription {
            plan_id,
            billing_period: "monthly".into(),
            starts_at: now,
            ends_at: now + chrono::Duration::days(30),
            is_complimentary: true,
            grant_reason: Some("Pilot partner courtesy grant [D-81]".into()),
        },
    )
    .await
    .expect("complimentary subscription with reason succeeds");
    assert!(sub.is_complimentary);
    tx.commit().await.expect("commit");

    let _ = sqlx::query("DELETE FROM subscriptions WHERE id = $1")
        .bind(sub.id)
        .execute(&owner)
        .await;
    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
}

#[tokio::test]
async fn a_plan_price_change_does_not_alter_a_sold_subscription() {
    let owner = owner_pool().await.expect("owner database required");

    let tenant_id = Uuid::now_v7();
    sqlx::query("INSERT INTO tenants (id, name, status) VALUES ($1, 'Sold Sub Pharmacy', 'active')")
        .bind(tenant_id)
        .execute(&owner)
        .await
        .expect("insert tenant");

    let plan_id = Uuid::now_v7();
    sqlx::query("INSERT INTO plans (id, code, name, grace_days) VALUES ($1, $2, 'Dynamic Price Plan', 7)")
        .bind(plan_id)
        .bind(format!("dyn-{}", Uuid::now_v7()))
        .execute(&owner)
        .await
        .expect("insert plan");

    sqlx::query("INSERT INTO plan_prices (plan_id, billing_period, price) VALUES ($1, 'monthly', 1000.00)")
        .bind(plan_id)
        .execute(&owner)
        .await
        .expect("insert initial price");

    let sub_id = Uuid::now_v7();
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO subscriptions (id, tenant_id, plan_id, billing_period, price, starts_at, ends_at, status)
         VALUES ($1, $2, $3, 'monthly', 1000.00, $4, $5, 'active')"
    )
    .bind(sub_id)
    .bind(tenant_id)
    .bind(plan_id)
    .bind(now)
    .bind(now + chrono::Duration::days(30))
    .execute(&owner)
    .await
    .expect("insert subscription");

    // Later: platform increases the plan price for future sales
    sqlx::query("UPDATE plan_prices SET price = 1500.00 WHERE plan_id = $1 AND billing_period = 'monthly'")
        .bind(plan_id)
        .execute(&owner)
        .await
        .expect("update plan price");

    // The subscription already sold still holds 1000.00
    let sub_price: (Decimal,) =
        sqlx::query_as("SELECT price FROM subscriptions WHERE id = $1")
            .bind(sub_id)
            .fetch_one(&owner)
            .await
            .expect("fetch sub price");

    assert_eq!(
        sub_price.0,
        Decimal::from(1000),
        "Price change in plan_prices must never alter a previously sold subscription [AC-U06]"
    );

    let _ = sqlx::query("DELETE FROM subscriptions WHERE id = $1")
        .bind(sub_id)
        .execute(&owner)
        .await;
    let _ = sqlx::query("DELETE FROM plan_prices WHERE plan_id = $1")
        .bind(plan_id)
        .execute(&owner)
        .await;
    let _ = sqlx::query("DELETE FROM plans WHERE id = $1")
        .bind(plan_id)
        .execute(&owner)
        .await;
    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(tenant_id)
        .execute(&owner)
        .await;
}

#[tokio::test]
async fn an_already_reviewed_payment_cannot_be_reviewed_again() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_id = TenantId::new_v7();
    let platform_user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, platform_user_id, vec![]);

    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .expect("begin tx");
    create_pharmacy(&tx, "Reviewed Payment Pharmacy", None, None, None, None)
        .await
        .expect("create pharmacy");
    tx.commit().await.expect("commit");

    let plan_id = Uuid::parse_str(STANDARD_PLAN_ID).unwrap();
    let sub_id = Uuid::now_v7();
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO subscriptions (id, tenant_id, plan_id, billing_period, price, starts_at, ends_at, status)
         VALUES ($1, $2, $3, 'monthly', 1000.00, $4, $5, 'active')"
    )
    .bind(sub_id)
    .bind(tenant_id.as_uuid())
    .bind(plan_id)
    .bind(now)
    .bind(now + chrono::Duration::days(30))
    .execute(&owner)
    .await
    .expect("insert subscription");

    let tx = Tx::begin(db.conn(), &ctx).await.expect("begin tx");
    let payment = submit_payment(
        &tx,
        NewPayment {
            subscription_id: Some(sub_id),
            amount: Decimal::from(1000),
            method: "bank_transfer".into(),
            reference: format!("REV-{}", Uuid::now_v7()),
            proof_file_key: None,
        },
    )
    .await
    .expect("submit payment");
    tx.commit().await.expect("commit payment");

    // 1. Confirm payment
    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin tx");
    let (confirmed, _) = confirm_payment(&tx, payment.id)
        .await
        .expect("confirm payment");
    assert_eq!(confirmed.status, "confirmed");
    tx.commit().await.expect("commit confirm");

    // 2. Second confirmation fails with already_reviewed
    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin tx");
    let err = confirm_payment(&tx, payment.id)
        .await
        .expect_err("second confirm must fail");
    assert_eq!(
        err.code,
        ErrorCode::Conflict("already_reviewed"),
        "Must return 409 already_reviewed"
    );
    tx.rollback().await.expect("rollback");

    // 3. Rejecting an already-confirmed payment also fails with already_reviewed
    let tx = Tx::begin_platform(db.conn(), &ctx).await.expect("begin tx");
    let err = reject_payment(&tx, payment.id, "Note".into())
        .await
        .expect_err("rejecting confirmed payment must fail");
    assert_eq!(err.code, ErrorCode::Conflict("already_reviewed"));
    tx.rollback().await.expect("rollback");

    let _ = sqlx::query("DELETE FROM subscription_payments WHERE id = $1")
        .bind(payment.id)
        .execute(&owner)
        .await;
    let _ = sqlx::query("DELETE FROM subscriptions WHERE id = $1")
        .bind(sub_id)
        .execute(&owner)
        .await;
    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
}
