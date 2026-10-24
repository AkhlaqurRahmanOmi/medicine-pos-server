# TASKS: 00 — Foundation

> Governed by [`specs/CONSTITUTION.md`](../CONSTITUTION.md) **Article II** — every leaf task
> carries a `VERIFY:` line. A task is ticked **only** when its command exits 0. Run:
>
> ```bash
> cargo xtask verify --module 00
> ```
>
> The agent that implements a task does not verify it. `cargo build` and `cargo clippy`
> passing is necessary, never sufficient.
>
> **Article V applies throughout: write migrations, do not run them.** Tasks whose assertion
> needs a database say so; a human applies the migration first.
>
> **On the assertion form used below — read the note at the bottom before adding a task.**
> `cargo test NAME -- --exact` is not what you want and will pass without running anything.

---

- [x] **Phase 1: `core` — no database, no HTTP, no dependencies**
  - [x] Create the `core` crate with `ids.rs` — `TenantId`, `BranchId`, `UserId`, `DeviceId` as `#[repr(transparent)]` newtypes over `Uuid`, constructed v7
        VERIFY: cargo test -p core ids_are_uuid_v7 2>&1 | grep -qE "^test .*ids_are_uuid_v7 \.\.\. ok$"
  - [x] `money.rs` — `Money(Decimal)` at scale 2 with checked arithmetic and `৳` Display
        VERIFY: cargo test -p core money_keeps_two_decimal_places 2>&1 | grep -qE "^test .*money_keeps_two_decimal_places \.\.\. ok$"
  - [x] Prove no float can enter the money type
        VERIFY: test -d crates/core/src && test -z "$(grep -rlsE '\bf32\b|\bf64\b' crates/core/src)"
  - [x] `error.rs` — `ErrorCode` serialised `lower_snake`, with `Conflict(&'static str)` for a module's own 409 code, and `AppError` carrying a user message and a separate log detail
        VERIFY: cargo test -p core every_error_code_maps_to_a_status 2>&1 | grep -qE "^test .*every_error_code_maps_to_a_status \.\.\. ok$"
  - [x] `config.rs` — figment, typed, parsed once; separate `database.url` (owner) and `database.app_url` (non-owner)
        VERIFY: cargo test -p core config_rejects_missing_database_url 2>&1 | grep -qE "^test .*config_rejects_missing_database_url \.\.\. ok$"
  - [x] Config rejects a malformed value rather than defaulting
        VERIFY: cargo test -p core config_rejects_malformed_port 2>&1 | grep -qE "^test .*config_rejects_malformed_port \.\.\. ok$"
  - [x] `context.rs`, `clock.rs`, `ports.rs`, `page.rs` — `TenantContext`, `trait Clock`, `trait MailSender`, `trait FileStorage`, `Page<T>`
        VERIFY: cargo test -p core --lib 2>&1 | grep -qE "test result: ok\. [1-9]"
  - [x] `core` depends on nothing else in the workspace
        VERIFY: test -f crates/core/Cargo.toml && test -z "$(grep -ls 'path = \"\.\./' crates/core/Cargo.toml)"

- [x] **Phase 2: database bootstrap and the foundation migration**
  - [x] Write `docs/db/bootstrap.sql` creating `pos_app` and `pos_jobs` — **written, not run**
        VERIFY: test -f docs/db/bootstrap.sql && grep -q "BYPASSRLS" docs/db/bootstrap.sql
  - [x] `compose.yaml` brings up Postgres and Redis — tools only, every value from `.env`, no credential in the file, no migration run from it
        VERIFY: docker compose config -q && test -z "$(grep -nE '^[[:space:]]*[A-Z0-9_]*(PASSWORD|SECRET|ACCESS_KEY)[A-Z0-9_]*:[[:space:]]*[^$[:space:]]' compose.yaml)"
  - [x] `pos_app` is created without `BYPASSRLS` and is not the schema owner
        VERIFY: cargo test -p migration bootstrap_app_role_has_no_bypassrls 2>&1 | grep -qE "^test .*bootstrap_app_role_has_no_bypassrls \.\.\. ok$"
  - [x] Write migration `m0001_foundation` with `app.current_tenant_id()` and `app.current_user_id()`, both `STABLE`
        VERIFY: grep -q "STABLE" crates/migration/src/m0001_foundation.rs
  - [x] Default grants give `pos_app` no `DELETE`
        VERIFY: cargo test -p migration app_role_cannot_delete_by_default 2>&1 | grep -qE "^test .*app_role_cannot_delete_by_default \.\.\. ok$"

