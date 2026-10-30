//! The one-time catalogue import — [`specs/01-catalog/SPEC.md`] §5 **Q1**.
//!
//! Loads the medex scrape (`companies.csv`, `medicines.csv`) into the central catalogue.
//! **Idempotent by design**: every row is matched on a natural key that already exists in the
//! schema, so running it twice inserts nothing the second time, and running it against a
//! half-finished import finishes the job rather than duplicating it. That is the whole reason
//! this is a seeder and not a migration — Article V.4 makes an applied migration immutable,
//! and a 136 MB data load that may need re-running is the opposite of immutable.
//!
//! ```bash
//! # dev
//! cargo run -p catalog --features seed --bin catalog-seed -- --dir "Extra Docs"
//! # production, against the owner role
//! DATABASE_URL=postgres://pos_owner:…@host/medicine_pos \
//!   cargo run --release -p catalog --features seed --bin catalog-seed -- --dir /srv/medex
//! ```
//!
//! # Why this needs the owner role
//!
//! `m0005_catalog`'s `companies_insert`, `generics_insert` and `products_insert` policies each
//! read `status = 'pending' AND submitted_by_tenant_id = app.current_tenant_id()`. There is
//! **no platform INSERT path** for the three submittable tables — the review flow inserts as a
//! tenant and then *updates* to `approved`. An imported row is `approved` from birth with no
//! submitting tenant, which no policy permits. So the import runs as `pos_owner`, which
//! bypasses RLS, exactly as the migrator does. `pos_app` is never used here and Article IV.1 is
//! untouched: the *application* still connects as a non-owner. Recorded as a finding in
//! `specs/01-catalog/BACKLOG.md` A6, because it also means a super admin cannot add a product
//! directly through the application at all.
//!
//! # The nine quality fixes
//!
//! Product Specification §2 specifies the handling; this is where it is implemented.
//!
//! | Problem | What happens here |
//! |---|---|
//! | Blank dosage form, 5,237 rows | Recovered from the last word of `source_url`. Verified to reproduce the spec's own counts: tablet 2,557, capsule 706, injection 540 |
//! | Test rows | The single `source = 'test'` row (`Testol`) is dropped before anything else |
//! | Prices as text | `৳ 8.00,` / `৳ 59.03,(4's pack: ৳ 236.12)` / `৳ 1,250.00` → the first amount, thousands separators removed. 24,626 parse, 0 fail |
//! | Stray whitespace | Every field is trimmed on read |
//! | Duplicate rows | Collapsed on the product's natural key, keeping the most complete copy |
//! | Empty columns | `dar_number`, company `address`, `license_status`, `dgda_post_id` are never read |
//! | Images | Never read — 4 rows carry any, 3 of them test uploads |
//! | Medical text | Stored once per generic, most common variant wins |
//! | Therapeutic classes | Comma-split, attached to the generic rather than the product |
//!
//! # Two places the source data contradicts the specification
//!
//! Both are measured, not assumed, and both are recorded in `specs/INDEX.md`.
//!
//! 1. **`products.medex_id` is left NULL.** Product Spec §2 says to de-duplicate on "the medex
//!    numeric id inside the URL" and sizes the problem at *40 pairs*. In the real file 3,103
//!    medex ids cover more than one product — differing by strength (990), by dosage form
//!    (1,027) or by brand name (219) — because one medex brand page lists several products.
//!    Only 867 are true duplicates. Since `idx_products_medex_id` is UNIQUE, writing that id
//!    would reject roughly 2,250 legitimate products. Identity therefore comes from the key
//!    the spec itself defines — "company + brand + strength + dosage form" — which is already
//!    `idx_products_unique`.
//! 2. **`product_packs` is not seeded.** §4 says central defaults exist "where the source data
//!    knows the box size (about 80% of rows)". Measured, `(N's pack: …)` matches 15,162 of
//!    30,670 rows — 49% — across 40-plus formats, and "Tenants define pack sizes" is
//!    `[Decided]`. A guess written into a table every pharmacy reads is worse than an empty
//!    one, so this leaves it empty. `BACKLOG.md` A7.
//!
//! `ingredients` and `generic_ingredients` are also left empty: the CSV has no ingredient
//! column, and splitting `generic_name` on `+` yields 2,246 "ingredients" including
//! *"23 herb formula to increase breast milk"* and *"20% Fat Emulsion"*. `BACKLOG.md` A8.

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

use rust_decimal::Decimal;
use serde::Deserialize;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

type Res<T> = Result<T, Box<dyn Error>>;

/// Rows per `UNNEST` statement. Large enough that 30,000 products are a handful of round
/// trips, small enough that no single statement carries a megabyte of parameters.
const CHUNK: usize = 4_000;

// ---------------------------------------------------------------------------------------------
// CSV shapes
// ---------------------------------------------------------------------------------------------

