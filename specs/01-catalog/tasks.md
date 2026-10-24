# TASKS: 01 — Catalogue

> Governed by [`specs/CONSTITUTION.md`](../CONSTITUTION.md) **Article II** — every leaf task
> carries a `VERIFY:` line. A task is ticked **only** when its command exits 0. Run:
>
> ```bash
> cargo xtask verify --module 01
> ```
>
> The agent that implements a task does not verify it. `cargo build` and `cargo clippy`
> passing is necessary, never sufficient.
>
> **Article V applies throughout: write the migration, do not run it.** Most assertions here
> need a database — a human applies `m0005_catalog` first, against the Postgres from
> `compose.yaml`.
>
> **This module adds no row to another module's contract.** Everything it needs —
> `Tx::begin`, `Tx::begin_platform`, `AuthUser`, `PlatformToken`, `api::require`, `ListPage`,
> `Pagination` — shipped with modules 00 and 03.

---

- [x] **Phase 1: `m0005_catalog` — the tables, written not run**
  - [x] `CREATE EXTENSION pg_trgm` is the first statement, before any index needs it
        VERIFY: test -f crates/migration/src/m0005_catalog.rs && grep -q "pg_trgm" crates/migration/src/m0005_catalog.rs
  - [x] No catalogue table carries a `tenant_id` except `catalog_issue_reports`
        VERIFY: cargo test -p migration no_catalog_table_carries_a_tenant_id 2>&1 | grep -qE "^test .*no_catalog_table_carries_a_tenant_id \.\.\. ok$"
  - [x] All thirteen tables force RLS, and each has a policy per command rather than one `FOR ALL`
        VERIFY: cargo test -p migration every_catalog_table_forces_rls 2>&1 | grep -qE "^test .*every_catalog_table_forces_rls \.\.\. ok$"
  - [x] A therapeutic class attaches to a generic and there is no product-level class table
        VERIFY: cargo test -p migration therapeutic_class_hangs_off_the_generic 2>&1 | grep -qE "^test .*therapeutic_class_hangs_off_the_generic \.\.\. ok$"
  - [x] Uniqueness covers pending and approved rows only, so a rejected row blocks nothing
        VERIFY: cargo test -p catalog a_rejected_row_does_not_block_a_later_submission 2>&1 | grep -qE "^test .*a_rejected_row_does_not_block_a_later_submission \.\.\. ok$"
  - [x] The product unique index uses `NULLS NOT DISTINCT`, so two non-medicines cannot collide
        VERIFY: cargo test -p catalog an_exact_duplicate_product_is_refused 2>&1 | grep -qE "^test .*an_exact_duplicate_product_is_refused \.\.\. ok$"
  - [x] `merged_into_id` is set if and only if the status is `merged`, by check constraint
        VERIFY: cargo test -p catalog merged_into_id_requires_merged_status 2>&1 | grep -qE "^test .*merged_into_id_requires_merged_status \.\.\. ok$"
  - [x] A row cannot merge into itself, by check constraint
        VERIFY: cargo test -p catalog a_row_cannot_merge_into_itself 2>&1 | grep -qE "^test .*a_row_cannot_merge_into_itself \.\.\. ok$"
  - [x] Anything that is not `non_medicine` needs both a generic and a dosage form
        VERIFY: cargo test -p catalog a_medicine_without_a_generic_is_rejected_by_the_database 2>&1 | grep -qE "^test .*a_medicine_without_a_generic_is_rejected_by_the_database \.\.\. ok$"
  - [x] An issue report points at exactly one of company, generic or product
        VERIFY: cargo test -p catalog an_issue_report_needs_exactly_one_target 2>&1 | grep -qE "^test .*an_issue_report_needs_exactly_one_target \.\.\. ok$"
  - [x] A rejection carries a note, at the database as well as the command
        VERIFY: cargo test -p catalog a_rejection_without_a_note_is_rejected_by_the_database 2>&1 | grep -qE "^test .*a_rejection_without_a_note_is_rejected_by_the_database \.\.\. ok$"
  - [x] `pos_app` holds no `UPDATE` on any shared catalogue table, and no `DELETE` anywhere
        VERIFY: cargo test -p catalog pos_app_cannot_write_a_lookup_table 2>&1 | grep -qE "^test .*pos_app_cannot_write_a_lookup_table \.\.\. ok$"
  - [x] The seed creates the five pack units the base-unit rule needs, and no import data
        VERIFY: cargo test -p catalog the_seed_creates_the_base_pack_units 2>&1 | grep -qE "^test .*the_seed_creates_the_base_pack_units \.\.\. ok$"
  - [x] Both trigram indexes exist — `products.brand_name` and `generics.name`
        VERIFY: cargo test -p migration catalog_has_both_trigram_indexes 2>&1 | grep -qE "^test .*catalog_has_both_trigram_indexes \.\.\. ok$"

