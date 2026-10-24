# PLAN: 01 — Catalogue

**Status:** Draft
**Crate:** `crates/catalog`

> Gate 2 artifact. [`SPEC.md`](SPEC.md) says what and why; this says how. If anything here
> contradicts the spec, the spec wins and this is corrected.
>
> **Q1, Q2 and Q3** in `SPEC.md` §5 are each scoped to one endpoint or one task group. Nothing
> in this plan depends on their answers except where it says so.

---

## 1. Crate layout

```
crates/catalog/
  Cargo.toml          # core, db, api — no cache, no module crate
  src/
    lib.rs            # mod repo;  pub mod commands, platform, queries, rest, sellable, types, perms
    types.rs          # API types — nothing generated goes on the wire
    queries.rs        # reads — take &Db
    commands.rs       # pharmacy writes — take &Tx
    platform.rs       # platform writes — take &Tx opened with begin_platform
    sellable.rs       # is_sellable, the one function other contexts call
    rest.rs           # axum handlers, no logic
    perms.rs          # the five permission slugs
    repo.rs           # SQL — private
    entity/           # generated SeaORM entities — private
```

**Dependencies out:** `core` for `AppError`, `ErrorCode` and the id newtypes; `db` for `Tx`
and the tenant context; `api` for `ListPage`, `Pagination`, `AuthUser`, `PlatformToken`,
`require` and the envelope. **Not `cache`** — the catalogue is the cheapest read in the system
and a Redis hop would cost more than the query.

**Dependencies in:** `server` only, for the two routers. `06 Inventory`, `07 Sales` and
`09 Workflows` will call `is_sellable` and `merge` once they exist — through this crate's
public surface, never its repo. No module crate appears as a dependency of this one in either
direction `[D-97]`.

**Two files the house layout does not have.** `platform.rs` follows module 02, which separated
platform writes so that "every one of these asserts `app.is_platform`" is a property of a file
rather than a habit. `sellable.rs` is one function in its own file for the same reason inverted:
it is the only thing other contexts may call, so it should be impossible to add to it by
accident.

---

## 2. Migrations

| # | Migration | Creates | RLS | Grants |
|---|---|---|---|---|
| 1 | `m0005_catalog` | `pg_trgm`; the 12 shared tables; `catalog_issue_reports`; 3 partial uniques; 5 check constraints; 2 GIN trigram indexes; 6 btree indexes | `FORCE` on all 13, in three policy shapes | app: `SELECT` on all · `INSERT` on the 3 submittables, issue reports and `product_packs` (own pending product only, `[AC-E09]`) · **no `UPDATE` on any shared table** · platform: `INSERT`/`UPDATE` everywhere · jobs: `SELECT` |

One migration, not several: the thirteen tables are a single referential web — a product needs
its company, generic, form and packs to exist — and splitting them would only create an
intermediate state nobody runs.

**Article V applies: write it, do not run it.** The command for a human:

```bash
cargo run -p migration -- up
```

### The three policy shapes

Module 02 established two of these. The first is new, and it is the only novel thing in this
migration.

```sql
-- Shape A — submittable record. Approved to everyone, pending to its submitter.
-- On the three tables: companies, generics, products.
ALTER TABLE products ENABLE ROW LEVEL SECURITY;
ALTER TABLE products FORCE ROW LEVEL SECURITY;

CREATE POLICY products_read ON products FOR SELECT USING (
    status = 'approved'
    OR submitted_by_tenant_id = app.current_tenant_id()
    OR current_setting('app.is_platform', true) = 'true'
);
CREATE POLICY products_submit ON products FOR INSERT WITH CHECK (
    status = 'pending'
    AND submitted_by_tenant_id = app.current_tenant_id()
);
CREATE POLICY products_platform_write ON products FOR UPDATE
    USING (current_setting('app.is_platform', true) = 'true')
    WITH CHECK (current_setting('app.is_platform', true) = 'true');
```

Three policies rather than one `FOR ALL`, because the three commands have genuinely different
conditions — and because module 02's single `FOR ALL` policy is precisely what let a grant
widen past its contract (`../02-tenancy/BACKLOG.md` **A3**). The `FOR INSERT WITH CHECK` is
what makes `[AC-E01]` structural: a pharmacy cannot insert an already-approved row, or a row
submitted in someone else's name, even if a handler forgets to set the columns.