/// `companies.csv`. The columns this does not name — `address`, `license_status`,
/// `dgda_post_id`, `generics_count`, `brands_count`, `sources`, `medex_slug`, `medex_url` — are
/// the ones Product Spec §2 says to drop.
#[derive(Debug, Deserialize)]
struct CompanyRow {
    id: String,
    company_name: String,
    medex_id: String,
}

/// `medicines.csv`, 27 columns of which these are the ones that reach the database.
#[derive(Debug, Deserialize)]
struct MedicineRow {
    company_id: String,
    brand_name: String,
    generic_name: String,
    strength: String,
    dosage_form: String,
    medicine_type: String,
    source: String,
    source_url: String,
    unit_price: String,
    indications: String,
    description: String,
    pharmacology: String,
    dosage_administration: String,
    interactions: String,
    contraindications: String,
    side_effects: String,
    pregnancy_lactation: String,
    precautions: String,
    therapeutic_class: String,
    storage_conditions: String,
}

/// The ten monograph fields, in `generic_monographs` column order.
type Monograph = [Option<String>; 10];

// ---------------------------------------------------------------------------------------------
// In-memory catalogue, built before anything is written
// ---------------------------------------------------------------------------------------------

#[derive(Debug)]
struct Product {
    company_key: String,
    brand_name: String,
    strength: Option<String>,
    dosage_form: String,
    generic_name: String,
    product_type: String,
    reference_unit_mrp: Option<Decimal>,
    /// How many of the fields this row filled in. Decides which copy survives a duplicate.
    completeness: u8,
}

#[derive(Debug, Default)]
struct Catalogue {
    /// medex id → company name, keyed by `companies.csv`'s own `id` column.
    companies: HashMap<String, (String, Option<i32>)>,
    dosage_forms: HashSet<String>,
    therapeutic_classes: HashSet<String>,
    /// generic name → its therapeutic classes.
    generic_classes: HashMap<String, HashSet<String>>,
    /// generic name → variant hash → (how many products carry it, the text itself).
    monographs: HashMap<String, HashMap<u64, (u32, Monograph)>>,
    /// Natural key → the surviving row.
    products: HashMap<(String, String, Option<String>, String), Product>,
    skipped_test_rows: usize,
    collapsed_duplicates: usize,
    recovered_forms: usize,
}

// ---------------------------------------------------------------------------------------------
// Field helpers — Product Spec §2, "Stray whitespace: trim on import"
// ---------------------------------------------------------------------------------------------

/// Trimmed, with the empty string treated as absent. Every field goes through this.
fn opt(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_owned())
    }
}

/// `৳ 8.00,` → `8.00`; `৳ 59.03,(4's pack: ৳ 236.12)` → `59.03`; `৳ 1,250.00` → `1250.00`.
///
/// The first amount is the unit price; anything after it describes a pack, which §4 leaves to
/// the tenant. Imported prices are a reference MRP only, never a price `[D-34]`.
fn parse_price(raw: &str) -> Option<Decimal> {
    let digits: String = raw
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
        .filter(|c| *c != ',')
        .collect();
    // A trailing separator survives the filter above: `৳ 8.00,` ends in `.` only if the source
    // did, and `Decimal` rejects that, so trim it rather than returning None.
    let cleaned = digits.trim_end_matches('.');
    if cleaned.is_empty() {
        return None;
    }
    Decimal::from_str_exact(cleaned).ok()
}

/// Product Spec §2: *"Blank dosage form — recover from the last word of `source_url`"*.
///
/// `…/brands/34627/evania-m-5-mg-tablet` → `Tablet`. Verified against the spec's own counts.
fn form_from_url(url: &str) -> Option<String> {
    let slug = url.trim_end_matches('/').rsplit('/').next()?;
    let word = slug.rsplit('-').next()?;
    if word.is_empty() || word.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut chars = word.chars();
    let first = chars.next()?;
    Some(first.to_uppercase().collect::<String>() + chars.as_str())
}

/// The base unit a dosage form is counted in — Product Spec §4, *"Base unit (piece, bottle,
/// vial) is stored on the dosage form, because it depends on the form, not the product."*
///
/// Order matters: `Powder for Suspension` is a bottle you reconstitute, not a sachet, and
/// `IV Infusion` is a vial rather than the bottle `Solution` alone would suggest.
fn base_unit_for(form: &str) -> &'static str {
    let f = form.to_ascii_lowercase();
    const VIAL: &[&str] = &["injection", "infusion", "ampoule", "vial", "iv ", "im/"];
    const BOTTLE: &[&str] = &[
        "syrup",
        "suspension",
        "solution",
        "drop",
        "elixir",
        "liquid",
        "mouthwash",
        "lotion",
        "shampoo",
        "spray",
        "inhaler",
    ];
    const TUBE: &[&str] = &["cream", "ointment", "gel", "paste", "balm"];
    const SACHET: &[&str] = &["powder", "sachet", "granule"];

    if VIAL.iter().any(|k| f.contains(k)) {
        "Vial"
    } else if BOTTLE.iter().any(|k| f.contains(k)) {
        "Bottle"
    } else if TUBE.iter().any(|k| f.contains(k)) {
        "Tube"
    } else if SACHET.iter().any(|k| f.contains(k)) {
        "Sachet"
    } else {
        "Piece"
    }
}

