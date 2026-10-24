# PLAN: 00 — Foundation

**Status:** Draft
**Crates:** `crates/core` · `crates/db` · `crates/api` · `crates/cache` · `crates/infra`

> Gate 2 artifact. [`SPEC.md`](SPEC.md) says what and why; this says how. Two items are
> **blocked** on the spec's open questions and are marked so — they are planned but not
> specified, and no task exists for them yet.

---

## 1. Crate layout

```
crates/
  core/                     # deps: none in-workspace
    config.rs               # typed env, parsed once, exits non-zero on error
    context.rs              # TenantContext, RequestContext, Actor
    ids.rs                  # TenantId, BranchId, UserId, DeviceId — newtypes over Uuid v7
    money.rs                # Money(Decimal), scale 2, Display as ৳
    error.rs                # AppError, ErrorCode
    clock.rs                # trait Clock
    ports.rs                # trait MailSender, trait FileStorage
    page.rs                 # Page<T>, PageRequest

  db/                       # deps: core
    pool.rs                 # Db — SeaORM connection; pool() exposes PgPool for sqlx [D-99]
    tx.rs                   # Tx, Tx::begin — the ONLY set_config in the workspace
    error.rs                # DbErr -> AppError
    canary.rs  #[cfg(test)] # a throwaway tenant table, to prove the RLS mechanism

  api/                      # deps: core, db
    envelope.rs             # ApiOk<T>: IntoResponse
    error.rs                # IntoResponse for AppError — the one mapping site
    extract.rs              # RequestId, AuthUser, TenantTx, Pagination
    permission.rs           # require("sales.create") route layer
    middleware.rs           # request id, tracing span
    health.rs               # /health, /health/ready
    openapi.rs              # utoipa root

  cache/                    # deps: core
    pool.rs · keys.rs · rate_limit.rs · idempotency.rs

  infra/                    # deps: core
    clock.rs                # SystemClock
    mail.rs                 # SmtpMailSender (lettre)
    storage.rs              # S3-compatible FileStorage
```

**Dependencies out:** `core` depends on nothing in the workspace; everything else depends on
`core`; `api` additionally on `db`.

**Dependencies in:** every module crate depends on `core`, `db` and `api`. `server` and
`jobs` additionally on `infra` — and **they are the only two**, which is how no module crate
transitively acquires an SMTP client.

---

## 2. Migrations

Foundation owns no tables. It does own three SQL functions that every module's policy calls,
which is the only way a policy stays one line.

### Before any migration — `docs/db/bootstrap.sql`

Run **once per database, by a human, as superuser**. Not a migration, because creating roles
needs privileges the migration role must not have.

```sql
CREATE ROLE pos_app  LOGIN PASSWORD '…';           -- subject to RLS, not table owner
CREATE ROLE pos_jobs LOGIN PASSWORD '…' BYPASSRLS; -- cross-tenant sweeps only
-- pos_owner is the migration role and owns the schema
```

### `m0001_foundation`

| Creates | Why |
|---|---|
| `app.current_tenant_id()` | `current_setting('app.tenant_id', true)::uuid`, `STABLE`, so a policy reads `tenant_id = app.current_tenant_id()` |
| `app.current_user_id()` | for audit triggers |
| `app.current_branch_ids()` | **BLOCKED — open question 2.** Return type depends on the wire form |
| default grants | `pos_app` gets `SELECT, INSERT, UPDATE` on future tables in `public`; never `DELETE` by default |

`STABLE` rather than `VOLATILE` matters: a `VOLATILE` function in a policy is re-evaluated per
row and the planner cannot use an index on `tenant_id`. This is the difference between a
sub-millisecond lookup and a sequential scan of every pharmacy's sales.

---

## 3. Types

```rust
// core/ids.rs — newtypes, #[repr(transparent)], free at runtime
pub struct TenantId(Uuid);   pub struct BranchId(Uuid);
pub struct UserId(Uuid);     pub struct DeviceId(Uuid);

// core/money.rs — Rule 9: no module can reach for f64 because Money is the only money type
pub struct Money(Decimal);   // scale 2, checked arithmetic, Display as "৳1,234.50"

// core/context.rs
pub struct TenantContext { tenant_id: TenantId, user_id: UserId, branch_ids: Vec<BranchId> }
pub struct RequestContext { request_id: RequestId, now: DateTime<Utc> }

// core/error.rs — serialised lower_snake, per the API Design Guide (Q1, answered)
pub enum ErrorCode { BadRequest, ValidationFailed, UnknownField,
                     InvalidToken, TokenExpired, DeviceRevoked,
                     PermissionDenied, BranchNotAllowed, SubscriptionReadOnly,
                     TenantSuspended, TenantPendingApproval,
                     NotFound, Conflict(&'static str), FileTooLarge, AppTooOld,
                     TooManyRequests, Internal }
//                   ^ the module's own 409 code: core cannot know what a batch is

pub struct AppError { code: ErrorCode, user_message: String, detail: Option<String> }
//                                     ^ shown to a pharmacy owner   ^ log and Sentry only [D-85]
```

