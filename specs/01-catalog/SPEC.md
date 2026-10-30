# SPEC: 01 — Catalogue

**Status:** Draft
**Crate:** `crates/catalog`
**Bounded context:** Context 1 — Catalogue
**Owner:** backend-architect role, drafted 2026-10-23

---

## 1. Purpose

The one medicine catalogue every pharmacy shares: manufacturers, generics and their medical
text, ingredients, therapeutic classes, dosage forms, pack units, and the products that
combine them. It owns the **submission and review lifecycle** that lets a pharmacy add what is
missing and lets platform staff approve, reject or merge it, and the **issue reports** a
pharmacy files against central data it cannot edit itself.

This is the only context in the system with **no `tenant_id` on its main tables** — the
catalogue is shared, so there is nothing to scope. Visibility of a pending submission is still
a row-level security policy rather than a `WHERE` clause, because "approved, plus my own
pending rows" is exactly the kind of filter that gets forgotten once.

**Not this module's job:**

| The nearest wrong assumption | Who owns it |
|---|---|
| What a pharmacy charges, its default price per product, MRP change requests | **04 Pricing**. `products.reference_unit_mrp` here is an imported suggestion and nothing more |
| A pharmacy's own pack sizes | **04 Pricing**. `product_packs` here is a central default where the source data knew the box size |
| Batches, stock on hand, expiry, reorder levels | **06 Inventory** |
| Moving a pharmacy's stock and sales onto the surviving product after a merge | **06 Inventory** and **07 Sales**, orchestrated by **09 Workflows** `[D-2]` `[D-97]`. This module sets `merged_into_id` and stops |
| Who is allowed to submit | **03 Identity** owns roles and permission grants. This module names the slugs and nothing else |
| What the phone caches and how it refreshes | **10 Sync** |
| The one-time import of 30,671 medex products | Unwritten. See **Q1** |

---

## 2. Decisions this module implements

Cite, never restate. The reasoning lives in Notion.

| Ref | Decision | Consequence here |
|---|---|---|
| `[D-1]` | Sell a pending catalogue submission? Yes, visible only to the submitting tenant | A pending row is readable and sellable by its submitter and invisible to everyone else — the RLS policy in `AC-U02`, and the reason a submission needs no separate table |
| `[D-2]` | Rejected as duplicate? Merge stock and sales onto the existing product | `merge` sets `status = 'merged'` and `merged_into_id`, and returns. The stock and sales move is `workflows`, not here |
| `[D-6]` | Loose tablets — stock counted in pieces | `dosage_forms.base_unit_id` says what one piece means; `product_packs.base_units` counts pieces, never a nested pack |
| `[D-8]` | Batch and expiry for non-medicines: optional | `product_type = 'non_medicine'` is the one case where `generic_id` and `dosage_form_id` may be NULL |
| `[D-21]` | Barcodes skipped in v1 | No barcode column, no lookup-by-barcode endpoint. Search is text |
| `[D-22]` | Medical text kept, online-only | `generic_monographs` is a 1:1 side table, read by one endpoint, and never included in a list response |
| `[D-27]` | Drug info offline: online-only, cached once opened | The monograph endpoint is a single-row read by design, so a client can cache per generic |
| `[D-34]` | No separate pack price — always unit price × pieces | `product_packs` carries `base_units` and no price column |
| `[D-62]` | Medex data licence: risk accepted | `medex_id` is a nullable unique on `companies` and `products`, the only trace of the source. Nothing structural depends on the monograph text, so dropping it would cost a feature and not the model |
| `[D-80]` | Platform staff roles are a fixed list in code | Review and lookup editing are platform-only permissions, hidden from a pharmacy's role editor rather than shown and refused |
| `[D-85]` | The API returns stable codes plus English | Every error below is a code; the Bengali wording is the client's |
| `[D-97]` | Cross-module flows live in a `workflows` crate | `catalog` depends on no module crate, and no module crate depends on `catalog` |

---

## 3. Behaviour

EARS acceptance criteria, each one a test name in [`plan.md`](plan.md) §6.

- **Ubiquitous (always true)**
  - `[AC-U01]` The catalogue tables SHALL carry no `tenant_id`. `catalog_issue_reports` is the
    single exception and is tenant-owned in full.
  - `[AC-U02]` A pharmacy SHALL read approved rows plus the rows it submitted itself, enforced
    by a row-level security policy and not by a `WHERE` clause.
  - `[AC-U03]` `merged_into_id` SHALL be set if and only if `status = 'merged'`.
  - `[AC-U04]` A product whose `product_type` is not `non_medicine` SHALL have both
    `generic_id` and `dosage_form_id`.
  - `[AC-U05]` An issue report SHALL point at exactly one of a company, a generic or a product.
  - `[AC-U06]` Monograph text SHALL NOT appear in any list response — only in the single
    generic's monograph read `[D-22]`.
  - `[AC-U07]` `ingredients`, `therapeutic_classes`, `dosage_forms` and `pack_units` SHALL be
    writable only by a caller holding the platform token.
  - `[AC-U08]` A therapeutic class SHALL hang off the generic and never off the product, so the
    fact is stored once.