- [x] **Phase 2: the crate, its entities and its types**
  - [x] Create `crates/catalog` mapped in `MODULES.map`, with `repo.rs` private
        VERIFY: ! grep -q "pub mod repo" crates/catalog/src/lib.rs
  - [x] Entities generated, never hand-written — the generator header is intact
        VERIFY: cargo test -p catalog entities_are_generated_not_handwritten 2>&1 | grep -qE "^test .*entities_are_generated_not_handwritten \.\.\. ok$"
  - [x] `entity/` is private to the crate
        VERIFY: ! grep -q "pub mod entity" crates/catalog/src/lib.rs
  - [x] `types.rs` — API types only, with `RecordStatus` and `ProductType` as enums and no `String` status
        VERIFY: cargo test -p catalog status_is_an_enum_and_an_unknown_value_is_an_error 2>&1 | grep -qE "^test .*status_is_an_enum_and_an_unknown_value_is_an_error \.\.\. ok$"
  - [x] `reference_unit_mrp` is `Decimal` and serialises as a string, never a number
        VERIFY: cargo test -p catalog reference_mrp_is_a_string_on_the_wire 2>&1 | grep -qE "^test .*reference_mrp_is_a_string_on_the_wire \.\.\. ok$"
  - [x] `catalog` depends on no module crate and holds no `anyhow`
        VERIFY: ! grep -qE "^(tenancy|identity|pricing|inventory|sales|customers|workflows|sync|reporting|jobs|anyhow)" crates/catalog/Cargo.toml

- [x] **Phase 3: what a pharmacy can read**
  - [x] `search` matches a misspelled brand name through the trigram index
        VERIFY: cargo test -p catalog search_matches_a_misspelled_brand_name 2>&1 | grep -qE "^test .*search_matches_a_misspelled_brand_name \.\.\. ok$"
  - [x] Search returns the caller's own pending rows beside approved ones, and nobody else's
        VERIFY: cargo test -p catalog search_returns_my_pending_row_and_not_yours 2>&1 | grep -qE "^test .*search_returns_my_pending_row_and_not_yours \.\.\. ok$"
  - [x] A discontinued product stays readable and sellable but is left out of search
        VERIFY: cargo test -p catalog a_discontinued_product_is_readable_but_not_searched 2>&1 | grep -qE "^test .*a_discontinued_product_is_readable_but_not_searched \.\.\. ok$"
  - [x] A merged row reads back carrying its target, so a client can follow the move
        VERIFY: cargo test -p catalog a_merged_row_reads_back_with_its_target 2>&1 | grep -qE "^test .*a_merged_row_reads_back_with_its_target \.\.\. ok$"
  - [x] No list response anywhere carries monograph text
        VERIFY: cargo test -p catalog no_list_response_carries_monograph_text 2>&1 | grep -qE "^test .*no_list_response_carries_monograph_text \.\.\. ok$"
  - [x] The monograph read returns `204` for a generic with no text, not an empty object
        VERIFY: cargo test -p catalog a_generic_without_a_monograph_is_204 2>&1 | grep -qE "^test .*a_generic_without_a_monograph_is_204 \.\.\. ok$"
  - [x] Products, companies and submissions are cursor-paginated, with no total count
        VERIFY: cargo test -p catalog catalog_lists_are_cursor_paginated 2>&1 | grep -qE "^test .*catalog_lists_are_cursor_paginated \.\.\. ok$"
  - [x] `lookups` is three queries, not one per row
        VERIFY: cargo test -p catalog lookups_is_three_queries 2>&1 | grep -qE "^test .*lookups_is_three_queries \.\.\. ok$"
  - [x] `is_sellable` answers from the read policy and not from a condition in Rust
        VERIFY: cargo test -p catalog is_sellable_follows_the_read_policy 2>&1 | grep -qE "^test .*is_sellable_follows_the_read_policy \.\.\. ok$"
  - [x] The prepared query cache is current, so a stale `.sqlx` fails here and not in a build
        VERIFY: cargo sqlx prepare --check --workspace

