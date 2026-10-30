#![cfg(feature = "seed")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! The one-time catalogue import must be safe to run twice — `specs/01-catalog/tasks.md`
//! Phase 9, `SPEC.md` §5 Q1.
//!
//! "Idempotent" is the whole reason this is a seeder rather than a migration, so it is asserted
//! rather than claimed: the binary runs twice over the same fixture and the second run must
//! insert nothing. The fixture is four rows, not the real 136 MB file, so this costs a second
//! and exercises the same code path — including the two rules that are easy to get wrong, a
//! blank dosage form recovered from the URL and a duplicate collapsed on the natural key.
//!
//! Needs the owner role: `m0005_catalog` has no platform INSERT policy for `products`, so the
//! import cannot run as `pos_app` at all. `BACKLOG.md` A6.

use sqlx::postgres::PgPoolOptions;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const OWNER_URL: &str = "postgres://pos_owner:devpass@127.0.0.1:5432/medicine_pos";

/// Every row this test writes is prefixed, so cleanup can never touch the real catalogue.
const MARK: &str = "ZZSeedProbe";
const PROBE_MEDEX_ID: i32 = 999_001;

fn owner_url() -> String {
    std::env::var("DATABASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| OWNER_URL.to_string())
}

async fn owner_pool() -> sqlx::PgPool {
    PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(1000))
        .connect(&owner_url())
        .await
        .expect("Owner database connection required")
}

/// Runs before as well as after: a previous failure must not make the first run a no-op and
/// turn this test green for the wrong reason.
async fn cleanup(pool: &sqlx::PgPool) {
    let like = format!("{MARK}%");
    sqlx::query("DELETE FROM products WHERE brand_name LIKE $1")
        .bind(&like)
        .execute(pool)
        .await
        .expect("clean products");
    // `generic_monographs` and `generic_therapeutic_classes` cascade from `generics`.
    sqlx::query("DELETE FROM generics WHERE name LIKE $1")
        .bind(&like)
        .execute(pool)
        .await
        .expect("clean generics");
    sqlx::query("DELETE FROM companies WHERE medex_id = $1 OR name LIKE $2")
        .bind(PROBE_MEDEX_ID)
        .bind(&like)
        .execute(pool)
        .await
        .expect("clean companies");
    sqlx::query("DELETE FROM therapeutic_classes WHERE name LIKE $1")
        .bind(&like)
        .execute(pool)
        .await
        .expect("clean classes");
}

/// The twenty columns `MedicineRow` reads, in header order. Rows are built from this rather
/// than written as comma-counted literals, because a row one field short fails the import with
/// `UnexpectedEndOfRow` and says nothing about which row.
const COLUMNS: &[&str] = &[
    "company_id",
    "brand_name",
    "generic_name",
    "strength",
    "dosage_form",
    "medicine_type",
    "source",
    "source_url",
    "unit_price",
    "indications",
    "description",
    "pharmacology",
    "dosage_administration",
    "interactions",
    "contraindications",
    "side_effects",
    "pregnancy_lactation",
    "precautions",
    "therapeutic_class",
    "storage_conditions",
];

/// One CSV line from `(column, value)` pairs; every column not named is empty.
fn row(fields: &[(&str, &str)]) -> String {
    let cells: Vec<String> = COLUMNS
        .iter()
        .map(|col| {
            let v = fields
                .iter()
                .find(|(c, _)| c == col)
                .map(|(_, v)| *v)
                .unwrap_or("");
            if v.contains(',') || v.contains('"') {
                format!("\"{}\"", v.replace('"', "\"\""))
            } else {
                v.to_owned()
            }
        })
        .collect();
    cells.join(",")
}