```sql
-- Shape B — platform lookup. Readable by all, writable only by platform.
-- On: ingredients, therapeutic_classes, pack_units, dosage_forms, product_packs,
--     generic_monographs, generic_ingredients, generic_therapeutic_classes.
CREATE POLICY pack_units_read ON pack_units FOR SELECT USING (true);
CREATE POLICY pack_units_admin ON pack_units FOR ALL
    USING (current_setting('app.is_platform', true) = 'true')
    WITH CHECK (current_setting('app.is_platform', true) = 'true');
```

Identical to module 02's plan-table shape, deliberately — a reader who knows one knows both.

**`product_packs` carries one exception to shape B**, for the optional pack a submitter may
offer `[AC-E09]`. It is an `INSERT` policy and nothing more — no pharmacy gains `UPDATE` or
`DELETE` on it, and the row is reachable only while the parent product is that pharmacy's own
pending submission:

```sql
CREATE POLICY product_packs_submit ON product_packs FOR INSERT WITH CHECK (
    current_setting('app.is_platform', true) = 'true'
    OR EXISTS (
        SELECT 1 FROM products p
        WHERE p.id = product_packs.product_id
          AND p.status = 'pending'
          AND p.submitted_by_tenant_id = app.current_tenant_id()
    )
);
```

Once the row is approved the `EXISTS` stops matching, so a pack cannot be added to an approved
product by anyone but platform staff — which is `PUT …/packs/{pack_unit_id}`'s job.

```sql
-- Shape C — the ordinary tenant policy, on catalog_issue_reports alone.
CREATE POLICY catalog_issue_reports_tenant ON catalog_issue_reports
    USING (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true')
    WITH CHECK (tenant_id = app.current_tenant_id() OR current_setting('app.is_platform', true) = 'true');
```

**A satellite inherits its generic's visibility, and shape B is not that.** `generic_monographs`,
`generic_ingredients` and `generic_therapeutic_classes` hang off a generic that may itself be
pending. Reading them is harmless — a monograph is medical reference text, not anybody's
business data — so they take shape B and are readable by all. The alternative, an `EXISTS`
subquery against the parent's status in every policy, costs a join on every read to hide
nothing worth hiding. **This is a judgement, so it is written down rather than assumed:** if a
pending generic's name is considered confidential, these three move to an `EXISTS` policy and
the queries do not change.

### Seeding

`pack_units` and `dosage_forms` need rows before any product can exist, and `[D-6]` fixes what
they mean. The migration seeds the units the base-unit rule depends on — `piece`, `strip`,
`box`, `bottle`, `vial` — and nothing else. The 758 therapeutic classes, 354 companies and
2,192 generics are import data, not seed data: they arrive by **Q1**, and a migration that
inlined 30,671 products would be unreviewable.

---

## 3. Types

```rust
// ids — core::ids, newtypes over Uuid v7, as elsewhere
CompanyId, GenericId, ProductId, IngredientId, TherapeuticClassId,
DosageFormId, PackUnitId, IssueReportId

// The status every submittable record carries. One enum, three tables.
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecordStatus { Pending, Approved, Rejected, Merged }

// Which table a review action is about. A path enum, never a free string.
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind { Company, Generic, Product }

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductType { Allopathic, Herbal, NonMedicine }

pub struct ProductSummary {        // what search and every list return
    pub id: ProductId,
    pub brand_name: String,
    pub strength: Option<String>,
    pub company_name: String,
    pub generic_name: Option<String>,
    pub dosage_form: Option<String>,
    pub product_type: ProductType,
    #[serde(with = "decimal_str_opt")]
    pub reference_unit_mrp: Option<Decimal>,
    pub is_discontinued: bool,
    pub status: RecordStatus,
}

pub struct Product { /* ProductSummary, plus packs: Vec<PackDefault> and ids */ }
pub struct Generic { /* id, name, status, ingredients: Vec<Ingredient>, classes: Vec<TherapeuticClass> */ }
pub struct Monograph { /* generic_id + the ten text fields, each Option<String> */ }
pub struct Submission { /* kind, id, display name, status, review_note, merged_into_id, submitted_at */ }
pub struct Lookups { /* dosage_forms, pack_units, therapeutic_classes */ }
pub struct IssueReport { /* id, target (one of three), message, status, admin_note */ }
pub struct Reviewed { /* kind, id, status, merged_into_id, reviewed_at */ }

pub struct NewProduct { /* company_id, generic_id, dosage_form_id, brand_name, strength,
                          product_type, reference_unit_mrp, pack: Option<NewPack>
                          — no status; the pack is optional [AC-E09] */ }
pub struct NewPack { /* pack_unit_id, base_units — no price [D-34] */ }
```

