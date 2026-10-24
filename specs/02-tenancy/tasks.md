# TASKS: 02 — Tenancy

> Governed by [`specs/CONSTITUTION.md`](../CONSTITUTION.md) **Article II** — every leaf task
> carries a `VERIFY:` line. A task is ticked **only** when its command exits 0. Run:
>
> ```bash
> cargo xtask verify --module 02
> ```
>
> The agent that implements a task does not verify it. `cargo build` and `cargo clippy`
> passing is necessary, never sufficient.
>
> **Article V applies throughout: write the migration, do not run it.** Most assertions here
> need a database — a human applies `m0002_tenancy` first, against the Postgres from
> `compose.yaml`.
>
> Two tasks in Phase 2 edit **module 00's crates**. Each one updates
> [`../00-foundation/CONTRACT.md`](../00-foundation/CONTRACT.md) §3 in the same change, or the
> two specs disagree about what foundation exposes.

---

- [x] **Phase 1: `m0002_tenancy` — the tables, written not run**
  - [x] `CREATE EXTENSION btree_gist` is the first statement, before any constraint needs it
        VERIFY: test -f crates/migration/src/m0002_tenancy.rs && grep -q "btree_gist" crates/migration/src/m0002_tenancy.rs
  - [x] The four plan tables carry no `tenant_id`, and RLS is still enabled with a platform-write policy
        VERIFY: cargo test -p migration plan_tables_have_no_tenant_id 2>&1 | grep -qE "^test .*plan_tables_have_no_tenant_id \.\.\. ok$"
  - [x] Every tenant-owned table in this migration forces RLS and carries a policy
        VERIFY: cargo test -p migration every_tenancy_table_forces_rls 2>&1 | grep -qE "^test .*every_tenancy_table_forces_rls \.\.\. ok$"
  - [x] Exactly one default branch per pharmacy, by partial unique index
        VERIFY: cargo test -p tenancy a_second_default_branch_is_rejected_by_the_database 2>&1 | grep -qE "^test .*a_second_default_branch_is_rejected_by_the_database \.\.\. ok$"
  - [x] Two pharmacies may both have a branch called the same thing
        VERIFY: cargo test -p tenancy two_tenants_may_both_have_a_branch_called_main 2>&1 | grep -qE "^test .*two_tenants_may_both_have_a_branch_called_main \.\.\. ok$"
  - [x] Overlapping subscriptions are refused by the exclusion constraint, not by code
        VERIFY: cargo test -p tenancy overlapping_subscriptions_are_rejected_by_the_database 2>&1 | grep -qE "^test .*overlapping_subscriptions_are_rejected_by_the_database \.\.\. ok$"
  - [x] A subscription cannot exist at a `(plan_id, billing_period)` that does not
        VERIFY: cargo test -p tenancy a_subscription_at_a_nonexistent_price_is_rejected 2>&1 | grep -qE "^test .*a_subscription_at_a_nonexistent_price_is_rejected \.\.\. ok$"
  - [x] A payment reference cannot be reused among unrejected rows
        VERIFY: cargo test -p tenancy a_payment_reference_cannot_be_reused 2>&1 | grep -qE "^test .*a_payment_reference_cannot_be_reused \.\.\. ok$"
  - [x] `subscription_payments.tenant_id` is `ON DELETE SET NULL` — the row outlives the pharmacy
        VERIFY: cargo test -p tenancy a_purge_keeps_the_payment_row_and_nulls_its_tenant 2>&1 | grep -qE "^test .*a_purge_keeps_the_payment_row_and_nulls_its_tenant \.\.\. ok$"
  - [x] `pos_app` has no `DELETE` anywhere in this module
        VERIFY: cargo test -p tenancy app_role_cannot_delete_in_tenancy 2>&1 | grep -qE "^test .*app_role_cannot_delete_in_tenancy \.\.\. ok$"
  - [x] The seed creates one plan with every limit unlimited `[D-89]`, and a row per limit type
        VERIFY: cargo test -p tenancy the_seed_plan_has_a_row_for_every_limit_type 2>&1 | grep -qE "^test .*the_seed_plan_has_a_row_for_every_limit_type \.\.\. ok$"