- **Event-driven (when)**
  - `[AC-E01]` WHEN a pharmacy submits a company, generic or product, the system SHALL store it
    with `status = 'pending'`, `submitted_by_tenant_id` and `submitted_by_user_id`, and make it
    visible to that pharmacy alone `[D-1]`.
  - `[AC-E02]` WHEN a submitted product matches an existing pending or approved product on
    `(company_id, brand_name, strength, dosage_form_id)`, the system SHALL refuse it with
    `409 duplicate_product` rather than queue a second copy.
  - `[AC-E03]` WHEN platform staff approve a row, the system SHALL set `status = 'approved'`,
    record `reviewed_by_user_id` and `reviewed_at`, and the row SHALL become visible to every
    pharmacy.
  - `[AC-E04]` WHEN platform staff merge a row into a surviving row, the system SHALL set
    `status = 'merged'` and `merged_into_id` in one transaction, and SHALL NOT itself move any
    stock or sales `[D-2]`.
  - `[AC-E05]` WHEN platform staff reject a row, the system SHALL require `review_note` and set
    `status = 'rejected'`.
  - `[AC-E06]` WHEN a pharmacy files an issue report, the system SHALL store it `open` against
    exactly one catalogue row.
  - `[AC-E07]` WHEN platform staff write a generic's medical text, the system SHALL write the
    generic and its monograph in one transaction.
  - `[AC-E08]` WHEN a search runs, the system SHALL match against brand name and generic name
    using the `pg_trgm` indexes, and SHALL return pending rows belonging to the caller
    alongside approved ones.
  - `[AC-E09]` WHEN a submitted product carries a pack size, the system SHALL store it as that
    product's central pack default in the same transaction as the submission, and WHEN it
    carries none, the system SHALL accept the submission with no pack row at all. A pack
    submitted this way SHALL carry `base_units` and no price `[D-34]`, and the insert SHALL be
    permitted only while the product is `pending` and was submitted by the caller.
  - `[AC-E10]` WHEN the review queue is read, the system SHALL return with each pending row up
    to five existing rows of the same kind whose name is most similar to it, ranked by trigram
    similarity over `brand_name` descending, and SHALL return an empty list rather than a low
    match when none passes the threshold. The suggestion SHALL be advisory only: it SHALL not
    block, pre-select or perform a merge.

- **State-driven (while)**
  - `[AC-S01]` WHILE a submission is pending, only the submitting pharmacy SHALL be able to read
    or sell it `[D-1]`.
  - `[AC-S02]` WHILE a product is `is_discontinued`, it SHALL remain readable and sellable from
    existing stock, and SHALL be excluded from search results unless explicitly asked for.
  - `[AC-S03]` WHILE a row is `merged`, a read of it SHALL return its `merged_into_id` so a
    client can follow the move, and the row SHALL be excluded from search.

- **Unwanted (if / then)**
  - `[AC-W01]` IF a caller without the platform token writes a lookup table, THEN the system
    SHALL respond `403 permission_denied` `[D-80]`.
  - `[AC-W02]` IF a submitted product is not `non_medicine` and omits company, generic, strength
    or dosage form, THEN the system SHALL respond `400 validation_failed` naming each missing
    field.
  - `[AC-W03]` IF a merge names a target that is itself `merged`, or names the row being merged,
    THEN the system SHALL respond `409 invalid_merge_target` — a merge chain is not resolved at
    read time, so it is refused at write time.
  - `[AC-W04]` IF a review action is taken on a row that is not `pending`, THEN the system SHALL
    respond `409 already_reviewed`.
  - `[AC-W05]` IF an issue report names zero or more than one target, THEN the system SHALL
    respond `400 validation_failed`.
  - `[AC-W06]` IF a pharmacy requests a pending row belonging to another pharmacy, THEN the
    system SHALL respond `404 not_found` and never `403` — the row's existence is not the
    caller's business.

**Offline behaviour.** Nothing in this module is created offline: offline scope is sales only
`[D-24]`, so every write here needs a connection and no endpoint takes a client-generated id.
The phone holds a **lean read copy** — brand name, strength, company, generic name, dosage
form, pack defaults — and **the monograph never ships to a device** `[D-22]` `[D-27]`: at
127 MB it cannot, which is why the monograph is its own endpoint and its own table. How the
lean copy reaches the phone and how it refreshes is **10 Sync**'s contract, not this one.