**The queue's suggestions are one `LATERAL` join, not a second round trip** `[AC-E10]`. The
`pg_trgm` GIN index that `[AC-E08]`'s search needs is the same index that serves this, so the
cost is one join per page of the queue — not per pending row, and no N+1:

```sql
-- per pending product; the company and generic variants differ only in table and column
LEFT JOIN LATERAL (
    SELECT c.id, c.brand_name AS name, similarity(c.brand_name, p.brand_name) AS similarity
    FROM products c
    WHERE c.id <> p.id
      AND c.status = 'approved'
      AND c.brand_name % p.brand_name          -- the GIN index answers this
    ORDER BY similarity DESC
    LIMIT 5
) s ON true
```

`%` uses `pg_trgm`'s threshold rather than a hand-picked number, so an empty list is the honest
answer when nothing is close. Suggestions are advisory: no handler reads them back, and a merge
is still `POST /admin/catalog/{kind}/{id}/merge` with an explicit target.

**`RecordStatus` and `ProductType` are enums, not `String`.** Module 02 used strings for
`status` and `mode`, and its own backlog item **C7** now asks for them back as enums — the
`CHECK (status IN (...))` in the database constrains the column while Rust constrains nothing,
and a `from_str_lossy` that defaults on an unknown value turns a schema drift into silent
wrong behaviour. Starting with the enum costs nothing here and is the house rule
(`rules/rust.md` §8, newtypes over primitives).

**Money is `Decimal` with string serialisation** `Art. IV.5` — `reference_unit_mrp` is the only
money in this context, and it is a suggestion rather than a price, which does not exempt it.

---

## 4. Commands

| Function | Writes to | Pre-conditions | Errors |
|---|---|---|---|
| `submit_company` | `companies` | `catalog.submit`; tenant context set | `duplicate_company_name` |
| `submit_generic` | `generics`, `generic_ingredients`, `generic_therapeutic_classes` | ditto; every ingredient and class id exists | `duplicate_generic_name`, `not_found` on an unknown ingredient or class |
| `submit_product` | `products`, and `product_packs` when a pack was given `[AC-E09]` | ditto; company and generic are approved or the caller's own pending; `[AC-U04]` holds; the `pack_unit_id` exists | `duplicate_product`, `validation_failed`, `not_found` |
| `report_issue` | `catalog_issue_reports` | `catalog.issue.report`; exactly one target, and it is visible to the caller | `validation_failed`, `not_found` |
| `approve` | one of the three | platform token; row is `pending` | `already_reviewed`, `not_found` |
| `reject` | one of the three | platform token; row is `pending`; note is non-empty after trim | `already_reviewed`, `validation_failed` |
| `merge` | one of the three | platform token; row is `pending`; target exists, is not the row, and is not itself `merged` | `already_reviewed`, `invalid_merge_target` |
| `upsert_monograph` | `generic_monographs` | platform token; generic exists | `not_found` |
| `resolve_issue` · `dismiss_issue` | `catalog_issue_reports` | platform token; report is `open` | `already_closed` |

**One transaction each, and no cross-module write.** `merge` is the only one with an obvious
temptation: `[D-2]` says the pharmacy's stock and sales move onto the surviving product, and
doing that here would mean `catalog` writing to `06`'s and `07`'s tables. It does not. `merge`
sets two columns; `09 Workflows` calls `merge` and the two stock functions in one `Tx` when
those modules exist. **Until then a merge is incomplete by design**, and that is the honest
state: the alternative is a module crate reaching across a boundary, which the constitution
forbids and which nothing would catch until it was everywhere.

