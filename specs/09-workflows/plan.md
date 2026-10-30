# PLAN: [NN] — [Module Title]

**Status:** Draft | In Review | Approved
**Crate:** `crates/[name]`

> Gate 2 artifact. How the module is built. `SPEC.md` says what and why; this says how.
> If a decision here contradicts the spec, the spec wins and this is corrected.

---

## 1. Crate layout

```
crates/[name]/
  Cargo.toml          # deps: core, db, api, [others]
  src/
    lib.rs            # mod repo;  pub mod commands;  pub mod queries;  pub mod rest;
    commands.rs       # writes — take &Tx and &TenantContext
    queries.rs        # reads — take &Db
    repo.rs           # SQL — private to this crate
    rest.rs           # axum handlers — no logic
    error.rs          # thiserror enum
    entity/           # generated SeaORM entities — private
```

**Dependencies out:** [which crates this one depends on, and why each is needed.]
**Dependencies in:** [which crates depend on this one. If any module crate appears here,
that is a constitution violation — route it through `workflows`.]

---

## 2. Migrations

In order. Each is raw SQL in `crates/migration`.

| # | Migration | Creates | RLS | Grants |
|---|---|---|---|---|
| 1 | `mNNNN_…` | [tables] | `FORCE` + policy | app: `SELECT, INSERT, UPDATE` · jobs: — |

**Append-only tables get insert-only grants.** State which, explicitly.

---

## 3. Types

```rust
// Domain types, newtypes, and the shapes crossing the crate boundary.
// Money is rust_decimal::Decimal. Ids are Uuid (v7). Timestamps are DateTime<Utc>.
```

---

## 4. Commands

| Function | Writes to | Pre-conditions | Errors |
|---|---|---|---|

---

## 5. Queries

| Function | Reads | Shape | sqlx macro or SeaORM? |
|---|---|---|---|

Reports and non-trivial queries use `sqlx::query_as!` — note which, so `cargo sqlx prepare`
is remembered.

---

## 6. Test plan

Every `[AC-*]` in the spec maps to at least one test name. Behaviour gets a test, not a grep.

| Acceptance criterion | Test name | Needs a database? |
|---|---|---|
| `[AC-U01]` | `module_invariant_name` | yes |

**RLS tests are mandatory for a tenant-owned module.** At minimum: a row from tenant A is
invisible to tenant B, and a write that names tenant B's id while acting as tenant A fails.

---

## 7. Risks

| Risk | Why it is plausible here | What catches it |
|---|---|---|