- [x] **Phase 3: `db` — the transaction that carries tenancy**
  - [x] `pool.rs` — `Db` connects as the **non-owner** role and exposes `pool()` for sqlx macros
        VERIFY: cargo test -p db pool_connects_as_non_owner 2>&1 | grep -qE "^test .*pool_connects_as_non_owner \.\.\. ok$"
  - [x] `tx.rs` — `Tx::begin` sets `app.tenant_id` and `app.user_id` via `set_config(name, $1, true)`
        VERIFY: cargo test -p db tx_sets_tenant_id_from_bound_parameter 2>&1 | grep -qE "^test .*tx_sets_tenant_id_from_bound_parameter \.\.\. ok$"
  - [x] The setting is transaction-local and does not survive onto a reused pooled connection
        VERIFY: cargo test -p db tx_setting_is_transaction_local 2>&1 | grep -qE "^test .*tx_setting_is_transaction_local \.\.\. ok$"
  - [x] A failed `set_config` aborts rather than continuing unset
        VERIFY: cargo test -p db tx_aborts_when_setting_fails 2>&1 | grep -qE "^test .*tx_aborts_when_setting_fails \.\.\. ok$"
  - [x] Commit on `Ok`, roll back on `Err`, without the caller saying so
        VERIFY: cargo test -p db tx_commits_on_ok_and_rolls_back_on_err 2>&1 | grep -qE "^test .*tx_commits_on_ok_and_rolls_back_on_err \.\.\. ok$"
  - [x] `set_config` appears in exactly one file in the whole workspace
        VERIFY: test "$(grep -rls 'set_config' crates/*/src | wc -l)" -eq 1
  - [x] `set_config` is never built with `format!`
        VERIFY: test -f crates/db/src/tx.rs && test -z "$(grep -rlsE 'format!.*set_config|set_config.*format!' crates)"
  - [x] `DbErr` maps to `AppError` in one place
        VERIFY: cargo test -p db db_error_maps_to_app_error 2>&1 | grep -qE "^test .*db_error_maps_to_app_error \.\.\. ok$"

- [x] **Phase 4: isolation proof — the canary**
  - [x] Test-only migration creating `_rls_canary` with `FORCE ROW LEVEL SECURITY` and the standard policy
        VERIFY: cargo test -p db canary_table_forces_rls 2>&1 | grep -qE "^test .*canary_table_forces_rls \.\.\. ok$"
  - [x] Tenant A cannot read tenant B's canary rows
        VERIFY: cargo test -p db rls_canary_blocks_cross_tenant_read 2>&1 | grep -qE "^test .*rls_canary_blocks_cross_tenant_read \.\.\. ok$"
  - [x] A write naming tenant B's id while acting as tenant A fails
        VERIFY: cargo test -p db rls_canary_blocks_cross_tenant_write 2>&1 | grep -qE "^test .*rls_canary_blocks_cross_tenant_write \.\.\. ok$"
  - [x] The canary test fails when the policy is dropped — proving it tests RLS and not nothing
        VERIFY: cargo test -p db canary_detects_missing_policy 2>&1 | grep -qE "^test .*canary_detects_missing_policy \.\.\. ok$"

- [x] **Phase 5: `cache`**
  - [x] `pool.rs` and `keys.rs` — deadpool-redis with a configured key prefix
        VERIFY: cargo test -p cache keys_are_prefixed 2>&1 | grep -qE "^test .*keys_are_prefixed \.\.\. ok$"
  - [x] `rate_limit.rs` — fixed-window counter `[D-94]`
        VERIFY: cargo test -p cache rate_limit_blocks_after_limit 2>&1 | grep -qE "^test .*rate_limit_blocks_after_limit \.\.\. ok$"
  - [x] The window expires and the counter resets
        VERIFY: cargo test -p cache rate_limit_window_expires 2>&1 | grep -qE "^test .*rate_limit_window_expires \.\.\. ok$"
  - [x] `idempotency.rs` — a guard keyed on a caller-supplied key
        VERIFY: cargo test -p cache idempotency_guard_rejects_replay 2>&1 | grep -qE "^test .*idempotency_guard_rejects_replay \.\.\. ok$"