/// The pack units every dosage form resolves to. `Piece` is the base unit stock is counted in
/// `[D-33]`; the rest are the containers Product Spec §4 names.
const PACK_UNITS: &[&str] = &["Piece", "Strip", "Box", "Bottle", "Vial", "Tube", "Sachet"];

fn hash_of(m: &Monograph) -> u64 {
    let mut h = DefaultHasher::new();
    m.hash(&mut h);
    h.finish()
}

// ---------------------------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------------------------

fn read_companies(path: &Path, cat: &mut Catalogue) -> Res<()> {
    let mut rdr = csv::Reader::from_path(path)?;
    for row in rdr.deserialize::<CompanyRow>() {
        let row = row?;
        let Some(name) = opt(&row.company_name) else {
            continue;
        };
        // Product Spec §2 names "Test Pharma Ltd" among the rows to delete.
        if name.eq_ignore_ascii_case("Test Pharma Ltd") {
            cat.skipped_test_rows += 1;
            continue;
        }
        let medex = opt(&row.medex_id).and_then(|v| v.parse::<i32>().ok());
        cat.companies.insert(row.id.trim().to_owned(), (name, medex));
    }
    Ok(())
}

fn read_medicines(path: &Path, cat: &mut Catalogue) -> Res<()> {
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_path(path)?;
    for row in rdr.deserialize::<MedicineRow>() {
        let row = row?;

        // Product Spec §2: test rows are deleted, not imported. One row carries `source = test`,
        // and it is also the only row with no generic and no URL.
        if row.source.trim() == "test" {
            cat.skipped_test_rows += 1;
            continue;
        }

        let Some(brand_name) = opt(&row.brand_name) else {
            continue;
        };
        let Some(generic_name) = opt(&row.generic_name) else {
            continue;
        };
        let company_key = row.company_id.trim().to_owned();
        if !cat.companies.contains_key(&company_key) {
            continue;
        }

        // The CHECK constraint on `products` requires a dosage form for anything that is not a
        // non-medicine, so a blank one is recovered rather than dropped.
        let dosage_form = match opt(&row.dosage_form) {
            Some(f) => f,
            None => match form_from_url(&row.source_url) {
                Some(f) => {
                    cat.recovered_forms += 1;
                    f
                }
                None => continue,
            },
        };

        let product_type = match row.medicine_type.trim() {
            "herbal" => "herbal",
            "" => continue,
            _ => "allopathic",
        };

        cat.dosage_forms.insert(dosage_form.clone());

        // Product Spec §3: *"each generic takes all the classes found on its products"*, and the
        // cell is comma-separated with several per product.
        let classes = cat.generic_classes.entry(generic_name.clone()).or_default();
        for raw in row.therapeutic_class.split(',') {
            if let Some(c) = opt(raw) {
                cat.therapeutic_classes.insert(c.clone());
                classes.insert(c);
            }
        }

        // 127 MB of the 136 MB is this text, and it is identical for every product sharing a
        // generic except in 75 cases. Only one copy of each distinct variant is held in memory.
        let mono: Monograph = [
            opt(&row.indications),
            opt(&row.description),
            opt(&row.pharmacology),
            opt(&row.dosage_administration),
            opt(&row.interactions),
            opt(&row.contraindications),
            opt(&row.side_effects),
            opt(&row.pregnancy_lactation),
            opt(&row.precautions),
            opt(&row.storage_conditions),
        ];
        let filled = u8::try_from(mono.iter().filter(|f| f.is_some()).count()).unwrap_or(u8::MAX);
        let key = hash_of(&mono);
        let variants = cat.monographs.entry(generic_name.clone()).or_default();
        match variants.get_mut(&key) {
            Some((count, _)) => *count += 1,
            None => {
                variants.insert(key, (1, mono));
            }
        }

        let strength = opt(&row.strength);
        let product = Product {
            company_key: company_key.clone(),
            brand_name: brand_name.clone(),
            strength: strength.clone(),
            dosage_form: dosage_form.clone(),
            generic_name,
            product_type: product_type.to_owned(),
            reference_unit_mrp: parse_price(&row.unit_price),
            completeness: filled,
        };

        // "Same product scraped twice: keep the complete copy." The natural key is the one the
        // spec defines and the schema already enforces.
        let nk = (company_key, brand_name, strength, dosage_form);
        match cat.products.get(&nk) {
            Some(existing) => {
                cat.collapsed_duplicates += 1;
                if product.completeness > existing.completeness {
                    cat.products.insert(nk, product);
                }
            }
            None => {
                cat.products.insert(nk, product);
            }
        }
    }
    Ok(())
}

