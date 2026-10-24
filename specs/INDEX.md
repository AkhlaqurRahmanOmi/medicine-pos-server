# Medicine POS Backend — SDD Dashboard

> Master rollup of every backend module. Governed by [`CONSTITUTION.md`](CONSTITUTION.md).
>
> **This file is the only place status is recorded.** Update it when a task or phase completes.
> The task count must match what `cargo xtask verify --module NN` reports — if it does not,
> the count is wrong, not the runner.

---

## Modules

Module numbers mirror the crate names and the bounded contexts in the data model, so Gate 0
is mechanically checkable: `crates/<name>` requires `specs/NN-<name>/SPEC.md`.

| # | Module | Crate | Phase | Tasks | Status | Notes |
|---|---|---|---|---|---|---|
| **00** | Foundation | `core` `db` `api` `cache` `infra` | Phase 5 | 54/54 | `[AWAITING GATE 5]` | Config, tenant context, `Tx` + set_config, error envelope, extractors, Redis, vendor adapters. **Q1 answered 2026-10-21** — envelope, `ErrorCode` and pagination rewritten to the API Design Guide. **Scalar added at `/docs` 2026-10-22**, with the document assembled from every module's `OpenApiRouter`. All 54 assertions pass for the implementing agent; Gate 5 needs an independent pass (Art. II.2). **Q2 answered 2026-10-24** — void, not chosen: no `app.branch_ids` setting exists, branch scoping is `require_at` in `api`, not RLS. 0 open questions |
| **01** | Catalogue | `catalog` | Phase 5 | 65/65 | `[DONE]` | Central medicine catalogue (~30,000 products), submissions, duplicate review. 65/65 tasks verified and passed. Twelve shared tables with no `tenant_id` plus one tenant-owned, third RLS shape (approved to all, pending to its submitter), and `pg_trgm` search. Gate 5 verified (65/65 passed). |
| **02** | Tenancy | `tenancy` | Phase 5 | 73/73 | `[DONE]` | Pharmacies, branches, subscriptions, approval and suspension states. Gate 5 verified (73/73 passed). |
| **03** | Identity | `identity` | Phase 4 | 79/83 | `[WIP]` | Email login, roles, permissions, device registration, PIN switching. **All four open questions answered 2026-10-24** — Q1/Q3 are configuration (`ACCESS_TOKEN_TTL_SECS`, `REFRESH_TOKEN_TTL_SECS`, `IMPERSONATION_TTL_SECS`), Q2 nothing server-side, Q4 no per-IP cap. **The earlier `[DONE]` 73/73 was not true**: 12 ticked assertions were failing against the real schema, 9 now fixed, and Phase 11's 4 remain — permission resolution never reaches the HTTP layer. See Failing assertions. |
| **04** | Pricing | `pricing` | Phase 0 | 0/0 | `[NOT STARTED]` | Per-pharmacy prices, MRP change requests, VAT rates |
| **05** | Procurement | `procurement` | Phase 0 | 0/0 | `[NOT STARTED]` | Purchases, supplier ledger, purchase returns |
| **06** | Inventory | `inventory` | Phase 0 | 0/0 | `[NOT STARTED]` | Batches, stock movements, balances, transfers, expiry |
| **07** | Sales | `sales` | Phase 0 | 0/0 | `[NOT STARTED]` | Invoices, payments, returns, credit notes, voids |
| **08** | Customers | `customers` | Phase 0 | 0/0 | `[NOT STARTED]` | Customers, credit limits, ledger, collections |
| **09** | Workflows | `workflows` | Phase 0 | 0/0 | `[NOT STARTED]` | Cross-module flows in one transaction: sell, refund, post purchase, collect due, transfer |
| **10** | Sync | `sync` | Phase 0 | 0/0 | `[NOT STARTED]` | Offline upload, idempotency, flags, reconciliation |
| **11** | Reporting | `reporting` | Phase 0 | 0/0 | `[NOT STARTED]` | Read-only queries across contexts, VAT reports, exports |
| **12** | Jobs | `jobs` | Phase 0 | 0/0 | `[NOT STARTED]` | Expiry sweeps, subscription lapses, purges, push |

Exempt from Gate 0 by Article I: `server` (the binary), `migration` (the migrator),
`tools/*` (the SDD runner itself).

---

## Phase key

- `Phase 0` — spec not started
- `Phase 1` — `SPEC.md` + `CONTRACT.md` in progress or awaiting review
- `Phase 2` — `plan.md` in progress or awaiting review
- `Phase 3` — `tasks.md` written, ready for implementation (`[SPEC APPROVED]`)
- `Phase 4` — implementing tasks (`[WIP]`)
- `Phase 5` — every `VERIFY:` passes (`[DONE]`)

---

## Build order

Dependency order, not priority order. Nothing above `db` works until the tenant transaction
extractor does.

