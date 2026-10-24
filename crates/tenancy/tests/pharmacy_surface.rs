use chrono::Utc;
use db::{Db, Tx};
use pos_core::context::TenantContext;
use pos_core::error::ErrorCode;
use pos_core::ids::{TenantId, UserId};
use rust_decimal::Decimal;
use sea_orm::Database;
use sqlx::postgres::PgPoolOptions;
use std::str::FromStr;
use std::time::Duration;
use tenancy::commands::{
    create_branch, create_pharmacy, deactivate_branch, request_closure, submit_payment,
    update_settings,
};
use tenancy::queries::{branches, limit_remaining, settings, tenant};
use tenancy::types::{NewBranch, NewPayment, SettingsPatch};
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
async fn branches_are_cursor_paginated() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, user_id, vec![]);

    // 1. Create pharmacy (which creates 1 default branch "Main")
    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .expect("begin platform tx");
    create_pharmacy(&tx, "Pagination Test Pharmacy", None, None, None, None)
        .await
        .expect("create pharmacy");
    tx.commit().await.expect("commit pharmacy");

    // 2. Add 4 more branches (total 5)
    for i in 1..=4 {
        let tx = Tx::begin(db.conn(), &ctx).await.expect("begin tx");
        create_branch(
            &tx,
            NewBranch {
                name: format!("Branch {i}"),
                address: None,
                phone: None,
                drug_license_no: None,
            },
        )
        .await
        .expect("create branch");
        tx.commit().await.expect("commit branch");
    }

    // 3. Page 1: limit 2
    let page1 = branches(
        &db,
        tenant_id,
        api::extract::Pagination {
            cursor: None,
            limit: 2,
        },
    )
    .await
    .expect("page 1");

    assert_eq!(page1.data.len(), 2);
    assert!(page1.has_more);
    assert!(page1.next_cursor.is_some());

    // 4. Page 2: limit 2
    let page2 = branches(
        &db,
        tenant_id,
        api::extract::Pagination {
            cursor: page1.next_cursor.clone(),
            limit: 2,
        },
    )
    .await
    .expect("page 2");

    assert_eq!(page2.data.len(), 2);
    assert!(page2.has_more);
    assert!(page2.next_cursor.is_some());

    // 5. Page 3: limit 2
    let page3 = branches(
        &db,
        tenant_id,
        api::extract::Pagination {
            cursor: page2.next_cursor.clone(),
            limit: 2,
        },
    )
    .await
    .expect("page 3");

    assert_eq!(page3.data.len(), 1);
    assert!(!page3.has_more);
    assert!(page3.next_cursor.is_none());

    // Verify all 5 are distinct
    let mut all_ids = Vec::new();
    all_ids.extend(page1.data.into_iter().map(|b| b.id));
    all_ids.extend(page2.data.into_iter().map(|b| b.id));
    all_ids.extend(page3.data.into_iter().map(|b| b.id));
    assert_eq!(all_ids.len(), 5);

    let mut deduped = all_ids.clone();
    deduped.dedup();
    assert_eq!(deduped.len(), 5);

    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
}

#[tokio::test]
async fn creating_a_pharmacy_creates_one_default_branch_and_settings() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, user_id, vec![]);

    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .expect("begin platform tx");
    let t = create_pharmacy(
        &tx,
        "Single Tx Pharmacy",
        Some("Legal Name Ltd".into()),
        Some("BIN123456".into()),
        Some("01711111111".into()),
        Some("test@pharmacy.com".into()),
    )
    .await
    .expect("create pharmacy");
    tx.commit().await.expect("commit tx");

    assert_eq!(t.id, tenant_id);
    assert_eq!(t.name, "Single Tx Pharmacy");
    assert_eq!(t.status, "pending_approval");
    assert_eq!(t.access.mode, "pending_approval");

    // Verify tenant query reads correctly
    let fetched_tenant = tenant(&db, tenant_id).await.expect("query tenant");
    assert_eq!(fetched_tenant.name, "Single Tx Pharmacy");
    assert_eq!(fetched_tenant.access.mode, "pending_approval");

    // Verify settings created in the same transaction
    let s = settings(&db, tenant_id).await.expect("query settings");
    assert_eq!(s.vat_rate, Decimal::ZERO);
    assert_eq!(s.rounding_mode, "nearest_half");
    assert_eq!(s.near_expiry_days, 90);

    // Verify branch created in the same transaction
    let br_page = branches(
        &db,
        tenant_id,
        api::extract::Pagination {
            cursor: None,
            limit: 10,
        },
    )
    .await
    .expect("query branches");

    assert_eq!(br_page.data.len(), 1);
    let br = &br_page.data[0];
    assert!(br.is_default);
    assert!(br.is_active);
    assert_eq!(br.name, "Main");

    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
}

