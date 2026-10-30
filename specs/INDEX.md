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
| **00** | Foundation | `core` `db` `api` `cache` `infra` | Phase 5 | 54/54 | `[AWAITING GATE 5]` | Config, tenant context, `Tx` + set_config, error envelope, extractors, Redis, vendor adapters. **Q1 answered 2026-10-21** — envelope, `ErrorCode` and pagination rewritten to the API Design Guide. **Scalar added at `/docs` 2026-10-22**, with the document assembled from every module's `OpenApiRouter`. All 54 assertions pass for the implementing agent; Gate 5 needs an independent pass (Art. II.2). **Q2 answered 2026-10-24** — void, not chosen: no `app.branch_ids` setting exists, branch scoping is `require_at` in `api`, not RLS. 0 open questions  **Reopened 2026-10-25:** `audit_logs` is this module's (06 Q3) — cross-cutting, `tenant_id`-nullable, polymorphic by documented exception, written by every module and owned by none. It needs a new phase here: the table, its insert-only grant, and a helper a module can call inside its own transaction. `06-inventory` `[AC-E13]` is the first caller waiting. |
| **01** | Catalogue | `catalog` | Phase 5 | 69/69 | `[DONE]` | Central medicine catalogue (~30,000 products), submissions, duplicate review. Twelve shared tables with no `tenant_id` plus one tenant-owned, third RLS shape (approved to all, pending to its submitter), and `pg_trgm` search. Gate 5 verified (65/65 passed). **Phase 8 added and closed 2026-10-25, 65 → 66.** `BACKLOG.md` A1: this module's five permission slugs were seeded by no migration, so `catalog.view` could not be granted to any role and **every gated catalogue endpoint was unreachable for every user, the owner included** — found incidentally by module 05's Gate 1 pass, verified against the live database. `m0006_catalog_permissions` seeds the five rows with `[D-78]` assignability, the owner template `[D-79]`, and the owner/manager/cashier roles of the 74 pharmacies that already exist; applied by a human. **Verified independently (Art. II.2)** by an agent given only the module number: the assertion fails loudly when the database is unreachable, which is the failure mode that sank module 03's suite. **Phase 9 added and closed 2026-10-25, 66 → 69: Q1 is answered and the catalogue is no longer empty.** The medex scrape arrived as `Extra Docs/`, so the import is an **idempotent seeder** — `crates/catalog/src/bin/seed.rs` behind the off-by-default `seed` feature, not a migration (Art. V.4 immutability) and not an `xtask`. 30,450 products, 2,192 generics, 353 companies, 664 therapeutic classes, 148 dosage forms, ~7s; run twice to prove 0 inserted the second time. Product Spec §2's nine quality fixes are implemented and reproduce the page's own counts (5,237 forms recovered: tablet 2,557, capsule 706, injection 540). **It needs `pos_owner`** — there is no platform INSERT policy for the submittable tables, which also means a super admin cannot add a product at all (A6). Seven uncounted items in `BACKLOG.md` — the test's slug list is hand-copied rather than derived from `catalog::perms`, three of `m0006`'s six statements are asserted by nothing, `catalog.lookup.manage` is enforced nowhere, and who may file an issue report is undeclared judgement. |
| **02** | Tenancy | `tenancy` | Phase 5 | 73/73 | `[DONE]` | Pharmacies, branches, subscriptions, approval and suspension states. Gate 5 verified (73/73 passed). |
| **03** | Identity | `identity` | Phase 5 | 93/93 | `[DONE]` | Email login, roles, permissions, device registration, PIN switching. Gate 5 verified (93/93 passed). Phase 12 completed and verified: all endpoints permission-gated with `api::require`, five read handlers read in tenant transactions, token carries session id with per-request revocation check `[AC-U07]`, full contract surface routed and mounted, active-branch filtering on grants `[AC-U13]`, live database invariant assertions, and strict Gate 5 verification passing. |
| **04** | Pricing | `pricing` | Phase 0 | 0/0 | `[NOT STARTED]` | Per-pharmacy prices, VAT rates. **Not MRP change requests** — corrected 2026-10-25: Context 4 says *"Batch MRP and cost price belong to `batches` (context 6)"*, and `batch_mrp_change_requests` is owned by **06 Inventory** `[D-38]` |
| **05** | Procurement | `procurement` | Phase 1 | 0/0 | `[AWAITING REVIEW]` | **Gate 1 complete 2026-10-25, all four questions answered.** Nine tenant-owned tables including the append-only `supplier_ledger_entries` and `supplier_balances` — `SELECT`-only to `pos_app`, maintained by a `SECURITY DEFINER` trigger, so no code path can set a balance the ledger does not say. **Posting is 09's**: `post_purchase`, and now `post_supplier_return` as the **seventh flow** (Q1); both routes are `crates/server`'s (09 Q6) and this contract claims neither. **A posted purchase is never cancelled** (Q2) — a return or a stock adjustment corrects it, leaving both append-only tables intact. A line priced below cost is **accepted with a warning**, not refused (Q3). `supplier_payments` **gains `status`** so a wrong payment is cancelled rather than left standing (Q4). `[D-39]`'s advance is stored nowhere — the ledger carries the full payment and the balance may go negative. Free goods: 06 gets a line cost total plus a unit count, never a rounded per-unit cost. **Blocked only by 06 Inventory at Phase 0** |
| **06** | Inventory | `inventory` | Phase 1 | 0/0 | `[AWAITING REVIEW]` | **Gate 1 drafted 2026-10-25, four of five questions answered.** Eleven tenant-owned tables including the central append-only `stock_movements` and `stock_balances` — `SELECT`-only to `pos_app` behind a `SECURITY DEFINER` trigger, the shape 05 uses, corroborated by Context 6's own rule that *the application role may only `INSERT`*. **The cost basis is stored exactly, not as a rounded average** (`cost_total` + a unit count, per-unit derived and display-only); **`batches.unit_cost` is removed**. `[D-17]`'s "non-sellable" is a derived expiry read, not a write. **Q2:** `reorder_levels.product_id` → `products` (01) — Notion's ERD says so and its constraints name no FK, so the export's `tenant_products` has nothing behind it. **Q3:** `audit_logs` is 00's, **which reopens module 00**. **Q4:** `workflows::transfer` stays and earns its place as the one named exception to 09 `[AC-U05]` — it asserts the slug at both branches, which `require_at` cannot. **Q5:** no withdrawal of a pending MRP request. **Q1 (`sync_flags`) blocks nothing here** and travels with `10-sync`. |
| **07** | Sales | `sales` | Phase 0 | 0/0 | `[NOT STARTED]` | Invoices, payments, returns, credit notes, voids |
| **08** | Customers | `customers` | Phase 0 | 0/0 | `[NOT STARTED]` | Customers, credit limits, ledger, collections |
| **09** | Workflows | `workflows` | Phase 1 | 0/0 | `[AWAITING REVIEW]` | **Gate 1 complete 2026-10-25, all seven questions answered.** Q7, raised by module 05, added `post_supplier_return` as the **seventh flow**. Seven cross-module flows in one `Tx` — sell, refund, post purchase, collect due, transfer `[D-97]`, plus **`void_sale`** (Q1) and **`post_supplier_return`** (Q7). **Owns no table, no route, no permission and no error code**, and authors no migration. Idempotent on the client's UUIDv7 root id, checked in-transaction, **not** in Redis `[D-26]` `[D-48]`. `[D-2]`'s merge runs once per tenant under `12-jobs` (Q2); offline is strictly sales so `sync` replays `sell` alone (Q4); four offline failures flag rather than reject (Q5). **A module's `queries.rs` is generic over `C: ConnectionTrait`** so a flow reads inside its own transaction (Q3) — `[AC-U10]` carries the `FOR UPDATE` rule the generic signature no longer enforces. **Routes live in `crates/server`, wire types in `workflows`** (Q6), so **no module may claim one of the six paths**. **Blocked only by 06–08 at Phase 0 — 05 is at Gate 1 and its half of both its flows is provided** (SPEC §3, *Deferred*) — Gate 2 cannot be written against a guess. Sixteen Notion amendments pending — four applied 2026-10-25 (see Requirement drift) |
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