- [x] **Phase 2: what foundation must add first**
  - [x] `Tx::begin_platform` in `crates/db` sets `app.is_platform` beside the tenant settings, with `set_config` still in one file
        VERIFY: cargo test -p db begin_platform_sets_is_platform_transaction_locally 2>&1 | grep -qE "^test .*begin_platform_sets_is_platform_transaction_locally \.\.\. ok$"
  - [x] `set_config` still appears in exactly one file after the addition
        VERIFY: test "$(grep -rls 'set_config' crates/*/src | wc -l)" -eq 1
  - [x] `trait AccessModeSource` in `crates/core/ports.rs`, so `api` never depends on a module crate
        VERIFY: cargo test -p core access_mode_source_is_object_safe 2>&1 | grep -qE "^test .*access_mode_source_is_object_safe \.\.\. ok$"
  - [x] `api` does not depend on `tenancy`
        VERIFY: test -f crates/api/Cargo.toml && test -z "$(grep -ls 'tenancy' crates/api/Cargo.toml)"

- [x] **Phase 3: the crate, its types, and the access rule**
  - [x] Create `crates/tenancy` mapped in `MODULES.map`, with `repo.rs` private
        VERIFY: test -f crates/tenancy/src/repo.rs && grep -qE "^mod repo;" crates/tenancy/src/lib.rs
  - [x] Entities generated, never hand-written — the generator header is intact
        VERIFY: test -d crates/tenancy/src/entity && test -z "$(grep -rLs 'sea-orm-codegen' crates/tenancy/src/entity/*.rs)"
  - [x] `types.rs` — API types with money as `Decimal`, and no generated entity on the wire
        VERIFY: test -z "$(grep -rlsE '\bf32\b|\bf64\b' crates/tenancy/src)" && test -z "$(grep -ls 'entity::' crates/tenancy/src/types.rs)"
  - [x] `decide(facts, now)` resolves every status × subscription combination
        VERIFY: cargo test -p tenancy access_mode_state_table 2>&1 | grep -qE "^test .*access_mode_state_table \.\.\. ok$"
  - [x] `Suspended and pending are different codes, never collapsed`
        VERIFY: cargo test -p tenancy suspended_and_pending_are_different_codes 2>&1 | grep -qE "^test .*suspended_and_pending_are_different_codes \.\.\. ok$"
  - [x] The grace period gives full access with a warning, and the warning carries its end
        VERIFY: cargo test -p tenancy inside_grace_is_full_with_a_warning 2>&1 | grep -qE "^test .*inside_grace_is_full_with_a_warning \.\.\. ok$"
  - [x] No column anywhere stores an access mode — it is derived `[AC-U02]`
        VERIFY: test -z "$(grep -rlsE 'access_mode[^(]*(varchar|text|enum)' crates/migration/src)"

- [x] **Phase 4: the pharmacy's own surface**
  - [x] `queries.rs` — `access_mode`, `tenant`, `settings`, `branches`, cursor-paginated
        VERIFY: cargo test -p tenancy branches_are_cursor_paginated 2>&1 | grep -qE "^test .*branches_are_cursor_paginated \.\.\. ok$"
  - [x] Creating a pharmacy creates its default branch and settings in one transaction
        VERIFY: cargo test -p tenancy creating_a_pharmacy_creates_one_default_branch_and_settings 2>&1 | grep -qE "^test .*creating_a_pharmacy_creates_one_default_branch_and_settings \.\.\. ok$"
  - [x] A duplicate branch name within a pharmacy is `409 duplicate_branch_name`
        VERIFY: cargo test -p tenancy duplicate_branch_name_within_a_tenant_is_rejected 2>&1 | grep -qE "^test .*duplicate_branch_name_within_a_tenant_is_rejected \.\.\. ok$"
  - [x] The default branch and the last active branch cannot be deactivated
        VERIFY: cargo test -p tenancy the_last_active_branch_cannot_be_deactivated 2>&1 | grep -qE "^test .*the_last_active_branch_cannot_be_deactivated \.\.\. ok$"
  - [x] `update_settings` is partial, and rejects a malformed VAT rate rather than defaulting
        VERIFY: cargo test -p tenancy settings_patch_rejects_a_malformed_vat_rate 2>&1 | grep -qE "^test .*settings_patch_rejects_a_malformed_vat_rate \.\.\. ok$"
  - [x] A pharmacy cannot create a payment in any state but `submitted`
        VERIFY: cargo test -p tenancy a_pharmacy_cannot_create_a_confirmed_payment 2>&1 | grep -qE "^test .*a_pharmacy_cannot_create_a_confirmed_payment \.\.\. ok$"
  - [x] `request_closure` sets `closing` with its purge date thirty days out
        VERIFY: cargo test -p tenancy closure_sets_a_purge_date_thirty_days_out 2>&1 | grep -qE "^test .*closure_sets_a_purge_date_thirty_days_out \.\.\. ok$"
  - [x] `limit_remaining` is one shared rule, and `NULL` means unlimited
        VERIFY: cargo test -p tenancy a_null_limit_is_unlimited 2>&1 | grep -qE "^test .*a_null_limit_is_unlimited \.\.\. ok$"
  - [x] Over a limit after a downgrade, a pharmacy keeps what it has and cannot add more
        VERIFY: cargo test -p tenancy over_a_limit_after_a_downgrade_keeps_what_it_has 2>&1 | grep -qE "^test .*over_a_limit_after_a_downgrade_keeps_what_it_has \.\.\. ok$"

