#![allow(clippy::unwrap_used, clippy::expect_used)]

use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use uuid::Uuid;

const OWNER_URL: &str = "postgres://pos_owner:devpass@127.0.0.1:5432/medicine_pos";
const APP_URL: &str = "postgres://pos_app:devpass@127.0.0.1:5432/medicine_pos";

async fn owner_pool() -> sqlx::PgPool {
    let url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| OWNER_URL.to_string());
    PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(1000))
        .connect(&url)
        .await
        .expect("Owner database connection required")
}

async fn app_pool() -> sqlx::PgPool {
    let url = std::env::var("APP_DATABASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| APP_URL.to_string());
    PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(1000))
        .connect(&url)
        .await
        .expect("App database connection required")
}

#[tokio::test]
async fn a_rejected_row_does_not_block_a_later_submission() {
    let pool = owner_pool().await;
    let company_name = format!("Pharma Rejected {}", Uuid::now_v7());

    // 1. Insert a rejected company with review_note
    let id1 = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO companies (id, name, status, review_note)
        VALUES ($1, $2, 'rejected', 'Invalid registration')
        "#,
        id1,
        company_name
    )
    .execute(&pool)
    .await
    .expect("Rejected company insert should succeed");

    // 2. Insert another company with the same name as pending
    let id2 = Uuid::now_v7();
    let res = sqlx::query!(
        r#"
        INSERT INTO companies (id, name, status)
        VALUES ($1, $2, 'pending')
        "#,
        id2,
        company_name
    )
    .execute(&pool)
    .await;

    assert!(
        res.is_ok(),
        "Rejected company must not block a later pending submission with the same name: {:?}",
        res.err()
    );
}

#[tokio::test]
async fn an_exact_duplicate_product_is_refused() {
    let pool = owner_pool().await;
    let company_id = Uuid::now_v7();
    let company_name = format!("Company NonMed {}", Uuid::now_v7());

    sqlx::query!(
        "INSERT INTO companies (id, name, status) VALUES ($1, $2, 'approved')",
        company_id,
        company_name
    )
    .execute(&pool)
    .await
    .expect("Company insert");

    let brand = format!("Bandage {}", Uuid::now_v7());

    // First non-medicine (NULL strength and NULL dosage_form_id)
    let p1 = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO products (id, company_id, brand_name, product_type, status)
        VALUES ($1, $2, $3, 'non_medicine', 'approved')
        "#,
        p1,
        company_id,
        brand
    )
    .execute(&pool)
    .await
    .expect("First non-medicine insert should succeed");

    // Second non-medicine with exact same brand and company
    let p2 = Uuid::now_v7();
    let res = sqlx::query!(
        r#"
        INSERT INTO products (id, company_id, brand_name, product_type, status)
        VALUES ($1, $2, $3, 'non_medicine', 'pending')
        "#,
        p2,
        company_id,
        brand
    )
    .execute(&pool)
    .await;

    assert!(
        res.is_err(),
        "NULLS NOT DISTINCT partial unique index must refuse duplicate non-medicine product"
    );
}

#[tokio::test]
async fn merged_into_id_requires_merged_status() {
    let pool = owner_pool().await;
    let c1 = Uuid::now_v7();
    let c2 = Uuid::now_v7();

    sqlx::query!(
        "INSERT INTO companies (id, name, status) VALUES ($1, $2, 'approved')",
        c1,
        format!("Company Target {}", Uuid::now_v7())
    )
    .execute(&pool)
    .await
    .expect("Target company");

    // 1. Pending with merged_into_id set must fail
    let res1 = sqlx::query!(
        "INSERT INTO companies (id, name, status, merged_into_id) VALUES ($1, $2, 'pending', $3)",
        c2,
        format!("Company Source {}", Uuid::now_v7()),
        c1
    )
    .execute(&pool)
    .await;
    assert!(res1.is_err(), "Pending with merged_into_id must fail check constraint");

    // 2. Merged with merged_into_id NULL must fail
    let c3 = Uuid::now_v7();
    let res2 = sqlx::query!(
        "INSERT INTO companies (id, name, status, merged_into_id) VALUES ($1, $2, 'merged', NULL)",
        c3,
        format!("Company Source 2 {}", Uuid::now_v7())
    )
    .execute(&pool)
    .await;
    assert!(res2.is_err(), "Merged with merged_into_id NULL must fail check constraint");

    // 3. Merged with merged_into_id set must succeed
    let res3 = sqlx::query!(
        "INSERT INTO companies (id, name, status, merged_into_id) VALUES ($1, $2, 'merged', $3)",
        c3,
        format!("Company Source 3 {}", Uuid::now_v7()),
        c1
    )
    .execute(&pool)
    .await;
    assert!(res3.is_ok(), "Merged with valid merged_into_id must succeed");
}

#[tokio::test]
async fn a_row_cannot_merge_into_itself() {
    let pool = owner_pool().await;
    let id = Uuid::now_v7();

    let res = sqlx::query!(
        "INSERT INTO companies (id, name, status, merged_into_id) VALUES ($1, $2, 'merged', $1)",
        id,
        format!("Self Merge {}", Uuid::now_v7())
    )
    .execute(&pool)
    .await;

    assert!(res.is_err(), "A row cannot merge into itself (chk_companies_merge_self)");
}

