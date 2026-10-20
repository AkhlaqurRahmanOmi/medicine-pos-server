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
| **00** | Foundation | `core` `db` `api` `cache` `infra` | Phase 4 | 2/47 | `[SPEC APPROVED]` | Config, tenant context, `Tx` + set_config, error envelope, extractors, Redis, vendor adapters. 2 open questions |
| **01** | Catalogue | `catalog` | Phase 0 | 0/0 | `[NOT STARTED]` | Central medicine catalogue (~30,000 products), submissions, duplicate review |
| **02** | Tenancy | `tenancy` | Phase 0 | 0/0 | `[NOT STARTED]` | Pharmacies, branches, subscriptions, approval and suspension states |
| **03** | Identity | `identity` | Phase 0 | 0/0 | `[NOT STARTED]` | Email login, roles, permissions, device registration, PIN switching |
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

None yet — no tasks are ticked.

> When `cargo xtask verify --strict` reports a failure, list it here with the module, the task
> and the reason, until it is either fixed or un-ticked. A failing assertion that is not
> listed here is being hidden.

---

## Session log

| Date | What changed |
|---|---|
| 2026-10-19 | Workspace initialised. SDD scaffold installed: constitution, templates, runner, both enforcement gates, four agent entry points. |