---

## 4. Tenancy and permissions

| Question | Answer |
|---|---|
| Tenant-owned? | **No** for the twelve catalogue tables — platform-owned, shared. **Yes** for `catalog_issue_reports`, which carries `tenant_id` and a tenant policy |
| Branch-scoped? | No. A catalogue is a business-wide fact, so `require_at` is never used here |
| Permissions required | `catalog.view` to read · `catalog.submit` to submit · `catalog.issue.report` to file a report · platform-only: `catalog.review`, `catalog.lookup.manage` |
| Visible to a cashier? | Everything readable here is readable by a cashier. Submission is not: Product Spec §3 limits it to owner and manager, which **03 Identity** expresses as `catalog.submit` on those two seeded roles |

**What is specific to this module** — Article IV otherwise applies unchanged:

1. **A third RLS shape.** Module 02 established two: a tenant policy keyed on
   `app.current_tenant_id()`, and a platform table readable by all and writable only with
   `app.is_platform`. This module needs a third — **readable when `status = 'approved'`, or
   when `submitted_by_tenant_id = app.current_tenant_id()`; writable only with
   `app.is_platform`, except the insert of a pending row by its own submitter.** It is still
   `FORCE ROW LEVEL SECURITY` on every table, and it is still one policy per command.
2. **`submitted_by_tenant_id` is not a `tenant_id`.** It records who asked, not who owns, so
   `Art. IV.3` and `Art. IV.4` — composite FKs and tenant-scoped uniqueness — do **not** apply
   to it. Uniqueness here is global on purpose: two pharmacies must not be able to create two
   "Napa 500 mg" rows by the same company. This is the one place in the system where a plain
   `UNIQUE` is correct, and it is correct because the data is shared.
3. **Uniqueness applies to pending and approved rows only**, so a rejected or merged row never
   blocks a later legitimate submission. In Postgres that is a partial unique index, and for
   products it needs `NULLS NOT DISTINCT` because `dosage_form_id` is nullable for
   non-medicines.

---

## 5. Open questions

| # | Question | Blocks | Raised |
|---|---|---|---|
| **Q1** | **How does the one-time import of 30,671 products run?** Where the medex CSVs live, who runs it, whether it is a migration, a seed binary or an `xtask`, whether it is re-runnable, and whether the medex-id de-duplication and the monograph collapse (2,192 generics, 74 of which vary) belong in this crate or in a throwaway tool. `cargo xtask n catalogue` resolves to **Data Import Plan**, which [`../NOTION.map`](../NOTION.map) lists as *unwritten* with no page id. Product Specification §2 gives the data shape and the nine quality fixes, but not the mechanics. | The import tasks only — omitted from [`tasks.md`](tasks.md) by name. **Not Gate 2 for the catalogue surface:** no table, endpoint or policy below depends on the answer, and `medex_id` is declared by the data model either way | 2026-10-23 |

**Answered — Q1, 2026-10-25.** The source files arrived as `Extra Docs/companies.csv` and
`Extra Docs/medicines.csv`, so the mechanics are now settled. *Data Import Plan* in Notion is
still unwritten; this section is the record until it is, and it is the **largest page↔repo gap
module 01 has**.

**It is an idempotent seeder, not a migration.** `crates/catalog/src/bin/seed.rs`, behind the
crate's off-by-default `seed` feature so no other crate's dependency graph changes:

```bash
cargo run --release -p catalog --features seed --bin catalog-seed -- --dir "Extra Docs"
```

A migration was rejected outright: Article V.4 makes an applied migration immutable, and a
136 MB load that may need re-running after a source correction is the opposite of immutable. An
`xtask` was rejected too — that is the SDD runner, not a data tool. It lives inside module 01's
own crate because the import is this module's, and because a new crate under `crates/` would
fail Gate 0 (`tools/sdd-lint` maps every crate to a module via `MODULES.map`).

**Idempotency is a property of the keys, not of a flag.** Each step reads what is already there,
inserts only what is missing, and matches on a key the schema already enforces: `name` for the
four lookups and for `generics` (`idx_generics_name`), `medex_id` or `name` for `companies`, and
`(company_id, brand_name, strength, dosage_form_id)` for `products` — which is `idx_products_unique`,
and is Product Spec §3's own rule that *"a product is identified by company + brand + strength +
dosage form, never by brand name alone"*. The whole import is one transaction. Verified, not
claimed: [`tasks.md`](tasks.md) Phase 9 runs the binary twice over a fixture and asserts the
second run inserts nothing. `generic_monographs` is the one exception — it is re-written on every
run, deliberately, so a corrected source file reaches the database.