#[tokio::test]
async fn duplicate_branch_name_within_a_tenant_is_rejected() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, user_id, vec![]);

    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .expect("begin platform tx");
    create_pharmacy(&tx, "Dup Branch Pharmacy", None, None, None, None)
        .await
        .expect("create pharmacy");
    tx.commit().await.expect("commit");

    // Default branch is already named "Main". Attempting to add another "Main" must fail with 409
    let tx = Tx::begin(db.conn(), &ctx).await.expect("begin tx");
    let err = create_branch(
        &tx,
        NewBranch {
            name: "Main".into(),
            address: None,
            phone: None,
            drug_license_no: None,
        },
    )
    .await
    .expect_err("duplicate branch name must fail");

    assert_eq!(
        err.code,
        ErrorCode::Conflict("duplicate_branch_name"),
        "Must return 409 duplicate_branch_name"
    );
    assert_eq!(err.code.http_status(), 409);
    tx.rollback().await.expect("rollback");

    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
}

#[tokio::test]
async fn the_last_active_branch_cannot_be_deactivated() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, user_id, vec![]);

    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .expect("begin platform tx");
    create_pharmacy(&tx, "Deactivate Branch Pharmacy", None, None, None, None)
        .await
        .expect("create pharmacy");
    tx.commit().await.expect("commit");

    let br_page = branches(
        &db,
        tenant_id,
        api::extract::Pagination {
            cursor: None,
            limit: 10,
        },
    )
    .await
    .expect("branches");
    let default_branch_id = br_page.data[0].id;

    // 1. Attempting to deactivate the only active branch (default branch) must fail
    let tx = Tx::begin(db.conn(), &ctx).await.expect("begin tx");
    let err = deactivate_branch(&tx, default_branch_id)
        .await
        .expect_err("deactivating default branch must fail");
    assert_eq!(
        err.code,
        ErrorCode::Conflict("last_active_branch"),
        "Default branch cannot be deactivated"
    );
    tx.rollback().await.expect("rollback");

    // 2. Add second branch
    let tx = Tx::begin(db.conn(), &ctx).await.expect("begin tx");
    let second_branch = create_branch(
        &tx,
        NewBranch {
            name: "Branch 2".into(),
            address: None,
            phone: None,
            drug_license_no: None,
        },
    )
    .await
    .expect("create second branch");
    tx.commit().await.expect("commit branch 2");

    // 3. Attempting to deactivate default branch even when another active branch exists must still fail
    let tx = Tx::begin(db.conn(), &ctx).await.expect("begin tx");
    let err = deactivate_branch(&tx, default_branch_id)
        .await
        .expect_err("default branch still cannot be deactivated");
    assert_eq!(err.code, ErrorCode::Conflict("last_active_branch"));
    tx.rollback().await.expect("rollback");

    // 4. Deactivating second branch succeeds
    let tx = Tx::begin(db.conn(), &ctx).await.expect("begin tx");
    let deactivated = deactivate_branch(&tx, second_branch.id)
        .await
        .expect("deactivating branch 2 succeeds");
    assert!(!deactivated.is_active);
    tx.commit().await.expect("commit");

    // 5. Now only 1 active branch remains again. Attempting to deactivate it fails
    let tx = Tx::begin(db.conn(), &ctx).await.expect("begin tx");
    let err = deactivate_branch(&tx, default_branch_id)
        .await
        .expect_err("deactivating last active branch must fail");
    assert_eq!(err.code, ErrorCode::Conflict("last_active_branch"));
    tx.rollback().await.expect("rollback");

    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
}