- [x] **Phase 4: what a pharmacy can submit**
  - [x] A submission is stored `pending`, owned by its submitter, and visible to them alone
        VERIFY: cargo test -p catalog a_submission_is_pending_and_owned_by_its_submitter 2>&1 | grep -qE "^test .*a_submission_is_pending_and_owned_by_its_submitter \.\.\. ok$"
  - [x] A pharmacy cannot insert an already-approved row, refused by the insert policy
        VERIFY: cargo test -p catalog a_pharmacy_cannot_insert_an_approved_row 2>&1 | grep -qE "^test .*a_pharmacy_cannot_insert_an_approved_row \.\.\. ok$"
  - [x] A pharmacy cannot submit a row in another pharmacy's name
        VERIFY: cargo test -p catalog a_submission_cannot_be_filed_in_another_tenants_name 2>&1 | grep -qE "^test .*a_submission_cannot_be_filed_in_another_tenants_name \.\.\. ok$"
  - [x] A submission cannot reference another pharmacy's pending company or generic
        VERIFY: cargo test -p catalog a_submission_cannot_reference_another_pharmacys_pending_company 2>&1 | grep -qE "^test .*a_submission_cannot_reference_another_pharmacys_pending_company \.\.\. ok$"
  - [x] An exact duplicate is `409 duplicate_product`, not a second queue entry
        VERIFY: cargo test -p catalog a_duplicate_submission_is_a_conflict_not_a_queue_entry 2>&1 | grep -qE "^test .*a_duplicate_submission_is_a_conflict_not_a_queue_entry \.\.\. ok$"
  - [x] A medicine submission missing a required field names every missing field in `details`
        VERIFY: cargo test -p catalog a_medicine_submission_names_every_missing_field 2>&1 | grep -qE "^test .*a_medicine_submission_names_every_missing_field \.\.\. ok$"
  - [x] A product submitted with no pack is accepted, and one with a pack stores it as the central default
        VERIFY: cargo test -p catalog an_optional_pack_is_stored_with_the_submission 2>&1 | grep -qE "^test .*an_optional_pack_is_stored_with_the_submission \.\.\. ok$"
  - [x] A pharmacy cannot add a pack to a product it did not submit, or to an approved one
        VERIFY: cargo test -p catalog a_pack_insert_is_refused_outside_an_own_pending_product 2>&1 | grep -qE "^test .*a_pack_insert_is_refused_outside_an_own_pending_product \.\.\. ok$"
  - [x] A generic is submitted with its ingredients and classes in one transaction
        VERIFY: cargo test -p catalog a_generic_is_submitted_with_its_ingredients_in_one_transaction 2>&1 | grep -qE "^test .*a_generic_is_submitted_with_its_ingredients_in_one_transaction \.\.\. ok$"
  - [x] An issue report is filed `open` against exactly one visible row
        VERIFY: cargo test -p catalog an_issue_report_is_filed_open 2>&1 | grep -qE "^test .*an_issue_report_is_filed_open \.\.\. ok$"
  - [x] An issue report naming two targets is refused by the command, before the database
        VERIFY: cargo test -p catalog an_issue_report_with_two_targets_is_refused 2>&1 | grep -qE "^test .*an_issue_report_with_two_targets_is_refused \.\.\. ok$"