- [x] **Phase 6: `api` — HTTP plumbing**
  - [x] `envelope.rs` — no success wrapper: a response is the resource, a list is `{data, next_cursor, has_more}`, an error is `{error:{code, message, details, trace_id}}` `[Q1 answered]`
        VERIFY: cargo test -p api error_body_is_a_single_error_object 2>&1 | grep -qE "^test .*error_body_is_a_single_error_object \.\.\. ok$"
  - [x] A list response carries no total count, and `has_more` agrees with `next_cursor`
        VERIFY: cargo test -p api list_response_has_no_total_count 2>&1 | grep -qE "^test .*list_response_has_no_total_count \.\.\. ok$"
  - [x] `error.rs` — `impl IntoResponse for AppError`, exhaustive over `ErrorCode`
        VERIFY: cargo test -p api every_error_code_has_a_response 2>&1 | grep -qE "^test .*every_error_code_has_a_response \.\.\. ok$"
  - [x] An internal error returns a generic message and a `trace_id`; the detail never reaches the body
        VERIFY: cargo test -p api internal_error_hides_detail 2>&1 | grep -qE "^test .*internal_error_hides_detail \.\.\. ok$"
  - [x] `middleware.rs` — request id generated when absent, preserved when given
        VERIFY: cargo test -p api request_id_generated_when_absent 2>&1 | grep -qE "^test .*request_id_generated_when_absent \.\.\. ok$"
  - [x] `extract.rs` — `TenantTx` extractor, so a handler without it cannot reach the database
        VERIFY: cargo test -p api tenant_tx_extractor_opens_and_commits 2>&1 | grep -qE "^test .*tenant_tx_extractor_opens_and_commits \.\.\. ok$"
  - [x] `extract.rs` — cursor pagination: opaque `cursor`, `limit` defaulting to 50 and capped at 200, no `page` or `offset`
        VERIFY: cargo test -p api pagination_rejects_oversized_limit 2>&1 | grep -qE "^test .*pagination_rejects_oversized_limit \.\.\. ok$"
  - [x] `permission.rs` — `require(slug)` route layer returning 403 when the slug is absent
        VERIFY: cargo test -p api require_returns_forbidden_without_permission 2>&1 | grep -qE "^test .*require_returns_forbidden_without_permission \.\.\. ok$"
  - [x] `health.rs` — `/health` answers while the database is down
        VERIFY: cargo test -p api health_answers_while_database_is_down 2>&1 | grep -qE "^test .*health_answers_while_database_is_down \.\.\. ok$"
  - [x] `/health/ready` returns 503 when a dependency is unreachable
        VERIFY: cargo test -p api ready_returns_503_when_dependency_down 2>&1 | grep -qE "^test .*ready_returns_503_when_dependency_down \.\.\. ok$"
  - [x] `openapi.rs` — utoipa root that other crates attach to
        VERIFY: cargo test -p api openapi_document_builds 2>&1 | grep -qE "^test .*openapi_document_builds \.\.\. ok$"
  - [x] `openapi.rs` — the root declares the `bearer` scheme the handlers cite
        VERIFY: cargo test -p api the_root_declares_the_bearer_scheme 2>&1 | grep -qE "^test .*the_root_declares_the_bearer_scheme \.\.\. ok$"
  - [x] `api` holds no SQL
        VERIFY: test -d crates/api/src && test -z "$(grep -rlsE '\b(SELECT|INSERT|UPDATE|DELETE FROM)\b' crates/api/src)"

- [x] **Phase 7: `infra` — vendor adapters, named by nobody else**
  - [x] `clock.rs` — `SystemClock`, plus a `FixedClock` for tests
        VERIFY: cargo test -p infra fixed_clock_is_deterministic 2>&1 | grep -qE "^test .*fixed_clock_is_deterministic \.\.\. ok$"
  - [x] `mail.rs` — `SmtpMailSender` implementing `core::MailSender`
        VERIFY: cargo test -p infra smtp_sender_builds_from_config 2>&1 | grep -qE "^test .*smtp_sender_builds_from_config \.\.\. ok$"
  - [x] `storage.rs` — S3-compatible `FileStorage` for payment slips and exports
        VERIFY: cargo test -p infra storage_builds_from_config 2>&1 | grep -qE "^test .*storage_builds_from_config \.\.\. ok$"
  - [x] No module crate depends on `infra` — only `server` and `jobs` may
        VERIFY: test -d crates/infra && test -z "$(grep -lsE '^infra' crates/{catalog,tenancy,identity,pricing,procurement,inventory,sales,customers,workflows,sync,reporting}/Cargo.toml)"