**It requires the owner role, and that is a finding rather than a convenience.**
`m0005_catalog`'s `companies_insert`, `generics_insert` and `products_insert` policies all read
`status = 'pending' AND submitted_by_tenant_id = app.current_tenant_id()`. There is **no platform
INSERT path for the three submittable tables** — the review flow inserts as a tenant and then
*updates* to `approved`. An imported row is `approved` with no submitting tenant, which no policy
permits, so the seeder connects as `pos_owner` exactly as the migrator does. Article IV.1 is
untouched: the *application* still connects as a non-owner. The same gap means **a super admin
cannot add a product directly through the API at all** — [`BACKLOG.md`](BACKLOG.md) A6.

**The nine quality fixes of Product Spec §2 are implemented there**, and the measured results
agree with the page: 5,237 blank dosage forms recovered from the last word of `source_url`
(tablet 2,557, capsule 706, injection 540 — the page's own counts), the one `source = 'test'` row
and the `Test Pharma Ltd` company dropped, 24,626 prices parsed with 0 failures, every field
trimmed, 219 duplicate rows collapsed keeping the most complete copy, and 2,177 monographs stored
once per generic. **30,450 products, 2,192 generics, 353 companies, 664 therapeutic classes and
148 dosage forms**, in about seven seconds.

**Two places the file contradicts the page.** Both are measured, and both are in `INDEX.md`'s
drift register:

1. **`products.medex_id` is left NULL.** §2 says to de-duplicate on *"the medex numeric id inside
   the URL"* and sizes it at *40 pairs*. In the real file **3,103 medex ids cover more than one
   product** — differing by strength (990), dosage form (1,027) or brand name (219), because one
   medex brand page lists several products. Only 867 are true duplicates. `idx_products_medex_id`
   is UNIQUE, so writing that id would reject roughly 2,250 legitimate products. Either the index
   is wrong about what a medex id identifies, or the column is. Until that is decided the column
   stays empty and identity comes from the natural key, which costs the import its traceability
   back to a source row.
2. **`product_packs` is not seeded.** §4 says central defaults exist *"where the source data knows
   the box size (about 80% of rows)"*. Measured, `(N's pack: …)` matches **15,162 of 30,670 rows —
   49%** — across 40-plus formats, while *"Tenants define pack sizes"* is `[Decided]`. A guess
   written into a table every pharmacy reads is worse than an empty one. `BACKLOG.md` A7.

**`ingredients` and `generic_ingredients` are left empty.** The CSV has no ingredient column at
all, and splitting `generic_name` on `+` produces 2,246 "ingredients" including *"23 herb formula
to increase breast milk"* and *"20% Fat Emulsion"*. The two tables need a source this scrape does
not contain. `BACKLOG.md` A8.

**The monograph collapse rule is the page's**: the variant carried by the most products wins, and
a tie breaks on the longest text so two runs cannot disagree. The page says 74 generics vary; the
file has **75**.

**The files are gitignored.** `medicines.csv` is 136 MB, above GitHub's 100 MB per-file limit, so
committing it would make the repository unpushable. The seeder takes `--dir`, so the files are
supplied out of band for a production run.

**A spec with open questions blocking Gate 2 does not advance.** Q1–Q3 are each scoped to one
endpoint or one task group, by the rule in [`../rules/lifecycle.md`](../rules/lifecycle.md)
that a question blocks the gate it would change. The catalogue's tables, policies and read
surface are settled by Data Model Context 1 and Product Specification §3, so Gate 2 proceeds
for those.

**Answered — Q2, 2026-10-24.** Near-duplicate suggestions **are** in v1, computed as a
trigram similarity over `brand_name` against the `pg_trgm` GIN index the search surface
already requires `[AC-E08]`. Nothing cleverer: no new table, no new index, no learned ranking.
Product Specification §3's `[Proposed]` is thereby `[Decided]` — see `[AC-E10]`.

**Answered — Q3, 2026-10-24.** A pack size is **optional** on submission, not required. The
required set stays company, generic, strength and dosage form `[AC-W02]`. When a submitter
does give a pack it is stored as the central default, so `product_packs` is no longer
import-only — see `[AC-E09]`. `§4`'s rule is untouched: a pharmacy's own pack sizes and prices
remain **04 Pricing**'s, and `product_packs` still carries no price `[D-34]`.

**This widens one RLS policy.** `product_packs` was a shape-B platform lookup, writable only
with `app.is_platform`. A submitter now inserts one row into it, for a product it submitted and
that is still `pending` — a narrow `INSERT` policy, specified in
[`plan.md`](plan.md) §3 and asserted in [`tasks.md`](tasks.md) Phase 4. Nothing else about
shape B changes, and no pharmacy gains `UPDATE` on any shared table.