- [x] **Phase 5: the platform's surface**
  - [x] `approve` sets `active` and records who and when
        VERIFY: cargo test -p tenancy approve_records_who_and_when 2>&1 | grep -qE "^test .*approve_records_who_and_when \.\.\. ok$"
  - [x] Suspension requires a reason, at the database and at the command
        VERIFY: cargo test -p tenancy suspension_requires_a_reason 2>&1 | grep -qE "^test .*suspension_requires_a_reason \.\.\. ok$"
  - [x] Confirming a payment activates its subscription in the same transaction
        VERIFY: cargo test -p tenancy confirming_a_payment_activates_its_subscription_in_one_transaction 2>&1 | grep -qE "^test .*confirming_a_payment_activates_its_subscription_in_one_transaction \.\.\. ok$"
  - [x] A rejected payment requires a review note
        VERIFY: cargo test -p tenancy rejecting_a_payment_requires_a_note 2>&1 | grep -qE "^test .*rejecting_a_payment_requires_a_note \.\.\. ok$"
  - [x] A complimentary subscription without a `grant_reason` is refused `[D-81]`
        VERIFY: cargo test -p tenancy a_complimentary_subscription_without_a_reason_is_rejected 2>&1 | grep -qE "^test .*a_complimentary_subscription_without_a_reason_is_rejected \.\.\. ok$"
  - [x] A plan price change does not alter a subscription already sold
        VERIFY: cargo test -p tenancy a_plan_price_change_does_not_alter_a_sold_subscription 2>&1 | grep -qE "^test .*a_plan_price_change_does_not_alter_a_sold_subscription \.\.\. ok$"
  - [x] An already-reviewed payment cannot be reviewed again
        VERIFY: cargo test -p tenancy an_already_reviewed_payment_cannot_be_reviewed_again 2>&1 | grep -qE "^test .*an_already_reviewed_payment_cannot_be_reviewed_again \.\.\. ok$"

- [x] **Phase 6: isolation proof**
  - [x] Tenant A cannot read tenant B's branches, settings, subscriptions or payments
        VERIFY: cargo test -p tenancy tenant_a_cannot_read_tenant_b 2>&1 | grep -qE "^test .*tenant_a_cannot_read_tenant_b \.\.\. ok$"
  - [x] A write naming tenant B's id while acting as tenant A fails
        VERIFY: cargo test -p tenancy a_write_naming_another_tenant_fails 2>&1 | grep -qE "^test .*a_write_naming_another_tenant_fails \.\.\. ok$"
  - [x] Another pharmacy's row is `404`, never `403`
        VERIFY: cargo test -p tenancy another_pharmacys_row_is_404_not_403 2>&1 | grep -qE "^test .*another_pharmacys_row_is_404_not_403 \.\.\. ok$"
  - [x] These tests fail when the policy is dropped — they test RLS and not nothing
        VERIFY: cargo test -p tenancy isolation_fails_when_the_branch_policy_is_dropped 2>&1 | grep -qE "^test .*isolation_fails_when_the_branch_policy_is_dropped \.\.\. ok$"
  - [x] `app.is_platform` reads across tenants on these tables
        VERIFY: cargo test -p tenancy app_is_platform_reads_across_tenants 2>&1 | grep -qE "^test .*app_is_platform_reads_across_tenants \.\.\. ok$"
  - [x] `app.is_platform` appears in no other module's migration
        VERIFY: test -z "$(grep -rls 'app.is_platform' crates/migration/src | grep -vE '(m0002_tenancy|m0003_identity|m0004_tenancy)')"