/// The variant carried by the most products wins — Product Spec, *"the import keeps the most
/// common version"*. Ties break on the longest text so two runs cannot disagree.
fn winning_monograph(variants: &HashMap<u64, (u32, Monograph)>) -> Option<&Monograph> {
    variants
        .values()
        .max_by_key(|(count, mono)| {
            let len: usize = mono.iter().flatten().map(String::len).sum();
            (*count, len)
        })
        .map(|(_, mono)| mono)
}

// ---------------------------------------------------------------------------------------------
// Writing — every step inserts only what is missing, so a second run is a no-op
// ---------------------------------------------------------------------------------------------

/// `pack_units` and `therapeutic_classes` are the same shape — read what is there, insert what
/// is missing, return the complete map. The `SELECT` before the `INSERT` is what makes this
/// idempotent: nothing is written for a name already present, whoever wrote it.
///
/// Two near-identical functions rather than one generic over the table name, because sqlx 0.9
/// refuses a `format!`-built query outright (`SqlSafeStr`). Bypassing that with
/// `AssertSqlSafe` to save nine lines would be the wrong trade in a crate that touches a
/// shared catalogue.
async fn sync_pack_units(
    tx: &mut Transaction<'_, Postgres>,
    wanted: &HashSet<String>,
) -> Res<HashMap<String, Uuid>> {
    let existing: Vec<(Uuid, String)> = sqlx::query_as("SELECT id, name FROM pack_units")
        .fetch_all(&mut **tx)
        .await?;
    let mut map: HashMap<String, Uuid> = existing.into_iter().map(|(id, n)| (n, id)).collect();

    let missing: Vec<&String> = wanted.iter().filter(|n| !map.contains_key(*n)).collect();
    for batch in missing.chunks(CHUNK) {
        let ids: Vec<Uuid> = batch.iter().map(|_| Uuid::now_v7()).collect();
        let names: Vec<String> = batch.iter().map(|n| (*n).clone()).collect();
        sqlx::query(
            "INSERT INTO pack_units (id, name) SELECT * FROM UNNEST($1::uuid[], $2::varchar[])
             ON CONFLICT (name) DO NOTHING",
        )
        .bind(&ids)
        .bind(&names)
        .execute(&mut **tx)
        .await?;
        for (id, name) in ids.into_iter().zip(names) {
            map.insert(name, id);
        }
    }
    Ok(map)
}

async fn sync_therapeutic_classes(
    tx: &mut Transaction<'_, Postgres>,
    wanted: &HashSet<String>,
) -> Res<HashMap<String, Uuid>> {
    let existing: Vec<(Uuid, String)> = sqlx::query_as("SELECT id, name FROM therapeutic_classes")
        .fetch_all(&mut **tx)
        .await?;
    let mut map: HashMap<String, Uuid> = existing.into_iter().map(|(id, n)| (n, id)).collect();

    let missing: Vec<&String> = wanted.iter().filter(|n| !map.contains_key(*n)).collect();
    for batch in missing.chunks(CHUNK) {
        let ids: Vec<Uuid> = batch.iter().map(|_| Uuid::now_v7()).collect();
        let names: Vec<String> = batch.iter().map(|n| (*n).clone()).collect();
        sqlx::query(
            "INSERT INTO therapeutic_classes (id, name)
             SELECT * FROM UNNEST($1::uuid[], $2::varchar[])
             ON CONFLICT (name) DO NOTHING",
        )
        .bind(&ids)
        .bind(&names)
        .execute(&mut **tx)
        .await?;
        for (id, name) in ids.into_iter().zip(names) {
            map.insert(name, id);
        }
    }
    Ok(map)
}

async fn sync_dosage_forms(
    tx: &mut Transaction<'_, Postgres>,
    wanted: &HashSet<String>,
    units: &HashMap<String, Uuid>,
) -> Res<HashMap<String, Uuid>> {
    let existing: Vec<(Uuid, String)> = sqlx::query_as("SELECT id, name FROM dosage_forms")
        .fetch_all(&mut **tx)
        .await?;
    let mut map: HashMap<String, Uuid> = existing.into_iter().map(|(id, n)| (n, id)).collect();

    let missing: Vec<&String> = wanted.iter().filter(|n| !map.contains_key(*n)).collect();
    for batch in missing.chunks(CHUNK) {
        let mut ids = Vec::with_capacity(batch.len());
        let mut names = Vec::with_capacity(batch.len());
        let mut base = Vec::with_capacity(batch.len());
        for name in batch {
            let unit = base_unit_for(name);
            let Some(unit_id) = units.get(unit) else {
                return Err(format!("pack unit '{unit}' is missing for form '{name}'").into());
            };
            ids.push(Uuid::now_v7());
            names.push((*name).clone());
            base.push(*unit_id);
        }
        sqlx::query(
            "INSERT INTO dosage_forms (id, name, base_unit_id)
             SELECT * FROM UNNEST($1::uuid[], $2::varchar[], $3::uuid[])
             ON CONFLICT (name) DO NOTHING",
        )
        .bind(&ids)
        .bind(&names)
        .bind(&base)
        .execute(&mut **tx)
        .await?;
        for (id, name) in ids.into_iter().zip(names) {
            map.insert(name, id);
        }
    }
    Ok(map)
}