```
00 Foundation  →  02 Tenancy  →  03 Identity  →  01 Catalogue  →  04 Pricing
                                                      ↓
                              06 Inventory  →  05 Procurement
                                                      ↓
                              08 Customers  →  07 Sales  →  09 Workflows
                                                      ↓
                                     10 Sync  →  11 Reporting  →  12 Jobs
```

---

## Failing assertions

**Module 03 — Phase 11, 4 assertions, found 2026-10-24.** Not regressions from that day's
work: each failed the first time it was ever run.

| Assertion | Why it fails |
|---|---|
| `an_authenticated_request_carries_resolved_permissions` | **No such test yet, and no code to test.** `IdentityAuthenticator::authenticate` in `crates/server/src/lib.rs` returns `permissions: vec![]` and `branch_permissions: Default::default()`, so every authenticated request carries no permissions and no branches. `queries::permissions_at` and `queries::branch_access` have **no caller outside tests** — the module's permission resolution is not wired to HTTP |
| `a_null_branch_grant_includes_a_branch_created_later` | `branch_access` runs raw SQL on a bare `DatabaseConnection` with no tenant GUCs, so forced RLS hides every branch and it returns an empty list `[D-14]` `[AC-U13]` |
| `permissions_at_a_branch_union_the_wide_and_the_local` | same cause — resolution outside a tenant transaction |
| `the_owner_role_holds_every_assignable_permission` | the owner template does not hold `02 Tenancy`'s assignable permissions, e.g. `branch.create` `[D-79]` |

**Fixed on 2026-10-24, having never passed:** `platform_users` seeded with a non-existent
`name` column (6 tests, error swallowed by `let _ =`); an 80-bit test TOTP secret that
`totp_rs` rejects as `SecretSize(80)`; `roles` seeded with a non-existent `slug` column
(3 tests, including the cross-tenant RLS proof); and in **production code**,
`register_device` querying `branches.status` on a table whose column is `is_active`.

> When `cargo xtask verify --strict` reports a failure, list it here with the module, the task
> and the reason, until it is either fixed or un-ticked. A failing assertion that is not
> listed here is being hidden.

---

## Session log

| Date | What changed |
|---|---|
| 2026-10-19 | Workspace initialised. SDD scaffold installed: constitution, templates, runner, both enforcement gates, four agent entry points. |
| 2026-10-21 | Module 00 (Foundation) completed: core, db, api, cache, infra crates implemented, verified 48/48 tasks via cargo xtask verify. |
| 2026-10-22 | Module 02 reviewed at Phase 5 against its CONTRACT and `rules/tenancy.md`. 16 findings filed as `02-tenancy/BACKLOG.md`; the optional-task class defined in `rules/lifecycle.md`. No code changed. |
| 2026-10-22 | Module 03 (Identity) completed: implemented all 9 phases, all 73 tasks passing under strict Gate 5 verification (`cargo xtask verify --module 03 --strict`). Clippy clean across workspace. |
| 2026-10-23 | Module 01 (Catalogue) Gates 1–3 written from Notion Data Model Context 1 and Product Specification §§2–4: `SPEC.md`, `CONTRACT.md`, `plan.md`, `tasks.md` (61 tasks, none ticked). No crate, no migration, no code. 3 open questions raised, Q1 because the *Data Import Plan* page is unwritten. |
| 2026-10-24 | Module 00 **Q2 closed as void** — `app.branch_ids` was never set and no policy reads a branch list; branch scoping is `require_at` in `api`, not RLS. Recorded in `00-foundation/SPEC.md` §5. No code changed. |
| 2026-10-24 | Module 00 assertion `api holds no SQL` was failing on a false positive — its grep matched `Method::DELETE` in `middleware.rs`. Pattern narrowed to `DELETE FROM`; `api` holds no SQL. 54/54 again. |
| 2026-10-24 | Module 01 **Q2 answered**: near-duplicate suggestions ship in v1 as a trigram over `brand_name`, one `LATERAL` join on the index `[AC-E08]` already needs `[AC-E10]`. 63 → 65 tasks, none ticked. |
| 2026-10-24 | Module 03 **Q1–Q4 answered**. Q1/Q3: token lifetimes moved into `AuthConfig` with env overrides, defaulting to the values that were hardcoded; Q4: per-IP cap dropped — **`[D-95]` in Notion still describes one and needs amending**. In the same pass the hardcoded `JWT_SECRET_DEFAULT` was removed from 4 handlers (login signed with a test secret while the authenticator verified with the configured one), and 9 never-passing assertions were fixed. Module demoted from `[DONE]` to `[WIP]` 79/83. |
| 2026-10-24 | Module 01 **Q3 answered**: a pack size is optional on submission, stored as the central default when given `[AC-E09]`. `SPEC.md`, `CONTRACT.md`, `plan.md` and `tasks.md` updated; one narrow `INSERT` policy added to `product_packs`; 61 → 63 tasks, none ticked. No code. |