- [x] **Phase 7: the write gate every other module inherits**
  - [x] A write while read-only is `403 subscription_read_only`, and a read of the same data still succeeds
        VERIFY: cargo test -p tenancy read_only_rejects_a_write_and_allows_a_read 2>&1 | grep -qE "^test .*read_only_rejects_a_write_and_allows_a_read \.\.\. ok$"
  - [x] A blocked pharmacy is refused before a handler runs
        VERIFY: cargo test -p tenancy blocked_is_refused_before_the_handler 2>&1 | grep -qE "^test .*blocked_is_refused_before_the_handler \.\.\. ok$"
  - [x] The cached access mode is invalidated by every status and subscription write
        VERIFY: cargo test -p tenancy suspension_invalidates_the_cached_access_mode 2>&1 | grep -qE "^test .*suspension_invalidates_the_cached_access_mode \.\.\. ok$"
  - [x] An unreachable cache falls back to the database, and never to full access
        VERIFY: cargo test -p tenancy a_cache_outage_never_grants_full_access 2>&1 | grep -qE "^test .*a_cache_outage_never_grants_full_access \.\.\. ok$"

- [x] **Phase 8: HTTP, wiring and Gate 5**
  - [x] `rest.rs` — every handler documented, pharmacy routes and `/admin` routes on separate routers
        VERIFY: cargo test -p tenancy a_pharmacy_token_cannot_reach_an_admin_route 2>&1 | grep -qE "^test .*a_pharmacy_token_cannot_reach_an_admin_route \.\.\. ok$"
  - [x] Money and the VAT rate are strings on the wire, never numbers
        VERIFY: cargo test -p tenancy money_is_a_string_on_the_wire 2>&1 | grep -qE "^test .*money_is_a_string_on_the_wire \.\.\. ok$"
  - [x] `GET /tenant` carries the access summary, so the app needs no second call
        VERIFY: cargo test -p tenancy tenant_response_carries_the_access_summary 2>&1 | grep -qE "^test .*tenant_response_carries_the_access_summary \.\.\. ok$"
  - [x] `tenancy` holds no `anyhow`, and depends on no module crate
        VERIFY: test -z "$(grep -ls 'anyhow' crates/tenancy/Cargo.toml)" && test -z "$(grep -lsE '(catalog|identity|pricing|sales|inventory) *=' crates/tenancy/Cargo.toml)"
  - [x] `server` mounts both routers and wires `AccessModeSource`
        VERIFY: cargo test -p server tenancy_routes_are_mounted 2>&1 | grep -qE "^test .*tenancy_routes_are_mounted \.\.\. ok$"
  - [x] Whole workspace is clean
        VERIFY: cargo clippy --workspace --all-targets -- -D warnings

- [x] **Phase 9: Class A — Correctness and security**
  - [x] A1. A lapsed pharmacy can pay without first being un-lapsed
        VERIFY: cargo test -p tenancy a_read_only_tenant_can_still_submit_a_payment 2>&1 | grep -qE "^test .*a_read_only_tenant_can_still_submit_a_payment \.\.\. ok$"
  - [x] A2. A payment cannot name another pharmacy's subscription
        VERIFY: cargo test -p tenancy a_payment_cannot_name_another_tenants_subscription 2>&1 | grep -qE "^test .*a_payment_cannot_name_another_tenants_subscription \.\.\. ok$"
  - [x] A3. `pos_app` cannot confirm its own payment `[GATE]`
        VERIFY: cargo test -p tenancy pos_app_cannot_update_a_payment_or_insert_a_subscription 2>&1 | grep -qE "^test .*pos_app_cannot_update_a_payment_or_insert_a_subscription \.\.\. ok$"
  - [x] A4. The isolation suite fails when there is no database `[GATE]`
        VERIFY: test -z "$(grep -rl 'None => return' crates/tenancy/tests)"
  - [x] A5. The access gate fails closed
        VERIFY: cargo test -p api the_gate_refuses_when_no_access_source_is_wired 2>&1 | grep -qE "^test .*the_gate_refuses_when_no_access_source_is_wired \.\.\. ok$"
  - [x] A6. Suspension bites immediately
        VERIFY: cargo test -p tenancy a_suspension_survives_an_unreachable_cache 2>&1 | grep -qE "^test .*a_suspension_survives_an_unreachable_cache \.\.\. ok$"