None — no assertions are currently failing. All 93 tasks for Module 03 pass under strict Gate 5 verification (`cargo xtask verify --module 03 --strict`).

### Previous findings resolved (2026-10-30)

Module 03 failed Gate 5 on 2026-10-25 with 13 un-ticked tasks (3 un-ticked false/vacuous assertions plus 10 Phase 12 items). All 13 items were implemented, verified, and ticked on 2026-10-30, achieving 93/93 passing assertions under strict verification. Full history retained below:

Three assertions were un-ticked on 2026-10-25 as false or vacuous and re-implemented for Phase 12:

| Task | Why the tick was wrong |
|---|---|
| *A revoked session is refused before its access token expires* | `PharmacyClaims` carries no session id, so `authenticate` has nothing to look up and never calls `is_session_revoked`. The test revoked a cache key and read it back — no token, no request, no refusal |
| *`02-tenancy`'s routes are reached by a real token* | The test builds the app with no database, so the token now yields a 500, and the test asserts only `!= 401` and `!= 403`. The token never reaches the route |
| *Every endpoint in this module is in the leakage suite* | `^test result: ok\.` matches "ok. 0 passed", so an empty suite passes. The suite is 5 tests against 26 handlers, none over HTTP |

**Requirements the code does not meet** — these fail the gate on their own, Art. II.3 being
explicit that a passing suite is never sufficient. No endpoint calls `api::require`, so all
17 permission slugs guard nothing; `list_users`, `get_user`, `list_roles`, `get_role` and
`list_devices` read on a bare connection, so under forced RLS the lists always answer `[]`
and the gets always 404; and much of `CONTRACT.md` §2 was never routed — `/me`, the password
reset routes, the session endpoints, the platform-user register.