/// Companies are matched on `medex_id` where the source has one and on `name` otherwise —
/// `idx_companies_medex_id` and `idx_companies_name` are both unique, so both must be respected
/// or a re-run trips the one it did not check.
async fn sync_companies(
    tx: &mut Transaction<'_, Postgres>,
    cat: &Catalogue,
) -> Res<HashMap<String, Uuid>> {
    let existing: Vec<(Uuid, String, Option<i32>)> =
        sqlx::query_as("SELECT id, name, medex_id FROM companies WHERE status IN ('pending', 'approved')")
            .fetch_all(&mut **tx)
            .await?;
    let mut by_name: HashMap<String, Uuid> = HashMap::new();
    let mut by_medex: HashMap<i32, Uuid> = HashMap::new();
    for (id, name, medex) in existing {
        by_name.insert(name, id);
        if let Some(m) = medex {
            by_medex.insert(m, id);
        }
    }

    // csv id → the row's uuid, which is what `products.company_id` needs.
    let mut resolved: HashMap<String, Uuid> = HashMap::new();
    let mut new_rows: Vec<(String, String, Option<i32>, Uuid)> = Vec::new();

    for (csv_id, (name, medex)) in &cat.companies {
        let found = medex
            .and_then(|m| by_medex.get(&m).copied())
            .or_else(|| by_name.get(name).copied());
        match found {
            Some(id) => {
                resolved.insert(csv_id.clone(), id);
            }
            None => {
                let id = Uuid::now_v7();
                resolved.insert(csv_id.clone(), id);
                new_rows.push((csv_id.clone(), name.clone(), *medex, id));
                by_name.insert(name.clone(), id);
                if let Some(m) = medex {
                    by_medex.insert(*m, id);
                }
            }
        }
    }

    for batch in new_rows.chunks(CHUNK) {
        let ids: Vec<Uuid> = batch.iter().map(|r| r.3).collect();
        let names: Vec<String> = batch.iter().map(|r| r.1.clone()).collect();
        let medex: Vec<Option<i32>> = batch.iter().map(|r| r.2).collect();
        sqlx::query(
            "INSERT INTO companies (id, name, medex_id, status)
             SELECT u.id, u.name, u.medex_id, 'approved'
             FROM UNNEST($1::uuid[], $2::varchar[], $3::int[]) AS u(id, name, medex_id)
             ON CONFLICT (medex_id) WHERE medex_id IS NOT NULL DO NOTHING",
        )
        .bind(&ids)
        .bind(&names)
        .bind(&medex)
        .execute(&mut **tx)
        .await?;
    }
    Ok(resolved)
}

/// Generics have no medex id in the source, so `name` is the key —
/// `idx_generics_name ... WHERE status IN ('pending','approved')`.
async fn sync_generics(
    tx: &mut Transaction<'_, Postgres>,
    wanted: &HashSet<String>,
) -> Res<HashMap<String, Uuid>> {
    let existing: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT id, name FROM generics WHERE status IN ('pending', 'approved')")
            .fetch_all(&mut **tx)
            .await?;
    let mut map: HashMap<String, Uuid> = existing.into_iter().map(|(id, n)| (n, id)).collect();

    let missing: Vec<&String> = wanted.iter().filter(|n| !map.contains_key(*n)).collect();
    for batch in missing.chunks(CHUNK) {
        let ids: Vec<Uuid> = batch.iter().map(|_| Uuid::now_v7()).collect();
        let names: Vec<String> = batch.iter().map(|n| (*n).clone()).collect();
        sqlx::query(
            "INSERT INTO generics (id, name, status)
             SELECT u.id, u.name, 'approved' FROM UNNEST($1::uuid[], $2::varchar[]) AS u(id, name)
             ON CONFLICT (name) WHERE status IN ('pending', 'approved') DO NOTHING",
        )
        .bind(&ids)
        .bind(&names)
        .execute(&mut **tx)
        .await?;
        for (id, name) in ids.into_iter().zip(names) {
            map.insert(name, id);
        }
    }
    Ok(map)
}