**Permissions are `&'static str`, not an enum in `core`.** `core` cannot know what a sale is,
so a `Permission` enum there would need every module's slugs, which inverts the dependency.
Each module declares its own:

```rust
// crates/sales/src/lib.rs
pub mod perms { pub const CREATE: &str = "sales.create"; }
```

and `api::require(sales::perms::CREATE)` takes `&'static str`. The cost is that a typo
compiles; the mitigation is one test asserting every slug used in a route exists in the
permission seed, which catches it at Gate 5 rather than in production.

---

## 4. Commands

Foundation has no commands — it writes no domain data. `Tx::begin` is the nearest thing and
belongs to `db`.

| Function | Writes | Pre-conditions | Errors |
|---|---|---|---|
| `Tx::begin(&Db, &TenantContext)` | session settings only | context is populated | `Internal` if `set_config` fails — never proceeds with it unset |

---

## 5. Queries

| Function | Reads | Shape | sqlx or SeaORM |
|---|---|---|---|
| `health::live()` | nothing | `200` | neither |
| `health::ready(&Db, &Cache)` | `SELECT 1`, Redis `PING` | `200`/`503` | sqlx |

`Db::pool()` returns the underlying `PgPool` so module crates can use `query_as!` `[D-99]`.
Foundation proves the plumbing works; it writes no reports itself.

---

## 6. Test plan

Every `[AC-*]` maps to a test. Those needing a database run against a real Postgres — RLS,
grants and `set_config` exist nowhere else `[rules/tenancy.md]`.

| Criterion | Test | Database? |
|---|---|---|
| `[AC-U01]` `[AC-W01]` | `config_rejects_missing_database_url` | no |
| `[AC-U01]` | `config_rejects_malformed_port` | no |
| `[AC-E01]` | `tx_sets_tenant_id_from_bound_parameter` | yes |
| `[AC-E01]` | `tx_setting_is_transaction_local` | yes |
| `[AC-W02]` | `tx_aborts_when_setting_fails` | yes |
| `[AC-E02]` | `tx_commits_on_ok_and_rolls_back_on_err` | yes |
| `[AC-U03]` `[AC-U04]` | `rls_canary_blocks_cross_tenant_read` | yes |
| `[AC-U08]` | `every_error_code_maps_to_a_status` | no |
| `[AC-S01]` | `internal_error_hides_detail_in_production` | no |
| `[AC-E03]` | `request_id_is_generated_when_absent_and_kept_when_given` | no |
| `[AC-E04]` | `health_answers_while_database_is_down` | no |
| `[AC-E05]` | `ready_returns_503_when_redis_is_down` | yes |

### The canary table

Foundation has no tables, so there is nothing to prove isolation *on*. A test-only migration
creates `_rls_canary(tenant_id uuid, id uuid, note text)` with the standard policy, and the
isolation tests run against it. This tests the **mechanism** rather than any module's schema,
which is correct: the mechanism is foundation's product.

`tx_setting_is_transaction_local` is the one most worth writing carefully. Drop the `true`
from `set_config` and every other test still passes — the leak only appears when a *pooled
connection is reused by a different tenant*, which a single-threaded test will never see.
The test must open a transaction, set the tenant, commit, then check `current_setting` on the
same pooled connection is empty.

---

## 7. Risks

| Risk | Why it is plausible here | What catches it |
|---|---|---|
| `set_config` written with `format!` | It is the obvious way to write it, and it works | `rules/tenancy.md` grep assertion; `set_config` confined to one function |
| The app connects as `pos_owner` | Both URLs are in the same config struct and differ by one word | `rls_canary_blocks_cross_tenant_read` fails as owner — the canary is what makes this loud |
| Policy function left `VOLATILE` | Default if unstated; nothing fails, it just gets slow | Noted here; an `EXPLAIN` assertion is possible but deferred |
| Transaction leaks across requests | Only shows under connection reuse | `tx_setting_is_transaction_local` |
| `api` grows module knowledge | A permission check feels like it belongs there | `core` cannot see modules, so the slug stays `&'static str` |

---

## 8. Blocked — planned, not specified

One item, in [`SPEC.md`](SPEC.md) §5.

| Item | Blocked on | What unblocks it |
|---|---|---|
| `app.current_branch_ids()` and every branch-scoped policy | **Q2** — is `app.branch_ids` a Postgres array or comma-joined text? | `set_config` takes text only, so the policy must parse. Which form decides the function's return type and the policy's shape |

> **Q1 is answered (2026-10-21)** from the API Design Guide, §§2–5 and 7. There is no success
> wrapper — a response is the resource; lists are `{data, next_cursor, has_more}` with no
> total; errors are `{error:{code, message, details, trace_id}}` with `lower_snake` codes.
> `ApiOk<T>: IntoResponse` survives unchanged, which is why this was never a mechanism
> question. Shapes in [`CONTRACT.md`](CONTRACT.md) §2.

**`tenant_id` and `user_id` are unaffected by Q2**, so the transaction, the canary and all the
isolation tests proceed now. Only branch scoping waits.