**And the assertion form itself.** Every integration suite returns early and passes when the
database is unreachable (`get_db` ends in `.ok()`), which is ~45 of the 83 assertions,
Phase 9's isolation proof included. Twelve more are `read_to_string` + `contains` over a
migration's source, three of them on tasks whose own text says *by the database*. The
verifier confirmed the underlying schema facts are true in the live database, so those are
mis-tiered rather than false — but they are not the proof the tasks claim, and
`rules/enforcement.md` calls a grep that asserts behaviour a false assertion.

---

**Record of what was wrong, 2026-10-24 to 2026-10-25.** Twelve ticked assertions in module 03
failed the first time they were ever run, so the `[DONE]` 73/73 of 2026-10-22 was never true:

| What | Why it had never passed |
|---|---|
| 6 platform tests | `platform_users` seeded with a `name` column the table has never had, swallowed by `let _ =` |
| 1 platform login test | an 80-bit test TOTP secret, which `totp_rs` rejects as `SecretSize(80)` |
| 3 isolation tests, incl. the cross-tenant RLS proof | `roles` seeded with a non-existent `slug` column |
| 3 device tests | **production code**: `register_device` queried `branches.status`, where the column is `is_active` |
| 4 permission tests (Phase 11) | tenant-scoped reads on a bare connection, which forced RLS answers with an empty set rather than an error. `IdentityAuthenticator` returned `permissions: vec![]`, so every `require` denied, and `permissions_at` / `branch_access` had no caller outside tests |

Also fixed in that pass: four handlers signed JWTs with a hardcoded test secret while the
authenticator verified with the configured one, so no deployment with a real `JWT_SECRET`
could have accepted its own tokens.


> When `cargo xtask verify --strict` reports a failure, list it here with the module, the task
> and the reason, until it is either fixed or un-ticked. A failing assertion that is not
> listed here is being hidden.

---

## Requirement drift — Notion amendments pending

> Where a spec records an answer that **Notion does not yet say**. Article III makes Notion the
> source of truth, so each of these is a gap until someone edits the page. **Delete a row once
> its page is amended** — this register exists to be emptied, not to grow.