/// A monograph is one row per generic, so a re-run **refreshes** it rather than skipping it: if
/// the source file changes, the text should follow. That is still idempotent — two runs over
/// the same file leave the same rows.
async fn sync_monographs(
    tx: &mut Transaction<'_, Postgres>,
    cat: &Catalogue,
    generics: &HashMap<String, Uuid>,
) -> Res<usize> {
    let mut rows: Vec<(Uuid, &Monograph)> = Vec::new();
    for (name, variants) in &cat.monographs {
        let Some(id) = generics.get(name) else {
            continue;
        };
        if let Some(mono) = winning_monograph(variants) {
            if mono.iter().any(Option::is_some) {
                rows.push((*id, mono));
            }
        }
    }

    let mut written = 0;
    for batch in rows.chunks(CHUNK) {
        let ids: Vec<Uuid> = batch.iter().map(|r| r.0).collect();
        let col = |i: usize| -> Vec<Option<String>> {
            batch.iter().map(|r| r.1[i].clone()).collect()
        };
        let res = sqlx::query(
            "INSERT INTO generic_monographs (
                 generic_id, indications, description, pharmacology, dosage, interactions,
                 contraindications, side_effects, pregnancy_and_lactation, precautions, storage)
             SELECT * FROM UNNEST(
                 $1::uuid[], $2::text[], $3::text[], $4::text[], $5::text[], $6::text[],
                 $7::text[], $8::text[], $9::text[], $10::text[], $11::text[])
             ON CONFLICT (generic_id) DO UPDATE SET
                 indications = EXCLUDED.indications,
                 description = EXCLUDED.description,
                 pharmacology = EXCLUDED.pharmacology,
                 dosage = EXCLUDED.dosage,
                 interactions = EXCLUDED.interactions,
                 contraindications = EXCLUDED.contraindications,
                 side_effects = EXCLUDED.side_effects,
                 pregnancy_and_lactation = EXCLUDED.pregnancy_and_lactation,
                 precautions = EXCLUDED.precautions,
                 storage = EXCLUDED.storage,
                 updated_at = now()",
        )
        .bind(&ids)
        .bind(col(0))
        .bind(col(1))
        .bind(col(2))
        .bind(col(3))
        .bind(col(4))
        .bind(col(5))
        .bind(col(6))
        .bind(col(7))
        .bind(col(8))
        .bind(col(9))
        .execute(&mut **tx)
        .await?;
        written += usize::try_from(res.rows_affected()).unwrap_or(0);
    }
    Ok(written)
}

async fn sync_generic_classes(
    tx: &mut Transaction<'_, Postgres>,
    cat: &Catalogue,
    generics: &HashMap<String, Uuid>,
    classes: &HashMap<String, Uuid>,
) -> Res<usize> {
    let mut gids: Vec<Uuid> = Vec::new();
    let mut cids: Vec<Uuid> = Vec::new();
    for (generic, names) in &cat.generic_classes {
        let Some(gid) = generics.get(generic) else {
            continue;
        };
        for name in names {
            if let Some(cid) = classes.get(name) {
                gids.push(*gid);
                cids.push(*cid);
            }
        }
    }

    let mut written = 0;
    for (g, c) in gids.chunks(CHUNK).zip(cids.chunks(CHUNK)) {
        let res = sqlx::query(
            "INSERT INTO generic_therapeutic_classes (generic_id, therapeutic_class_id)
             SELECT * FROM UNNEST($1::uuid[], $2::uuid[])
             ON CONFLICT (generic_id, therapeutic_class_id) DO NOTHING",
        )
        .bind(g)
        .bind(c)
        .execute(&mut **tx)
        .await?;
        written += usize::try_from(res.rows_affected()).unwrap_or(0);
    }
    Ok(written)
}

/// Products are matched on `(company_id, brand_name, strength, dosage_form_id)` — Product Spec
/// §3's *"a product is identified by company + brand + strength + dosage form, never by brand
/// name alone"*, which is `idx_products_unique`. `medex_id` is deliberately NULL; see the module
/// comment for the measurement that forced it.
async fn sync_products(
    tx: &mut Transaction<'_, Postgres>,
    cat: &Catalogue,
    companies: &HashMap<String, Uuid>,
    generics: &HashMap<String, Uuid>,
    forms: &HashMap<String, Uuid>,
) -> Res<usize> {
    let existing: Vec<(Uuid, String, Option<String>, Option<Uuid>)> = sqlx::query_as(
        "SELECT company_id, brand_name, strength, dosage_form_id FROM products
         WHERE status IN ('pending', 'approved')",
    )
    .fetch_all(&mut **tx)
    .await?;
    let present: HashSet<(Uuid, String, Option<String>, Option<Uuid>)> =
        existing.into_iter().collect();

    let mut ids = Vec::new();
    let mut company_ids = Vec::new();
    let mut generic_ids = Vec::new();
    let mut form_ids = Vec::new();
    let mut brands = Vec::new();
    let mut strengths = Vec::new();
    let mut types = Vec::new();
    let mut mrps = Vec::new();

    for p in cat.products.values() {
        let (Some(cid), Some(gid), Some(fid)) = (
            companies.get(&p.company_key),
            generics.get(&p.generic_name),
            forms.get(&p.dosage_form),
        ) else {
            continue;
        };
        if present.contains(&(*cid, p.brand_name.clone(), p.strength.clone(), Some(*fid))) {
            continue;
        }
        ids.push(Uuid::now_v7());
        company_ids.push(*cid);
        generic_ids.push(*gid);
        form_ids.push(*fid);
        brands.push(p.brand_name.clone());
        strengths.push(p.strength.clone());
        types.push(p.product_type.clone());
        mrps.push(p.reference_unit_mrp);
    }

    let mut written = 0;
    let total = ids.len();
    let mut start = 0;
    while start < total {
        let end = (start + CHUNK).min(total);
        let res = sqlx::query(
            "INSERT INTO products (
                 id, company_id, generic_id, dosage_form_id, brand_name, strength,
                 product_type, reference_unit_mrp, status)
             SELECT u.id, u.company_id, u.generic_id, u.dosage_form_id, u.brand_name,
                    u.strength, u.product_type, u.reference_unit_mrp, 'approved'
             FROM UNNEST($1::uuid[], $2::uuid[], $3::uuid[], $4::uuid[], $5::varchar[],
                         $6::varchar[], $7::varchar[], $8::numeric[])
                  AS u(id, company_id, generic_id, dosage_form_id, brand_name, strength,
                       product_type, reference_unit_mrp)
             ON CONFLICT (company_id, brand_name, strength, dosage_form_id)
                 WHERE status IN ('pending', 'approved') DO NOTHING",
        )
        .bind(&ids[start..end])
        .bind(&company_ids[start..end])
        .bind(&generic_ids[start..end])
        .bind(&form_ids[start..end])
        .bind(&brands[start..end])
        .bind(&strengths[start..end])
        .bind(&types[start..end])
        .bind(&mrps[start..end])
        .execute(&mut **tx)
        .await?;
        written += usize::try_from(res.rows_affected()).unwrap_or(0);
        start = end;
    }
    Ok(written)
}