`submit_product` checks that the company and generic are visible to the caller before
inserting. It would be tempting to let the FK do it — but a FK is checked with row security
off (the mechanism behind `../02-tenancy/BACKLOG.md` **A2**), so a plain FK would happily
accept another pharmacy's pending company id. The check is explicit, and
`a_submission_cannot_reference_another_pharmacys_pending_company` is the test that holds it.

---

## 5. Queries

| Function | Reads | Shape | sqlx or SeaORM? |
|---|---|---|---|
| `search` | `products` ⋈ `companies` ⋈ `generics` ⋈ `dosage_forms` | `ListPage<ProductSummary>`, keyset | **`sqlx::query_as!`** — trigram similarity and an `ORDER BY similarity DESC` are not expressible in SeaORM's builder |
| `products` | same four | `ListPage<ProductSummary>`, keyset on `id` | SeaORM |
| `product` | same four + `product_packs` ⋈ `pack_units` | `Product` | SeaORM, two queries |
| `generic` | `generics` + both junctions | `Generic` | SeaORM, `find_with_related` |
| `monograph` | `generic_monographs` | `Option<Monograph>` | SeaORM, `find_by_id` |
| `companies` | `companies` | `ListPage<Company>`, keyset | SeaORM |
| `lookups` | 3 lookup tables | `Lookups` | SeaORM, three queries — **not** one per row `[C2]` |
| `submissions` | the 3 submittables | `ListPage<Submission>` | **`sqlx::query_as!`** — a `UNION ALL` of three tables with a literal `kind` column |
| `queue` | the 3 submittables | `ListPage<QueueItem>` | **`sqlx::query_as!`**, same `UNION ALL`, oldest first, each row with its suggestions from a `LATERAL` join `[AC-E10]` |
| `is_sellable` | `products` | `bool` | SeaORM, one `find_by_id` under the read policy |

`cargo sqlx prepare` must be re-run when any of the three macro queries changes, and
`cargo sqlx prepare --check` is a `VERIFY:` line in Phase 4 so that a stale `.sqlx` cache
fails the gate rather than the next person's build.

**`is_sellable` is a policy read, not a status check.** It asks the database for the row under
the caller's tenant context and returns `true` when it comes back and is `approved` or the
caller's own `pending` — so the answer is the RLS policy's answer `[D-1]`, not a condition
reimplemented in Rust that could drift from it.

**Keyset pagination throughout**, `(id)` ascending, matching module 02. Search is the one
exception: it orders by similarity, so its cursor is `(similarity, id)` and the query carries
both.

---

## 6. Test plan

Every `[AC-*]` maps to at least one test. Behaviour gets a test, not a `grep`.