- [x] **Phase 10: Class B — Contract surface completion**
  - [x] B1. The pharmacy's own missing endpoints
        VERIFY: cargo test -p tenancy every_pharmacy_contract_route_is_mounted 2>&1 | grep -qE "^test .*every_pharmacy_contract_route_is_mounted \.\.\. ok$"
  - [x] B2. The export request surface
        VERIFY: cargo test -p tenancy an_export_request_is_accepted_once_per_tenant 2>&1 | grep -qE "^test .*an_export_request_is_accepted_once_per_tenant \.\.\. ok$"
  - [x] B3. The platform's missing endpoints
        VERIFY: cargo test -p tenancy every_admin_contract_route_is_mounted 2>&1 | grep -qE "^test .*every_admin_contract_route_is_mounted \.\.\. ok$"
  - [x] B4. `GET /plans` returns a `ListPage`
        VERIFY: cargo test -p tenancy plans_are_returned_as_a_list_page 2>&1 | grep -qE "^test .*plans_are_returned_as_a_list_page \.\.\. ok$"
  - [x] B5. Every endpoint enforces the permission the contract gives it
        VERIFY: cargo test -p tenancy each_route_requires_its_contract_permission 2>&1 | grep -qE "^test .*each_route_requires_its_contract_permission \.\.\. ok$"

- [x] **Phase 11: Class C — Cost, concurrency, and types**
  - [x] C1. The gate's hot path reads two rows, not the whole history
        VERIFY: cargo test -p tenancy access_mode_reads_at_most_two_subscription_rows 2>&1 | grep -qE "^test .*access_mode_reads_at_most_two_subscription_rows \.\.\. ok$"
  - [x] C2. `get_plans` is not N+1
        VERIFY: cargo test -p tenancy the_plan_catalogue_is_one_query_per_table 2>&1 | grep -qE "^test .*the_plan_catalogue_is_one_query_per_table \.\.\. ok$"
  - [x] C3. One access summary, one tenant mapping
        VERIFY: test "$(grep -c 'full_with_warning' crates/tenancy/src/repo.rs)" = "1"
  - [x] C4. Database errors are classified by code, not by substring
        VERIFY: test -z "$(grep -n 'duplicate key' crates/tenancy/src/repo.rs)"
  - [x] C5. `limit_remaining` enforces every limit type it is given
        VERIFY: cargo test -p tenancy an_unimplemented_limit_type_is_an_error_not_unlimited 2>&1 | grep -qE "^test .*an_unimplemented_limit_type_is_an_error_not_unlimited \.\.\. ok$"
  - [x] C6. The plan limit holds under concurrency
        VERIFY: cargo test -p tenancy two_concurrent_branch_creates_cannot_both_pass_the_limit 2>&1 | grep -qE "^test .*two_concurrent_branch_creates_cannot_both_pass_the_limit \.\.\. ok$"
  - [x] C7. Status and mode are types, not strings
        VERIFY: cargo test -p tenancy an_unknown_tenant_status_is_an_error_not_pending 2>&1 | grep -qE "^test .*an_unknown_tenant_status_is_an_error_not_pending \.\.\. ok$"
  - [x] C8. A settings patch can clear a nullable field
        VERIFY: cargo test -p tenancy a_patch_can_clear_the_receipt_header 2>&1 | grep -qE "^test .*a_patch_can_clear_the_receipt_header \.\.\. ok$"

---

## Not in this list, deliberately

| Thing | Why |
|---|---|
| The endpoint that creates a pharmacy | **Q1** — unspecified. `SPEC.md` §5, `plan.md` §8 |
| The phone's `tenant_settings` read | **Q2** — `10-sync`'s contract decides the path |
| Building the export archive | `12-jobs`. This module owns the request row and its state only |
| The nightly purge | `12-jobs`, by the rule in `CONTRACT.md` §4. The `ON DELETE SET NULL` behaviour it depends on **is** asserted here |

All 19 optional items from the Phase 5 review have been promoted to Phases 9–11 above.

## On the assertion form

`cargo test NAME -- --exact` **passes when it matches nothing**, so a typo in a test name
reads as success. Every assertion above pipes through
`grep -qE "^test .*NAME \.\.\. ok$"` instead, which fails unless that exact test ran and
passed. Copy the form; do not shorten it.