| Decision | What Notion says | What was decided, and where it is recorded |
|---|---|---|
| *Data Model, Context 5* | The ERD's attribute lists omit a `ledger_entry_id` on `purchases`, `supplier_returns` and `supplier_payments`, though the relationship lines and the BCNF note (*"the purchase, return or payment points at its entry"*) require one | The column is specified in `05-procurement/CONTRACT.md` §1 — the only shape that avoids the polymorphic FK the cross-context rules forbid. **Page:** Data Model & ERD, 2026-10-25 |
| *Data Model, Context 5* | Context 5 types `invoice_date`, `expiry_date` and `paid_at` as `date`, while *Rules that hold across every context* says dates are `timestamptz` | Resolved in favour of the specific: a bill's date is a calendar date in Asia/Dhaka. `posted_at` and `occurred_at` stay `timestamptz`. `05-procurement/CONTRACT.md` §1, 2026-10-25 |
| *Data Model, Contexts 5 & 6* | **The page contradicts itself on the cost basis.** Context 6 types `batches.unit_cost` as a stored *"average cost per base unit"*, while Context 5's *Derived, not stored* list says *"Effective cost per unit with free goods: `quantity × unit_cost ÷ (quantity + free_quantity)` — this is what feeds the batch's average cost"* | Resolved toward the page's own *derived* list: `stock_movements.unit_cost` becomes **`cost_total`** (a line total, which its comment already describes), `batches.unit_cost` is **removed**, and `stock_balances` gains `cost_basis_total` + `cost_basis_units`. `85.00 ÷ 11` does not terminate, so no decimal scale stores it truthfully. `06-inventory/SPEC.md` `[AC-U06]`, 2026-10-25 |
| *Data Model, Context 6* | *"**Zero-stock blocking** locks the balance row and checks it"* | **Not executable.** `SELECT … FOR UPDATE` needs `UPDATE` on at least one column, and the page's own grant rule makes `stock_balances` `SELECT`-only to `pos_app`. Proven against the dev database: the same statement on a `SELECT`-only table answers *permission denied for table pack_units*. The lock is taken on the **`batches`** row instead, where `pos_app` holds a column-level `UPDATE`. `06-inventory/SPEC.md`, 2026-10-25 |
| *Data Model, Contexts 5 & 6* | *"Every table filters on `tenant_id` **and** `branch_id` being in `app.branch_ids`"* | **There is no such setting.** Module 00's Q2 closed `app.branch_ids` as void on 2026-10-24: no policy reads a branch list and branch scoping is `require_at` plus a `WHERE` in the handler. The page reads as an RLS guarantee the system does not have — **the largest page↔repo gap so far**, and it applies to Context 5 too. `06-inventory/SPEC.md` §4 · `05-procurement/SPEC.md` §4, 2026-10-25 |
| *Data Model, Context 5* | *"[A posted purchase] can be cancelled only while all of its stock is still on hand"* | **Not in v1** — a posted purchase is never cancelled; a return or a stock adjustment corrects it, which is the page's own fallback. Narrower than Notion, deliberately: it keeps both append-only tables intact and needs no `cancellation` ledger type. `05-procurement/SPEC.md` `[AC-U05]`, Q2, 2026-10-25 |
| `[D-17]` | A return into an expired batch is accepted and marked non-sellable | The flag belongs in `10-sync`'s `sync_flags`, whose `type` enum has no value for it — the same gap 09's Q5 half-recorded for the two new offline flag cases. `06-inventory/SPEC.md` **Q1** and `09-workflows/SPEC.md` `[AC-S02]`, 2026-10-25 |
| `[D-97]` | **Amended 2026-10-25 to list six files.** It now needs a third amendment, to seven | **Amend it to stop enumerating files and state the rule instead** — *a flow is added when it provably spans two or more modules, and each addition is a decision* — which is what `09-workflows/SPEC.md` `[AC-W04]` already carries. Two amendments in two days for the same reason is the signal. `09-workflows/SPEC.md` §5 Q7, 2026-10-25 |
| *Data Model, Context 5* | `supplier_payments` has no status column, so a wrong payment cannot be voided | **`status` ∈ `active \| cancelled`**, with a cancelled payment excluded from the allocation check, the outstanding-bills read and the balance. `05-procurement/SPEC.md` `[AC-S07]`, Q4, 2026-10-25 |
| *no number* | Notion does not say who owns `audit_logs`, and no spec claimed it | **00 Foundation owns it** — infrastructure, not a bounded context. **Wants a decision number assigned in Notion**, and reopens a Phase 5 module. `06-inventory/SPEC.md` §5 Q3, 2026-10-25 |
| *no number* | Notion does not say a flow may authorise anything, and 09 `[AC-U05]` forbids it outright | **One named exception:** `transfer` asserts the caller's slug at both branches, because `require_at` resolves only one from the path. Shaped so the flow is *told* the branch set rather than resolving it. **Wants a decision number.** `09-workflows/SPEC.md` `[AC-U05a]`, 06 Q4, 2026-10-25 |
| *Product Spec §2* | *"Same product, different URL slug — **40 pairs** — de-duplicate on the medex numeric id inside the URL"* | **Wrong by two orders of magnitude.** 3,103 medex ids cover more than one product — differing by strength (990), dosage form (1,027) or brand name (219) — because one medex brand page lists several products. Only 867 are true duplicates. Since `idx_products_medex_id` is UNIQUE, writing that id would reject ~2,250 legitimate products, so **`products.medex_id` is left NULL** and identity comes from the natural key. Either the index is wrong about what a medex id identifies, or the column is. `01-catalog/SPEC.md` §5 Q1, 2026-10-25 |
| *Product Spec §4* | *"Central defaults offered where the source data knows the box size (**about 80% of rows** say things like `30's pack`)"* `[Proposed]` | **49%, not 80%** — `(N's pack: …)` matches 15,162 of 30,670 rows across 40-plus formats, while *"Tenants define pack sizes"* is `[Decided]`. `product_packs` is therefore seeded by nothing. The proposal was justified on the 80% figure, so the real one belongs on the page. `01-catalog/BACKLOG.md` A7, 2026-10-25 |
| *Product Spec* | *"For the **74** generics whose text differs between products, the import keeps the most common version"* `[Proposed]` | The rule is implemented as written, tie-broken on the longest text so two runs cannot disagree. The count is **75**. Trivial, recorded only because the page states a number. `01-catalog/SPEC.md` §5 Q1, 2026-10-25 |
| *Data Import Plan* | **The page is unwritten and has no id** in `NOTION.map`, so Article III has no source for the largest piece of work module 01 ever did | The whole of Q1's answer — mechanism, idempotency keys, the nine quality fixes, the owner-role requirement and the three tables left empty — lives in `01-catalog/SPEC.md` §5 and `tasks.md` Phase 9. **This is module 01's largest page↔repo gap.** 2026-10-25 |
| `[D-25]` · `[D-46]` | Offline oversell and over-limit credit are accepted and flagged | **Two more** offline failures are flagged: a batch expired between sale and upload `[D-17]`, and a price changed server-side. Everything else rejects, a deleted customer included. `09-workflows/SPEC.md` `[AC-S02]` · `[AC-S03]`, Q5, 2026-10-25. **Page:** Product Specification — wording unverified, left to a human |