// ---------------------------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------------------------

struct Args {
    dir: PathBuf,
    /// Absent is only legal with `--dry-run`, which never opens a connection.
    database_url: Option<String>,
    dry_run: bool,
}

fn parse_args() -> Res<Args> {
    let mut dir = PathBuf::from("Extra Docs");
    let mut url = None;
    let mut dry_run = false;

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--dir" => dir = PathBuf::from(it.next().ok_or("--dir needs a path")?),
            "--database-url" => url = Some(it.next().ok_or("--database-url needs a value")?),
            "--dry-run" => dry_run = true,
            "--help" | "-h" => {
                println!(
                    "catalog-seed — the one-time catalogue import (idempotent)\n\n\
                     --dir <path>           directory holding companies.csv and medicines.csv\n\
                     --database-url <url>   defaults to $DATABASE_URL; must be the owner role\n\
                     --dry-run              parse and report, write nothing"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument '{other}' — try --help").into()),
        }
    }

    let database_url = url.or_else(|| std::env::var("DATABASE_URL").ok());
    if database_url.is_none() && !dry_run {
        return Err("set --database-url or DATABASE_URL (the owner role)".into());
    }
    Ok(Args { dir, database_url, dry_run })
}

async fn connect(url: &str) -> Res<PgPool> {
    let pool = PgPoolOptions::new().max_connections(1).connect(url).await?;
    Ok(pool)
}