fn write_fixture(dir: &Path) {
    std::fs::create_dir_all(dir).expect("fixture dir");

    std::fs::write(
        dir.join("companies.csv"),
        format!("id,company_name,medex_id\n1,{MARK} Pharma Ltd,{PROBE_MEDEX_ID}\n"),
    )
    .expect("write companies.csv");

    let alpha_url =
        format!("https://medex.com.bd/brands/999001/{}-alpha-5-mg-tablet", MARK.to_lowercase());
    let alpha = format!("{MARK} Alpha");
    let beta = format!("{MARK} Beta");
    let testol = format!("{MARK} Testol");
    let gen_one = format!("{MARK} Generic One");
    let gen_two = format!("{MARK} Generic Two");
    let class_a = format!("{MARK} Class A");

    // A blank dosage form, to be recovered from the URL's last word.
    let first = row(&[
        ("company_id", "1"),
        ("brand_name", &alpha),
        ("generic_name", &gen_one),
        ("strength", "5 mg"),
        ("medicine_type", "allopathic"),
        ("source", "medex"),
        ("source_url", &alpha_url),
        ("unit_price", "৳ 8.00,"),
        ("indications", "indications text"),
        ("therapeutic_class", &class_a),
    ]);
    // The same product scraped twice. It collapses, and this fuller copy is the one kept.
    let duplicate = row(&[
        ("company_id", "1"),
        ("brand_name", &alpha),
        ("generic_name", &gen_one),
        ("strength", "5 mg"),
        ("medicine_type", "allopathic"),
        ("source", "medex"),
        ("source_url", &alpha_url),
        ("unit_price", "৳ 8.00,"),
        ("indications", "indications text"),
        ("description", "a description only this copy has"),
        ("therapeutic_class", &class_a),
    ]);
    // A second, distinct product — and a price with a thousands separator.
    let second = row(&[
        ("company_id", "1"),
        ("brand_name", &beta),
        ("generic_name", &gen_two),
        ("strength", "10 mg"),
        ("dosage_form", "Capsule"),
        ("medicine_type", "herbal"),
        ("source", "medex"),
        ("source_url", "https://medex.com.bd/brands/999002/beta"),
        ("unit_price", "৳ 1,250.00"),
    ]);
    // Product Spec §2: a test row is deleted, not imported.
    let test_row = row(&[
        ("company_id", "1"),
        ("brand_name", &testol),
        ("generic_name", &gen_two),
        ("strength", "1 mg"),
        ("dosage_form", "Tablet"),
        ("medicine_type", "allopathic"),
        ("source", "test"),
    ]);

    let body = [COLUMNS.join(","), first, duplicate, second, test_row].join("\n");
    std::fs::write(dir.join("medicines.csv"), body + "\n").expect("write medicines.csv");
}

/// `products inserted    N` out of the binary's report.
fn products_inserted(stdout: &str) -> i64 {
    stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix("products inserted"))
        .and_then(|rest| rest.trim().parse::<i64>().ok())
        .unwrap_or_else(|| panic!("no 'products inserted' line in:\n{stdout}"))
}

fn run_seeder(dir: &Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_catalog-seed"))
        .arg("--dir")
        .arg(dir)
        .arg("--database-url")
        .arg(owner_url())
        .output()
        .expect("run catalog-seed");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.success(),
        "catalog-seed failed: {}\n{stdout}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

#[tokio::test]
async fn running_the_import_twice_inserts_nothing_the_second_time() {
    let pool = owner_pool().await;
    cleanup(&pool).await;

    let dir: PathBuf = std::env::temp_dir().join(format!("catalog-seed-probe-{}", std::process::id()));
    write_fixture(&dir);

    // 1. First run: the two surviving products land, the duplicate collapses, the test row goes.
    let first = run_seeder(&dir);
    assert_eq!(
        products_inserted(&first),
        2,
        "two products should survive four rows — one duplicate pair and one test row:\n{first}"
    );

    // 2. Second run, same input: nothing new. This is the property the whole binary exists for.
    let second = run_seeder(&dir);
    assert_eq!(
        products_inserted(&second),
        0,
        "a second run must insert nothing:\n{second}"
    );

    // 3. And the database agrees — two rows, not four and not two pairs.
    let count: (i64,) = sqlx::query_as("SELECT count(*) FROM products WHERE brand_name LIKE $1")
        .bind(format!("{MARK}%"))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count.0, 2, "the catalogue should hold exactly the two surviving products");

    // 4. The blank dosage form was recovered from the URL's last word, not left null — the
    //    `chk_products_medicine_requires_generic_and_form` constraint would have rejected it.
    let form: (String,) = sqlx::query_as(
        "SELECT f.name FROM products p JOIN dosage_forms f ON f.id = p.dosage_form_id
         WHERE p.brand_name = $1",
    )
    .bind(format!("{MARK} Alpha"))
    .fetch_one(&pool)
    .await
    .expect("the recovered-form product should exist with a dosage form");
    assert_eq!(form.0, "Tablet", "'…-5-mg-tablet' should recover as Tablet");

    // 5. The fuller of the two duplicate copies is the one that survived.
    let desc: (Option<String>,) = sqlx::query_as(
        "SELECT m.description FROM generic_monographs m
         JOIN generics g ON g.id = m.generic_id WHERE g.name = $1",
    )
    .bind(format!("{MARK} Generic One"))
    .fetch_one(&pool)
    .await
    .expect("the generic should carry a monograph");
    assert_eq!(
        desc.0.as_deref(),
        Some("a description only this copy has"),
        "the most complete copy of a duplicated row is the one kept"
    );

    // 6. The price parsed out of `৳ 8.00,` rather than being dropped as unparseable text.
    let mrp: (Option<rust_decimal::Decimal>,) =
        sqlx::query_as("SELECT reference_unit_mrp FROM products WHERE brand_name = $1")
            .bind(format!("{MARK} Alpha"))
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        mrp.0,
        Some(rust_decimal::Decimal::from_str_exact("8.00").unwrap()),
        "`৳ 8.00,` is a reference MRP of 8.00"
    );

    cleanup(&pool).await;
    let _ = std::fs::remove_dir_all(&dir);
}