**Applied to Notion 2026-10-25** — *Architecture & Tech Stack*, four amendments, verified by
re-fetching the page: `[D-97]`'s file list is marked non-exhaustive and lists six including
`void_sale`; a new **Cross-module HTTP surface** row records Q6; `[D-98]` now reads "generic
over `C: ConnectionTrait`" and carries the `FOR UPDATE` rule; `[D-95]`'s per-IP cap is dropped
with the carrier-NAT reason. §3's file listing, its "six files above" prose and the operations
table were updated to match, so §3 and §13 do not contradict each other.

---

## Session log

| Date | What changed |
|---|---|
| 2026-10-30 | Module 03 (Identity) **Phase 12 closed, 93/93 passed under strict Gate 5 verification**. All remaining Gate 5 tasks resolved: permission gates on all pharmacy endpoints, 5 read handlers in tenant transactions, tokens carry session id with per-request revocation check `[AC-U07]`, missing contract endpoints routed and mounted in OpenAPI router, inactive branch grant filtering `[AC-U13]`, live database invariant assertions replacing textual grep assertions, Redis lockout tests using live Redis, and mounted audience middleware verification. Workspace clippy completely clean with 0 warnings. Module 03 promoted to Phase 5 `[DONE]`. |
| 2026-10-19 | Workspace initialised. SDD scaffold installed: constitution, templates, runner, both enforcement gates, four agent entry points. |
| 2026-10-21 | Module 00 (Foundation) completed: core, db, api, cache, infra crates implemented, verified 48/48 tasks via cargo xtask verify. |
| 2026-10-22 | Module 02 reviewed at Phase 5 against its CONTRACT and `rules/tenancy.md`. 16 findings filed as `02-tenancy/BACKLOG.md`; the optional-task class defined in `rules/lifecycle.md`. No code changed. |
| 2026-10-22 | Module 03 (Identity) completed: implemented all 9 phases, all 73 tasks passing under strict Gate 5 verification (`cargo xtask verify --module 03 --strict`). Clippy clean across workspace. |
| 2026-10-23 | Module 01 (Catalogue) Gates 1–3 written from Notion Data Model Context 1 and Product Specification §§2–4: `SPEC.md`, `CONTRACT.md`, `plan.md`, `tasks.md` (61 tasks, none ticked). No crate, no migration, no code. 3 open questions raised, Q1 because the *Data Import Plan* page is unwritten. |
| 2026-10-24 | Module 00 **Q2 closed as void** — `app.branch_ids` was never set and no policy reads a branch list; branch scoping is `require_at` in `api`, not RLS. Recorded in `00-foundation/SPEC.md` §5. No code changed. |
| 2026-10-24 | Module 00 assertion `api holds no SQL` was failing on a false positive — its grep matched `Method::DELETE` in `middleware.rs`. Pattern narrowed to `DELETE FROM`; `api` holds no SQL. 54/54 again. |
| 2026-10-24 | Module 01 **Q2 answered**: near-duplicate suggestions ship in v1 as a trigram over `brand_name`, one `LATERAL` join on the index `[AC-E08]` already needs `[AC-E10]`. 63 → 65 tasks, none ticked. |
| 2026-10-24 | Module 03 **Q1–Q4 answered**. Q1/Q3: token lifetimes moved into `AuthConfig` with env overrides, defaulting to the values that were hardcoded; Q4: per-IP cap dropped — **`[D-95]` in Notion still describes one and needs amending**. In the same pass the hardcoded `JWT_SECRET_DEFAULT` was removed from 4 handlers (login signed with a test secret while the authenticator verified with the configured one), and 9 never-passing assertions were fixed. Module demoted from `[DONE]` to `[WIP]` 79/83. |
| 2026-10-25 | Module 01 **Q1 answered and Phase 9 closed, 66 → 69 — the catalogue is no longer empty.** The medex scrape arrived, so the one-time import is an idempotent seeder in module 01's own crate behind a `seed` feature: 30,450 products, 2,192 generics, 353 companies in ~7s, proven by running it twice. Product Spec §2's nine fixes reproduce the page's own counts. Four drift rows added: the medex id is **not** unique per product (3,103 ids cover several, against the page's *40 pairs*), `pack_info` covers 49% not 80%, 75 generics vary not 74, and *Data Import Plan* is still unwritten. Three backlog items: **no platform INSERT policy exists for companies, generics or products**, so the seeder needs `pos_owner` and a super admin cannot add a product; `product_packs` is seeded by nothing; `ingredients` has no source in the data at all. `Extra Docs/` gitignored — 136 MB is over GitHub's per-file limit. |
| 2026-10-25 | Module 01 **Phase 8 added and ticked, 65 → 66**, closing `BACKLOG.md` A1. `m0006_catalog_permissions` makes the five catalogue slugs grantable; **verified independently** (Art. II.2) by an agent given only the module number, which confirmed the assertion fails rather than passing vacuously with no database and checked the claim in the live database separately from the test. The same pass filed four uncounted backlog items, the largest being that the test's slug list is hand-copied and does not link the `catalog` crate — A1's own bug one level up — and that `m0006`'s backfill and manager/cashier grants, true in the database, are asserted by nothing. A repo-wide no-op `down` in three migrations is recorded as a rules gap rather than a module item. |
| 2026-10-25 | Module 06 **Q2–Q5 answered**. Q2 settled by reading Context 4 and the cross-cutting ERD rather than choosing: `reorder_levels.product_id` → `products`, since Notion's constraints name no FK and `tenant_products` is created by a first purchase. Q3 gives `audit_logs` to 00, **reopening a Phase 5 module**. Q4 keeps `workflows::transfer` with the two-branch assertion as `[AC-U05a]`, the one named exception to 09's no-authorisation rule, which also narrows the `api` gap from new middleware to one `AuthUser` accessor. Q5 leaves a pending MRP request un-withdrawable. Q1 travels with `10-sync`. |
| 2026-10-25 | Module 06 (Inventory) **Gate 1 drafted** from Notion *Data Model* Context 6: eleven tables, the central append-only stock ledger on 05's trigger shape, and **an exact cost basis rather than a stored average** — the page contradicts itself there, and `85.00 ÷ 11` settles it. Two requests from 05 and 09 refused with reasons (no allocation command, no flag columns). It also found that Context 6's documented zero-stock lock **cannot execute** under the page's own grants, proven against the dev database, and that both Contexts 5 and 6 describe RLS on `app.branch_ids`, a setting module 00 closed as void. 5 open questions. Five *Requirement drift* rows added. No crate, no code. |
| 2026-10-25 | Module 05 **Q1–Q4 answered**, closing its Gate 1. `post_supplier_return` becomes `workflows`' **seventh flow**, which also closes 09's Q7; a posted purchase is never cancelled, so no reversal flow and no `cancellation` ledger type are needed; a line priced below cost is accepted with a warning on the line; `supplier_payments` gains a `status` and a cancel route. Two more *Requirement drift* rows — `[D-97]` should state the rule rather than list files, and Context 5 needs the payment status column. |
| 2026-10-25 | Module 05 (Procurement) **Gate 1 drafted** from Notion *Data Model* Context 5: nine tables, an append-only supplier ledger with a `SECURITY DEFINER` balance trigger, and no claim on the posting route (09 Q6). 4 open questions. It also **reopened module 09 as Q7** — posting a supplier return is cross-module with no flow to own it — and provided 05's half of `post_purchase`. Two *Requirement drift* rows added for Context 5's missing `ledger_entry_id` columns and its `date` vs `timestamptz` conflict. No crate, no code. |
| 2026-10-25 | **Four amendments applied to Notion** (*Architecture & Tech Stack*): `[D-97]` non-exhaustive with six flows, a new Cross-module HTTP surface decision, `[D-98]` generic over `C: ConnectionTrait` with the `FOR UPDATE` rule, `[D-95]`'s per-IP cap dropped. Verified by re-fetch; two follow-up fixes for a comma splice and the operations table. `[D-25]`/`[D-46]` left to a human — its page was never fetched, so its wording is unverified. |
| 2026-10-25 | Module 09 **Q3 and Q6 answered**, closing Gate 1. Reads become generic over `C: ConnectionTrait` with `[AC-U10]` guarding the `FOR UPDATE` rule the signature no longer carries; routes go in `crates/server` with the wire types in `workflows`, so no module may claim one of the six paths. `tenancy` and `catalog` have no generic reads and each needs a backlog item. Module 09 is now blocked only by 05–08 being Phase 0. |
| 2026-10-25 | Module 09 **Q1, Q2, Q4 and Q5 answered**. A void becomes the sixth flow; `[D-2]`'s merge runs per tenant under `12-jobs` (`[AC-E05]`); offline stays strictly sales; the flag set grows to four with a rejection rule for the rest (`[AC-S02]`, `[AC-S03]`). `SPEC.md` and `CONTRACT.md` updated, 2 questions left. Three Notion amendments now pending — see Requirement drift. |
| 2026-10-25 | Module 09 (Workflows) **Gate 1 drafted** from Notion *Architecture & Tech Stack* §3 and `[D-97]`: `SPEC.md` and `CONTRACT.md` for the orchestration layer — owns no table, no route, no error code; one flow is one caller-owned transaction with no saga, outbox or compensation; idempotency is the root row's primary key. `plan.md` and `tasks.md` deliberately untouched: Gate 2 cannot be written while 05–08 are Phase 0. 6 open questions, 2 of them engineering questions needing decision rows. No crate, no code. |
| 2026-10-25 | Module 03 **failed Gate 5**, verified independently with no session context while the runner reported 83/83. Permission gates absent from every endpoint, five read handlers on a bare connection, `[AC-U07]` unimplementable as the token stands, `CONTRACT.md` §2 part-unrouted, and ~45 assertions that pass with no database. 3 un-ticked, 10 filed as Phase 12. Module demoted to Phase 4 `[GATE 5 FAILED]` 80/93. |
| 2026-10-25 | Module 03 **Phase 11 closed**, 83/83 strict. New `identity::auth::authenticate` resolves permissions and branches in a tenant transaction and `crates/server`'s authenticator delegates to it; `queries::resolve_access` does it in two queries, not one per branch; `IdentityPermissionSource` and `branch_access_cached` likewise read with the context set. Awaiting an independent Gate 5. |
| 2026-10-24 | Module 01 **Q3 answered**: a pack size is optional on submission, stored as the central default when given `[AC-E09]`. `SPEC.md`, `CONTRACT.md`, `plan.md` and `tasks.md` updated; one narrow `INSERT` policy added to `product_packs`; 61 → 63 tasks, none ticked. No code. |