#[tokio::main]
async fn main() -> Res<()> {
    let args = parse_args()?;
    let companies_csv = args.dir.join("companies.csv");
    let medicines_csv = args.dir.join("medicines.csv");
    for f in [&companies_csv, &medicines_csv] {
        if !f.exists() {
            return Err(format!("{} not found", f.display()).into());
        }
    }

    let mut cat = Catalogue::default();
    println!("reading {}", companies_csv.display());
    read_companies(&companies_csv, &mut cat)?;
    println!("reading {} — 136 MB, this takes a moment", medicines_csv.display());
    read_medicines(&medicines_csv, &mut cat)?;

    let varying = cat.monographs.values().filter(|v| v.len() > 1).count();
    println!(
        "\nparsed\n  companies            {}\n  generics             {}\n  \
         dosage forms         {}\n  therapeutic classes  {}\n  products             {}\n  \
         test rows dropped    {}\n  duplicates collapsed {}\n  forms recovered      {}\n  \
         generics with conflicting medical text {varying} (most common copy wins)",
        cat.companies.len(),
        cat.monographs.len(),
        cat.dosage_forms.len(),
        cat.therapeutic_classes.len(),
        cat.products.len(),
        cat.skipped_test_rows,
        cat.collapsed_duplicates,
        cat.recovered_forms,
    );

    if args.dry_run {
        println!("\n--dry-run: nothing written");
        return Ok(());
    }

    let url = args
        .database_url
        .ok_or("set --database-url or DATABASE_URL (the owner role)")?;
    let pool = connect(&url).await?;
    let mut tx = pool.begin().await?;

    let units: HashSet<String> = PACK_UNITS.iter().map(|s| (*s).to_string()).collect();
    let unit_ids = sync_pack_units(&mut tx, &units).await?;
    let form_ids = sync_dosage_forms(&mut tx, &cat.dosage_forms, &unit_ids).await?;
    let class_ids = sync_therapeutic_classes(&mut tx, &cat.therapeutic_classes).await?;
    let company_ids = sync_companies(&mut tx, &cat).await?;
    let generic_names: HashSet<String> = cat.monographs.keys().cloned().collect();
    let generic_ids = sync_generics(&mut tx, &generic_names).await?;
    let monographs = sync_monographs(&mut tx, &cat, &generic_ids).await?;
    let links = sync_generic_classes(&mut tx, &cat, &generic_ids, &class_ids).await?;
    let products = sync_products(&mut tx, &cat, &company_ids, &generic_ids, &form_ids).await?;

    tx.commit().await?;

    println!(
        "\nwritten this run\n  monographs refreshed {monographs} (rewritten every run, by design)\n  \
         generic↔class links  {links}\n  products inserted    {products}\n\n\
         Run it again and every count but the monographs is 0 — that is the point."
    );
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// The three rules from Product Specification §2 that are pure functions, so they are testable
// without a database or a 136 MB file.
// ---------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_price_is_the_first_amount_and_nothing_after_it() {
        let d = |s: &str| Decimal::from_str_exact(s).expect("test literal");
        assert_eq!(parse_price("৳ 8.00,"), Some(d("8.00")));
        assert_eq!(parse_price("৳ 59.03,(4's pack: ৳ 236.12)"), Some(d("59.03")));
        assert_eq!(parse_price("৳ 1,250.00"), Some(d("1250.00")));
        assert_eq!(parse_price("৳ 7.00 (1 x 15: ৳ 105.00)"), Some(d("7.00")));
        assert_eq!(parse_price("৳ 250.00"), Some(d("250.00")));
        assert_eq!(parse_price(""), None);
        assert_eq!(parse_price("Price Unavailable"), None);
    }

    #[test]
    fn a_blank_dosage_form_is_recovered_from_the_last_word_of_the_url() {
        assert_eq!(
            form_from_url("https://medex.com.bd/brands/34627/evania-m-5-mg-tablet"),
            Some("Tablet".to_owned())
        );
        assert_eq!(
            form_from_url("https://medex.com.bd/brands/1/x-injection/"),
            Some("Injection".to_owned())
        );
        // A trailing number is a strength, not a form — better absent than wrong.
        assert_eq!(form_from_url("https://medex.com.bd/brands/1/aspirin-500"), None);
        assert_eq!(form_from_url(""), None);
    }

    #[test]
    fn a_dosage_form_resolves_to_the_container_it_is_sold_in() {
        // The two that a naive keyword order gets wrong.
        assert_eq!(base_unit_for("Powder for Suspension"), "Bottle");
        assert_eq!(base_unit_for("IV Infusion"), "Vial");

        assert_eq!(base_unit_for("Tablet"), "Piece");
        assert_eq!(base_unit_for("Capsule (Delayed Release)"), "Piece");
        assert_eq!(base_unit_for("Ophthalmic Solution"), "Bottle");
        assert_eq!(base_unit_for("Cream"), "Tube");
        assert_eq!(base_unit_for("Powder"), "Sachet");
        // Anything unrecognised counts in pieces, which `[D-33]` makes the base unit.
        assert_eq!(base_unit_for("Medical Device"), "Piece");
    }

    #[test]
    fn the_monograph_carried_by_the_most_products_wins() {
        let mono = |s: &str| -> Monograph {
            [Some(s.to_owned()), None, None, None, None, None, None, None, None, None]
        };
        let mut variants = HashMap::new();
        let common = mono("the text on 12 products");
        let rare = mono("the text on 2");
        variants.insert(hash_of(&common), (12_u32, common.clone()));
        variants.insert(hash_of(&rare), (2_u32, rare));
        assert_eq!(winning_monograph(&variants), Some(&common));
    }

    #[test]
    fn a_tie_breaks_on_the_longer_text_so_two_runs_agree() {
        let mono = |s: &str| -> Monograph {
            [Some(s.to_owned()), None, None, None, None, None, None, None, None, None]
        };
        let short = mono("brief");
        let long = mono("a considerably longer description of the same thing");
        let mut variants = HashMap::new();
        variants.insert(hash_of(&short), (5_u32, short));
        variants.insert(hash_of(&long), (5_u32, long.clone()));
        assert_eq!(winning_monograph(&variants), Some(&long));
    }

    #[test]
    fn every_dosage_form_keyword_resolves_to_a_seeded_pack_unit() {
        // `sync_dosage_forms` errors if a form maps to a unit that is not in `PACK_UNITS`, which
        // would fail the whole import at row one. Catch it here instead.
        for form in [
            "Tablet", "Capsule", "Syrup", "IM/IV Injection", "IV Infusion", "Cream", "Ointment",
            "Gel", "Powder", "Sachet", "Ophthalmic Solution", "Nasal Spray", "Inhaler",
            "Oral Suspension", "Powder for Suspension", "Lotion", "Shampoo", "Device",
        ] {
            let unit = base_unit_for(form);
            assert!(
                PACK_UNITS.contains(&unit),
                "form '{form}' maps to '{unit}', which PACK_UNITS does not seed"
            );
        }
    }
}