- [x] **Phase 5: the platform's review surface**
  - [x] Approval sets `approved`, records who and when, and publishes the row to everyone
        VERIFY: cargo test -p catalog approval_records_who_and_when_and_publishes_the_row 2>&1 | grep -qE "^test .*approval_records_who_and_when_and_publishes_the_row \.\.\. ok$"
  - [x] The queue offers up to five trigram matches per pending row, strongest first, and none when nothing is close
        VERIFY: cargo test -p catalog the_queue_suggests_near_duplicates_by_trigram 2>&1 | grep -qE "^test .*the_queue_suggests_near_duplicates_by_trigram \.\.\. ok$"
  - [x] Suggestions are one query for the page, not one per row
        VERIFY: cargo test -p catalog queue_suggestions_do_not_issue_a_query_per_row 2>&1 | grep -qE "^test .*queue_suggestions_do_not_issue_a_query_per_row \.\.\. ok$"
  - [x] Rejecting requires a note, at the command as well as the database
        VERIFY: cargo test -p catalog rejecting_requires_a_note 2>&1 | grep -qE "^test .*rejecting_requires_a_note \.\.\. ok$"
  - [x] A merge sets the target and moves no stock — the move belongs to `workflows`
        VERIFY: cargo test -p catalog a_merge_sets_the_target_and_moves_no_stock 2>&1 | grep -qE "^test .*a_merge_sets_the_target_and_moves_no_stock \.\.\. ok$"
  - [x] Merging into a row that is itself merged is refused, so no chain can start
        VERIFY: cargo test -p catalog merging_into_a_merged_row_is_refused 2>&1 | grep -qE "^test .*merging_into_a_merged_row_is_refused \.\.\. ok$"
  - [x] An already-reviewed row cannot be reviewed again
        VERIFY: cargo test -p catalog an_already_reviewed_row_cannot_be_reviewed_again 2>&1 | grep -qE "^test .*an_already_reviewed_row_cannot_be_reviewed_again \.\.\. ok$"
  - [x] The queue is one `UNION ALL` across the three tables, oldest first
        VERIFY: cargo test -p catalog the_review_queue_spans_all_three_kinds 2>&1 | grep -qE "^test .*the_review_queue_spans_all_three_kinds \.\.\. ok$"
  - [x] A generic and its monograph are written in one transaction
        VERIFY: cargo test -p catalog a_generic_and_its_monograph_are_written_in_one_transaction 2>&1 | grep -qE "^test .*a_generic_and_its_monograph_are_written_in_one_transaction \.\.\. ok$"
  - [x] Only platform staff write a lookup table — a pharmacy token is `403`
        VERIFY: cargo test -p catalog a_pharmacy_cannot_edit_a_lookup_table 2>&1 | grep -qE "^test .*a_pharmacy_cannot_edit_a_lookup_table \.\.\. ok$"
  - [x] Resolving or dismissing an issue report records the admin note, and closes it once
        VERIFY: cargo test -p catalog an_issue_report_closes_once_with_its_note 2>&1 | grep -qE "^test .*an_issue_report_closes_once_with_its_note \.\.\. ok$"
  - [x] Every platform write asserts `app.is_platform`, by opening with `begin_platform`
        VERIFY: test -z "$(grep -n 'Tx::begin(' crates/catalog/src/platform.rs)"