#[tokio::test]
async fn a_medicine_without_a_generic_is_rejected_by_the_database() {
    let pool = owner_pool().await;
    let company_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO companies (id, name, status) VALUES ($1, $2, 'approved')",
        company_id,
        format!("Company Med {}", Uuid::now_v7())
    )
    .execute(&pool)
    .await
    .expect("Company insert");

    let prod_id = Uuid::now_v7();
    let res = sqlx::query!(
        r#"
        INSERT INTO products (id, company_id, generic_id, dosage_form_id, brand_name, product_type, status)
        VALUES ($1, $2, NULL, NULL, 'Invalid Medicine', 'allopathic', 'pending')
        "#,
        prod_id,
        company_id
    )
    .execute(&pool)
    .await;

    assert!(
        res.is_err(),
        "Allopathic medicine without generic and dosage form must fail check constraint"
    );
}

#[tokio::test]
async fn an_issue_report_needs_exactly_one_target() {
    let pool = owner_pool().await;
    let tenant_id = Uuid::now_v7();
    let user_id = Uuid::now_v7();

    let company_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO companies (id, name, status) VALUES ($1, $2, 'approved')",
        company_id,
        format!("Company Target Issue {}", Uuid::now_v7())
    )
    .execute(&pool)
    .await
    .expect("Company insert");

    let generic_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO generics (id, name, status) VALUES ($1, $2, 'approved')",
        generic_id,
        format!("Generic Target Issue {}", Uuid::now_v7())
    )
    .execute(&pool)
    .await
    .expect("Generic insert");

    // 1. Zero targets -> fails
    let res_zero = sqlx::query!(
        r#"
        INSERT INTO catalog_issue_reports (id, tenant_id, user_id, message)
        VALUES ($1, $2, $3, 'No target message')
        "#,
        Uuid::now_v7(),
        tenant_id,
        user_id
    )
    .execute(&pool)
    .await;
    assert!(res_zero.is_err(), "Issue report with 0 targets must fail");

    // 2. Two targets -> fails
    let res_two = sqlx::query!(
        r#"
        INSERT INTO catalog_issue_reports (id, tenant_id, user_id, company_id, generic_id, message)
        VALUES ($1, $2, $3, $4, $5, 'Two targets message')
        "#,
        Uuid::now_v7(),
        tenant_id,
        user_id,
        company_id,
        generic_id
    )
    .execute(&pool)
    .await;
    assert!(res_two.is_err(), "Issue report with 2 targets must fail");

    // 3. Exactly one target -> succeeds
    let res_one = sqlx::query!(
        r#"
        INSERT INTO catalog_issue_reports (id, tenant_id, user_id, company_id, message)
        VALUES ($1, $2, $3, $4, 'One target message')
        "#,
        Uuid::now_v7(),
        tenant_id,
        user_id,
        company_id
    )
    .execute(&pool)
    .await;
    assert!(res_one.is_ok(), "Issue report with 1 target must succeed");
}

#[tokio::test]
async fn a_rejection_without_a_note_is_rejected_by_the_database() {
    let pool = owner_pool().await;
    let id = Uuid::now_v7();

    let res = sqlx::query!(
        "INSERT INTO companies (id, name, status, review_note) VALUES ($1, $2, 'rejected', NULL)",
        id,
        format!("No Note Rejected {}", Uuid::now_v7())
    )
    .execute(&pool)
    .await;

    assert!(res.is_err(), "Rejection without note must fail database check constraint");
}

#[tokio::test]
async fn pos_app_cannot_write_a_lookup_table() {
    let pool = app_pool().await;
    let unit_id = Uuid::now_v7();

    // pos_app has REVOKE UPDATE, DELETE on pack_units and shape B RLS check
    let res_insert = sqlx::query!(
        "INSERT INTO pack_units (id, name) VALUES ($1, $2)",
        unit_id,
        format!("unit_{}", Uuid::now_v7())
    )
    .execute(&pool)
    .await;

    assert!(
        res_insert.is_err(),
        "pos_app without platform token cannot insert into pack_units"
    );

    let res_update = sqlx::query!(
        "UPDATE pack_units SET name = 'hacked' WHERE name = 'piece'"
    )
    .execute(&pool)
    .await;

    assert!(
        res_update.is_err(),
        "pos_app cannot update a lookup table"
    );
}

#[tokio::test]
async fn the_seed_creates_the_base_pack_units() {
    let pool = owner_pool().await;

    let rows = sqlx::query!("SELECT name FROM pack_units ORDER BY name")
        .fetch_all(&pool)
        .await
        .expect("Fetch pack units");

    let names: Vec<String> = rows.into_iter().map(|r| r.name).collect();
    for expected in ["bottle", "box", "piece", "strip", "vial"] {
        assert!(
            names.contains(&expected.to_string()),
            "Seed must include pack unit '{expected}', found: {names:?}"
        );
    }
}