#[tokio::test]
async fn settings_patch_rejects_a_malformed_vat_rate() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, user_id, vec![]);

    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .expect("begin platform tx");
    create_pharmacy(&tx, "Settings VAT Pharmacy", None, None, None, None)
        .await
        .expect("create pharmacy");
    tx.commit().await.expect("commit");

    // Negative VAT rate is rejected
    let tx = Tx::begin(db.conn(), &ctx).await.expect("begin tx");
    let err = update_settings(
        &tx,
        SettingsPatch {
            vat_rate: Some(Decimal::from(-1)),
            ..Default::default()
        },
    )
    .await
    .expect_err("negative vat_rate must be rejected");
    assert_eq!(err.code, ErrorCode::ValidationFailed);
    tx.rollback().await.expect("rollback");

    // > 100 VAT rate is rejected
    let tx = Tx::begin(db.conn(), &ctx).await.expect("begin tx");
    let err = update_settings(
        &tx,
        SettingsPatch {
            vat_rate: Some(Decimal::from(105)),
            ..Default::default()
        },
    )
    .await
    .expect_err(">100 vat_rate must be rejected");
    assert_eq!(err.code, ErrorCode::ValidationFailed);
    tx.rollback().await.expect("rollback");

    // Valid VAT rate is accepted and partial update preserves other fields
    let tx = Tx::begin(db.conn(), &ctx).await.expect("begin tx");
    let updated = update_settings(
        &tx,
        SettingsPatch {
            vat_rate: Some(Decimal::from_str("7.50").unwrap()),
            ..Default::default()
        },
    )
    .await
    .expect("valid vat_rate succeeds");
    assert_eq!(updated.vat_rate, Decimal::from_str("7.50").unwrap());
    assert_eq!(updated.rounding_mode, "nearest_half"); // preserved
    tx.commit().await.expect("commit");

    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
}

#[tokio::test]
async fn a_pharmacy_cannot_create_a_confirmed_payment() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, user_id, vec![]);

    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .expect("begin platform tx");
    create_pharmacy(&tx, "Payment Pharmacy", None, None, None, None)
        .await
        .expect("create pharmacy");
    tx.commit().await.expect("commit");

    let ref_str = format!("TRX-{}", Uuid::now_v7());
    let tx = Tx::begin(db.conn(), &ctx).await.expect("begin tx");
    let payment = submit_payment(
        &tx,
        NewPayment {
            subscription_id: None,
            amount: Decimal::from(1000),
            method: "bank_transfer".into(),
            reference: ref_str.clone(),
            proof_file_key: None,
        },
    )
    .await
    .expect("submit payment");
    tx.commit().await.expect("commit");

    assert_eq!(payment.status, "submitted");

    // Verify row in database is indeed 'submitted'
    let row: (String,) =
        sqlx::query_as("SELECT status FROM subscription_payments WHERE id = $1")
            .bind(payment.id)
            .fetch_one(&owner)
            .await
            .expect("fetch payment row");
    assert_eq!(row.0, "submitted");

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
async fn closure_sets_a_purge_date_thirty_days_out() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, user_id, vec![]);

    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .expect("begin platform tx");
    create_pharmacy(&tx, "Closing Pharmacy", None, None, None, None)
        .await
        .expect("create pharmacy");
    tx.commit().await.expect("commit");

    let before = Utc::now();
    let tx = Tx::begin(db.conn(), &ctx).await.expect("begin tx");
    let closed = request_closure(&tx, user_id)
        .await
        .expect("request closure");
    tx.commit().await.expect("commit");

    assert_eq!(closed.status, "closing");

    let (del_req, purge_at): (chrono::DateTime<Utc>, chrono::DateTime<Utc>) = sqlx::query_as(
        "SELECT deletion_requested_at, purge_at FROM tenants WHERE id = $1",
    )
    .bind(tenant_id.as_uuid())
    .fetch_one(&owner)
    .await
    .expect("fetch closure dates");

    let diff = purge_at - del_req;
    assert_eq!(diff.num_days(), 30);
    assert!(del_req >= before);

    // Second closure request must fail with already_closing
    let tx = Tx::begin(db.conn(), &ctx).await.expect("begin tx");
    let err = request_closure(&tx, user_id)
        .await
        .expect_err("second closure must fail");
    assert_eq!(err.code, ErrorCode::Conflict("already_closing"));
    tx.rollback().await.expect("rollback");

    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
}

#[tokio::test]
async fn a_null_limit_is_unlimited() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, user_id, vec![]);

    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .expect("begin platform tx");
    create_pharmacy(&tx, "Unlimited Plan Pharmacy", None, None, None, None)
        .await
        .expect("create pharmacy");
    tx.commit().await.expect("commit");

    // Standard plan has max_value = NULL for branches, users, devices
    let branches_rem = limit_remaining(&db, tenant_id, "branches")
        .await
        .expect("check limit");
    assert_eq!(
        branches_rem, None,
        "NULL limit in plan_limits must return None (unlimited)"
    );

    let users_rem = limit_remaining(&db, tenant_id, "users")
        .await
        .expect("check limit");
    assert_eq!(users_rem, None);

    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
}