| Acceptance criterion | Test name | Database? |
|---|---|---|
| `[AC-U01]` | `no_catalog_table_carries_a_tenant_id` | no — reads the migration SQL |
| `[AC-U02]` `[AC-S01]` | `a_pending_row_is_invisible_to_another_pharmacy` | yes |
| `[AC-U02]` | `isolation_fails_when_the_product_read_policy_is_dropped` | yes |
| `[AC-U03]` | `merged_into_id_requires_merged_status` | yes |
| `[AC-U04]` | `a_medicine_without_a_generic_is_rejected_by_the_database` | yes |
| `[AC-U05]` | `an_issue_report_needs_exactly_one_target` | yes |
| `[AC-U06]` | `no_list_response_carries_monograph_text` | yes |
| `[AC-U07]` `[AC-W01]` | `pos_app_cannot_write_a_lookup_table` | yes |
| `[AC-U08]` | `therapeutic_class_hangs_off_the_generic` | no — migration SQL |
| `[AC-E01]` | `a_submission_is_pending_and_owned_by_its_submitter` | yes |
| `[AC-E01]` | `a_pharmacy_cannot_insert_an_approved_row` | yes |
| `[AC-E02]` | `an_exact_duplicate_product_is_refused` | yes |
| `[AC-E02]` | `a_rejected_row_does_not_block_a_later_submission` | yes |
| `[AC-E03]` | `approval_records_who_and_when_and_publishes_the_row` | yes |
| `[AC-E04]` | `a_merge_sets_the_target_and_moves_no_stock` | yes |
| `[AC-E05]` | `rejecting_requires_a_note` | yes |
| `[AC-E06]` | `an_issue_report_is_filed_open` | yes |
| `[AC-E07]` | `a_generic_and_its_monograph_are_written_in_one_transaction` | yes |
| `[AC-E08]` | `search_matches_a_misspelled_brand_name` | yes |
| `[AC-E08]` | `search_returns_my_pending_row_and_not_yours` | yes |
| `[AC-S02]` | `a_discontinued_product_is_readable_but_not_searched` | yes |
| `[AC-S03]` | `a_merged_row_reads_back_with_its_target` | yes |
| `[AC-W02]` | `a_medicine_submission_names_every_missing_field` | yes |
| `[AC-W03]` | `merging_into_a_merged_row_is_refused` | yes |
| `[AC-W04]` | `an_already_reviewed_row_cannot_be_reviewed_again` | yes |
| `[AC-W05]` | `an_issue_report_with_two_targets_is_refused` | yes |
| `[AC-W06]` | `another_pharmacys_pending_row_is_404_not_403` | yes |
| — | `a_submission_cannot_reference_another_pharmacys_pending_company` | yes |
| — | `is_sellable_follows_the_read_policy` | yes |

**RLS tests are mandatory**, and `isolation_fails_when_the_product_read_policy_is_dropped`
is the one that proves the others test something: it drops the policy as the owner role,
asserts the cross-tenant read now succeeds, and restores it.

**A test that cannot reach the database fails.** Module 02's suite returns early when
`APP_DATABASE_URL` is unreachable, and its whole isolation suite therefore reports `ok`
against no database at all (`../02-tenancy/BACKLOG.md` **A4**). This module's harness
`.expect()`s the connection instead, so a missing database is a red suite and never a green
one. `expect_used` is `warn` and not `deny` in the workspace lints, so this compiles; a test
harness is exactly the place it is right.

---

## 7. Risks

| Risk | Why it is plausible here | What catches it |
|---|---|---|
| The read policy is written as `FOR ALL` and a pharmacy edits central data | It is the shape module 02 used on all six tables, so it is the shape a reader copies | `pos_app_cannot_write_a_lookup_table`, and no `UPDATE` grant on any shared table — the grant is the real guard, the policy is the second one |
| A pharmacy's pending row leaks into another's search | The condition is in a policy, not a `WHERE`, so a wrong policy is invisible in the query | `search_returns_my_pending_row_and_not_yours`, plus the drop-the-policy test |
| `submit_product` accepts another pharmacy's pending company by FK | Postgres checks referential integrity with row security off — the module 02 defect, one module later | `a_submission_cannot_reference_another_pharmacys_pending_company` |
| Monograph text reaches a list response and then a phone | Ten `Option<String>` columns on a 1:1 table are one careless `find_also_related` away from a list | `no_list_response_carries_monograph_text`, and `Monograph` existing only as its own type |
| Trigram search degrades as the catalogue grows | 30,671 products with two GIN indexes is small, but `ORDER BY similarity` cannot use the index for ordering | `search` carries `LIMIT` from the keyset page; a `% `threshold filter precedes the sort so the index does the selection |
| The partial unique index omits `NULLS NOT DISTINCT` | It is a Postgres 15+ clause that reads like a detail; without it every non-medicine with a NULL `dosage_form_id` is distinct from every other | `an_exact_duplicate_product_is_refused` is written with a non-medicine, where the bug shows |
| A merge chain forms — A merged into B, B later merged into C | Nothing stops a reviewer merging a row that is already a target | `[AC-W03]` refuses a merged target at write time, so chains cannot start. Resolving them at read time was the alternative and is strictly worse: every read pays for it |
| The import lands as a migration | A 30,671-row `INSERT` is the path of least resistance, and `m0005` is right there | **Q1** is unanswered, so no import task exists to be done wrongly. This is the risk the open question exists to hold |