- [x] **Phase 8: wiring and Gate 5**
  - [x] `server` builds `AppState` from `Config` and mounts `/health` and `/openapi.json`
        VERIFY: cargo test -p server health_route_is_mounted 2>&1 | grep -qE "^test .*health_route_is_mounted \.\.\. ok$"
  - [x] `server` serves Scalar at `/docs`, with the document inlined in the page
        VERIFY: cargo test -p server scalar_is_served_at_docs_with_the_document_inlined 2>&1 | grep -qE "^test .*scalar_is_served_at_docs_with_the_document_inlined \.\.\. ok$"
  - [x] Every route the router mounts is in the document it serves — `split_for_parts()`, so the two cannot diverge
        VERIFY: cargo test -p server the_document_carries_every_mounted_module_path 2>&1 | grep -qE "^test .*the_document_carries_every_mounted_module_path \.\.\. ok$"
  - [x] The document offers a bearer token and tags the platform surface apart from the pharmacy one
        VERIFY: cargo test -p server the_document_offers_a_bearer_token_and_names_its_tags 2>&1 | grep -qE "^test .*the_document_offers_a_bearer_token_and_names_its_tags \.\.\. ok$"
  - [x] Whole workspace is clean
        VERIFY: test -d crates/api/src && cargo clippy --workspace --all-targets -- -D warnings
  - [x] `cargo sqlx prepare` output committed so the server builds without a database
        VERIFY: cargo sqlx prepare --workspace --check

---

## Blocked — no task until the question is answered

From [`SPEC.md`](SPEC.md) §5 and [`plan.md`](plan.md) §8. **Do not implement these by
guessing.** Each becomes a Phase 9 task once answered.

| Item | Blocked on |
|---|---|
| Serialised field names in `api/envelope.rs` | **Q1** — confirm against the API Design Guide. The mechanism (`ApiOk<T>: IntoResponse`) is settled; only the names are open |
| `app.current_branch_ids()` and every branch-scoped policy | **Q2** — is `app.branch_ids` an array or comma-joined text? `set_config` takes text, so the policy must parse it either way |

`tenant_id` and `user_id` are unaffected by Q2. Phases 1–8 proceed now; only branch scoping
waits.

---

## The assertion form, and why it is ugly

Every test assertion here greps the output instead of trusting the exit code. Both obvious
shorter forms are broken, and both fail in the direction that matters — **passing without
proving anything.** Verified in this repository, not assumed:

| Form | What it does |
|---|---|
| `cargo test -p x name -- --exact` | **Exits 0 when no test matches the filter.** An assertion for a test nobody has written yet passes |
| `cargo test -p x name -- --exact` for a unit test | `--exact` wants the *fully-qualified* name — `tests::name`, not `name` — so it filters everything out and then exits 0 |
| `cargo test -p x name 2>&1 \| grep -qE "^test .*name \.\.\. ok$"` | Passes only when that test actually ran and passed |

The same applies to negative greps: `! grep -q …` turns a **missing file** into a pass, so
every one here is `test -d <dir> && test -z "$(grep -ls …)"` — the directory must exist before
the absence of a pattern means anything.

When adding a task, copy an existing `VERIFY:` line and change the name. Do not shorten it.

---

## Phase 0: lint configuration (already done)

- [x] Workspace lints deny `perf`, `redundant_clone`, `unwrap_used`, `await_holding_lock`, and forbid `unsafe_code`; `clippy.toml` exempts tests
      VERIFY: grep -q "await_holding_lock" Cargo.toml && grep -q "allow-unwrap-in-tests" clippy.toml
- [x] Every crate opts in to the workspace lints
      VERIFY: test "$(grep -rls 'workspace = true' crates/*/Cargo.toml tools/*/Cargo.toml | wc -l)" -ge 3