- [x] **Phase 6: isolation proof**
  - [x] A pending row of tenant A is invisible to tenant B
        VERIFY: cargo test -p catalog a_pending_row_is_invisible_to_another_pharmacy 2>&1 | grep -qE "^test .*a_pending_row_is_invisible_to_another_pharmacy \.\.\. ok$"
  - [x] Another pharmacy's pending row is `404`, never `403`
        VERIFY: cargo test -p catalog another_pharmacys_pending_row_is_404_not_403 2>&1 | grep -qE "^test .*another_pharmacys_pending_row_is_404_not_403 \.\.\. ok$"
  - [x] An issue report of tenant A is invisible to tenant B
        VERIFY: cargo test -p catalog an_issue_report_is_invisible_to_another_pharmacy 2>&1 | grep -qE "^test .*an_issue_report_is_invisible_to_another_pharmacy \.\.\. ok$"
  - [x] These tests fail when the read policy is dropped — they test RLS and not nothing
        VERIFY: cargo test -p catalog isolation_fails_when_the_product_read_policy_is_dropped 2>&1 | grep -qE "^test .*isolation_fails_when_the_product_read_policy_is_dropped \.\.\. ok$"
  - [x] The suite fails when no database is reachable, rather than reporting `ok`
        VERIFY: test -z "$(grep -rl 'None => return' crates/catalog/tests)"
  - [x] `app.is_platform` reads across every pharmacy's pending rows
        VERIFY: cargo test -p catalog app_is_platform_reads_every_pending_row 2>&1 | grep -qE "^test .*app_is_platform_reads_every_pending_row \.\.\. ok$"

- [x] **Phase 7: HTTP, wiring and Gate 5**
  - [x] `rest.rs` — every handler documented, pharmacy routes and `/admin` routes on separate routers
        VERIFY: cargo test -p catalog catalog_routes_are_mounted 2>&1 | grep -qE "^test .*catalog_routes_are_mounted \.\.\. ok$"
  - [x] Handlers carry no logic — no `sea_orm` or `sqlx` in `rest.rs`
        VERIFY: ! grep -rqE "(sea_orm|sqlx)::" crates/catalog/src/rest.rs
  - [x] Every endpoint enforces the permission its contract gives it
        VERIFY: cargo test -p catalog each_route_requires_its_contract_permission 2>&1 | grep -qE "^test .*each_route_requires_its_contract_permission \.\.\. ok$"
  - [x] The three review paths are three handlers behind one pattern, not one polymorphic write
        VERIFY: cargo test -p catalog each_record_kind_has_its_own_review_handler 2>&1 | grep -qE "^test .*each_record_kind_has_its_own_review_handler \.\.\. ok$"
  - [x] `server` mounts both routers and the OpenAPI document carries every path
        VERIFY: cargo test -p server catalog_paths_are_in_the_openapi_document 2>&1 | grep -qE "^test .*catalog_paths_are_in_the_openapi_document \.\.\. ok$"
  - [x] Whole workspace is clean
        VERIFY: cargo clippy --workspace --all-targets -- -D warnings

---

## Not in this list, deliberately

| Thing | Why |
|---|---|
| The one-time import of 30,671 products | **Q1** — unspecified. `SPEC.md` §5, `plan.md` §2. The Notion page that would hold it, *Data Import Plan*, is unwritten with no id. **No import task exists**, so there is nothing here to do wrongly — see `plan.md` §7, last row |
| The lean catalogue copy the phone holds, and how it refreshes | **10 Sync**'s contract. This module owns the tables and the read endpoints, not the delivery |
| Moving stock and sales onto the surviving product after a merge | **06 Inventory**, **07 Sales**, orchestrated by **09 Workflows** `[D-2]` `[D-97]`. A merge here is deliberately incomplete until those exist |
| A pharmacy's own pack sizes and prices | **04 Pricing**. `product_packs` here is a central default with no price `[D-34]` |

## On the assertion form

`cargo test NAME -- --exact` **passes when it matches nothing**, so a typo in a test name
reads as success. Every assertion above pipes through
`grep -qE "^test .*NAME \.\.\. ok$"` instead, which fails unless that exact test ran and
passed. Copy the form; do not shorten it.

Two assertions are deliberately structural rather than behavioural, by the rule in
[`../rules/enforcement.md`](../rules/enforcement.md): *every platform write opens with
`begin_platform`* and *the suite fails with no database* are both facts about the text of the
code, and a test could only assert them by reimplementing the grep.