#[tokio::test]
async fn over_a_limit_after_a_downgrade_keeps_what_it_has() {
    let db = get_db().await.expect("database required");
    let owner = owner_pool().await.expect("owner database required");

    let tenant_id = TenantId::new_v7();
    let user_id = UserId::new_v7();
    let ctx = TenantContext::new(tenant_id, user_id, vec![]);

    // 1. Create pharmacy with default branch ("Main")
    let tx = Tx::begin_platform(db.conn(), &ctx)
        .await
        .expect("begin platform tx");
    create_pharmacy(&tx, "Downgrade Pharmacy", None, None, None, None)
        .await
        .expect("create pharmacy");
    tx.commit().await.expect("commit");

    // 2. Create 2 more branches (total 3 branches)
    for i in 1..=2 {
        let tx = Tx::begin(db.conn(), &ctx).await.expect("begin tx");
        create_branch(
            &tx,
            NewBranch {
                name: format!("Branch {i}"),
                address: None,
                phone: None,
                drug_license_no: None,
            },
        )
        .await
        .expect("create branch");
        tx.commit().await.expect("commit");
    }

    // 3. Now create a custom downgraded plan with max_value = 2 for branches
    let plan_id = Uuid::now_v7();
    sqlx::query("INSERT INTO plans (id, code, name, grace_days) VALUES ($1, $2, 'Downgraded Plan', 7)")
        .bind(plan_id)
        .bind(format!("downgraded-{}", Uuid::now_v7()))
        .execute(&owner)
        .await
        .expect("insert downgraded plan");

    sqlx::query("INSERT INTO plan_prices (plan_id, billing_period, price) VALUES ($1, 'monthly', 500.00)")
        .bind(plan_id)
        .execute(&owner)
        .await
        .expect("insert plan price");

    sqlx::query("INSERT INTO plan_limits (plan_id, limit_key, max_value) VALUES ($1, 'branches', 2)")
        .bind(plan_id)
        .execute(&owner)
        .await
        .expect("insert plan limit of 2 branches");

    // Subscribe tenant to this downgraded plan
    let sub_id = Uuid::now_v7();
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO subscriptions (id, tenant_id, plan_id, billing_period, price, starts_at, ends_at, status)
         VALUES ($1, $2, $3, 'monthly', 500.00, $4, $5, 'active')"
    )
    .bind(sub_id)
    .bind(tenant_id.as_uuid())
    .bind(plan_id)
    .bind(now - chrono::Duration::days(1))
    .bind(now + chrono::Duration::days(29))
    .execute(&owner)
    .await
    .expect("insert subscription");

    // 4. Pharmacy has 3 branches, limit is 2. Limit remaining must be 0
    let remaining = limit_remaining(&db, tenant_id, "branches")
        .await
        .expect("limit remaining");
    assert_eq!(remaining, Some(0));

    // 5. Existing 3 branches must all still be there and active
    let br_page = branches(
        &db,
        tenant_id,
        api::extract::Pagination {
            cursor: None,
            limit: 10,
        },
    )
    .await
    .expect("get branches");
    assert_eq!(br_page.data.len(), 3);
    assert!(br_page.data.iter().all(|b| b.is_active));

    // 6. Attempting to add a 4th branch must fail with 409 plan_limit_reached
    let tx = Tx::begin(db.conn(), &ctx).await.expect("begin tx");
    let err = create_branch(
        &tx,
        NewBranch {
            name: "Branch 4".into(),
            address: None,
            phone: None,
            drug_license_no: None,
        },
    )
    .await
    .expect_err("adding branch beyond limit must fail");

    assert_eq!(
        err.code,
        ErrorCode::Conflict("plan_limit_reached"),
        "Must return 409 plan_limit_reached"
    );
    tx.rollback().await.expect("rollback");

    // Cleanup
    let _ = sqlx::query("DELETE FROM subscriptions WHERE id = $1")
        .bind(sub_id)
        .execute(&owner)
        .await;
    let _ = sqlx::query("DELETE FROM plan_limits WHERE plan_id = $1")
        .bind(plan_id)
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
        .bind(tenant_id.as_uuid())
        .execute(&owner)
        .await;
}
