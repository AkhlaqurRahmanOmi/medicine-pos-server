# CONTRACT: 09 — Workflows

**Status:** Draft — Gate 1
**Crate:** `crates/workflows`
**Spec:** [`SPEC.md`](SPEC.md)
**Bounded context:** none — above the contexts

> Gate 1 artifact. Normally: the tables a module owns and the HTTP surface it exposes.
> **This module owns no table and exposes no route** (§1, §2) — so its contract is the Rust
> surface `server`, `sync` and `jobs` call, and the direction of the crate graph. The
> envelope, the error shape and the headers are
> [`../00-foundation/CONTRACT.md`](../00-foundation/CONTRACT.md) §2 and are not restated.
>
> Parameter and return types marked **deferred** wait on 05–08's Gate 1 ([`SPEC.md`](SPEC.md)
> §3, *Deferred*). They are named here so the shape is reviewable; they are not invented here.

---

## 1. Tables owned

**None, and this is a decision rather than an omission.**

A table is owned by exactly one module, and every row these five flows write already has an
owner: the sale and its lines belong to `07-sales`, the batch and the movement to
`06-inventory`, the supplier ledger to `05-procurement`, the customer ledger and the
collection to `08-customers`. A table owned here would be a table two modules write — which
is the coupling `[D-97]` removes.

What follows from that, concretely, so no reader has to infer it:

| Normally in this section | Here |
|---|---|
| Migration files | **This module writes none.** `crates/migration` gains nothing for module 09. There is no `migrations/` work item in its future `tasks.md` |
| `tenant_id`, composite keys, tenant-scoped uniques | Not applicable. The columns a flow fills are declared by the owning module's `CONTRACT.md` §1 |
| `FORCE ROW LEVEL SECURITY`, policies, grants | **No policy and no grant is authored here.** Isolation is not weaker for it: every statement a flow causes runs inside the caller's `Tx`, as the non-owner `pos_app` role, under the owning table's policy, with `app.tenant_id` already set ([`../rules/tenancy.md`](../rules/tenancy.md) rules 2–4). `[AC-U06]` forbids this crate from widening that — no `begin_platform`, no `BYPASSRLS` role, no second tenant |
| Append-only ledgers | Five of the seven flows write one — `stock_movements`, `supplier_ledger_entries`, `customer_ledger_entries`. All three are insert-only **by grant** in their owning module (rule 7). A workflow therefore has no correction path: a wrong entry is reversed by a new entry, never updated, and the reversal is itself a flow step |
| Triggers | None here. A running balance maintained by a trigger belongs to the table's module; a flow must not recompute what the trigger maintains. It **may** read one back mid-transaction — Q3 makes a module's reads generic over `C: ConnectionTrait`, so the read runs on the flow's own transaction and sees the trigger's effect — but the value it reads is the trigger's, never recomputed `[AC-U09]` |
| Check constraints | None here. A cross-module precondition no single table can express is checked in Rust, inside the flow, and fails as `[AC-W02]` |

### Historical copies

**This module owns no column, so it freezes none — but it is the transaction in which the
freezing happens**, and that is the part worth stating:

- A sale line's price, discount and VAT rate are copied onto the line *during* `sell`, from
  `04-pricing`, at the moment of the sale. Re-deriving them later would read today's price
  list and silently restate last month's invoice and last month's VAT return `[D-69]`.
- A batch's cost is copied during `post_purchase`, from the supplier's invoice. Re-deriving it
  from an average would make the margin on an old sale depend on a purchase made afterwards.
- A refund reads the copies off the original sale line rather than re-pricing `[D-16]`.
- A `transfer` moves stock at the batch's recorded cost, not at today's `[D-36]`.

Which columns these are is 04/05/06/07's to declare. **The invariant this layer owes them:**
a flow passes a frozen value through untouched and never recomputes one `[AC-U09]`.

---

## 2. HTTP surface

**None. This crate contains no handler, no route, no `axum` dependency and no
`#[utoipa::path]`.** It does not export an `OpenApiRouter` the way a module crate does
([`../00-foundation/CONTRACT.md`](../00-foundation/CONTRACT.md) §2), and nothing of it appears
in `/openapi.json` except through the routes below.

### Where the seven flows' routes live — **Q6, decided 2026-10-25**

A module crate cannot hold them: `sales` importing `workflows`, which imports `sales`, is
exactly the cycle `[D-97]` exists to prevent, and Cargo refuses it. So:

| Concern | Crate | Why |
|---|---|---|
| The route, the `utoipa::path`, the permission layers, the access-mode gate (and `[D-42]`'s same-day and permission checks for the void) | `crates/server` | The only crate that may import both `api` and `workflows`. Gate-0 exempt composition, which is what this is |
| The request and response structs, with `serde` + `utoipa` derives | `crates/workflows` | No single module owns "a sale plus its payments plus its ledger effect". A duplicate DTO in `server` would be a second definition of the wire shape, and it would drift |
| The commands, the tables, the error codes | the module crates | Unchanged |

**No module's `CONTRACT.md` may claim one of these seven paths as its own** — in particular 07's
may not declare `POST /v1/sales`. The paths themselves are 05–08's to name (deferred); the six
that will exist are the sale, the customer return, the purchase posting, the collection, the
transfer, the void (Q1) and the supplier-return posting (Q7). The accepted cost of this answer is that `crates/server` stops being pure
wiring and holds six endpoints; the rejected alternatives are in [`SPEC.md`](SPEC.md) §5.

### Error cases

**This module introduces no error code.** `AppError` and `ErrorCode` are `core`'s, the 409
codes belong to the module that raises them (`../00-foundation/CONTRACT.md` §2), and a
cross-module precondition no module owns fails as `validation_failed` (400) `[AC-W02]`.

| Condition | Status | Code | Message to the user |
|---|---|---|---|
| A step's precondition fails — no stock, expired batch, credit limit | 409 | the owning module's, **deferred** | the owning module's |
| A cross-module precondition fails — a transfer to the same branch, a collection against another branch's invoice | 400 | `validation_failed` | Plain language with a next action `[D-85]`; the owning modules supply the sentences |
| The same flow is replayed with the same root id | **200** | — | Not an error. The stored result is returned and nothing is written `[AC-E02]`, `[AC-E03]` |
| A lapsed subscription | 403 | `subscription_read_only` | Raised by the route, before the flow is entered — see `SPEC.md` §4 |
| Anything unexpected inside a step | 500 | `internal_error` | The transaction rolls back whole; the detail goes to the log with the trace id, never to the response |

**What the client may retry:** the whole flow, byte-identical, including every client-generated
id. That is safe by `[AC-E02]` and is the only retry this layer offers — there is no partial
state, so there is nothing else to retry.

---

## 3. Public Rust surface

Seven functions, one per file, re-exported from the crate root. There is no `repo`, no
`entity`, no `queries.rs` and no state struct: this crate holds behaviour only.

```rust
// Every workflow has this shape, and nothing else is public.
//   &Tx            the caller's transaction — opened, committed and rolled back by the caller
//   &TenantContext tenant, acting user, branch ids  [D-97]
//   -> plain data  owned structs defined in this crate, composed of module types

pub async fn sell(tx: &Tx, ctx: &TenantContext, input: SellInput)
    -> Result<SellResult, AppError>;              // 04 · 06 · 07 · 08

pub async fn refund(tx: &Tx, ctx: &TenantContext, input: RefundInput)
    -> Result<RefundResult, AppError>;            // 06 · 07 · 08  [D-16] [D-17] [D-43]

pub async fn post_purchase(tx: &Tx, ctx: &TenantContext, input: PostPurchaseInput)
    -> Result<PostPurchaseResult, AppError>;      // 05 · 06

pub async fn collect_due(tx: &Tx, ctx: &TenantContext, input: CollectDueInput)
    -> Result<CollectDueResult, AppError>;        // 07 · 08  [D-45]

pub async fn transfer(tx: &Tx, ctx: &TenantContext, input: TransferInput)
    -> Result<TransferResult, AppError>;          // 06, two branches  [D-36]

pub async fn void_sale(tx: &Tx, ctx: &TenantContext, input: VoidSaleInput)
    -> Result<VoidSaleResult, AppError>;          // 06 · 07 · 08  [D-42], Q1

pub async fn post_supplier_return(tx: &Tx, ctx: &TenantContext, input: PostSupplierReturnInput)
    -> Result<PostSupplierReturnResult, AppError>; // 05 · 06  Q7
```

`void_sale` is the sixth flow, added by answering **Q1** on 2026-10-25. `[D-42]`'s
preconditions — same day, online, nothing already returned, and the permission — are checked
by the **route** `[AC-U05]`; the flow cancels the sale, returns the stock and reverses the
ledger in the caller's one transaction. Its root id is the void's own, not the sale's, so a
replayed void is `[AC-E02]`'s no-op rather than a second reversal.

**A flow's reads.** Q3, decided 2026-10-25: a module's `queries.rs` functions are generic over
`C: ConnectionTrait`, so a workflow calls them with `tx.transaction()?` and sees its own
uncommitted writes. `[AC-U10]` carries the rule that the generic signature no longer enforces
— a read that decides whether a write is allowed locks with `FOR UPDATE` — and the textual
assertion that goes with it. `tenancy` and `catalog` have no generic reads today and each needs
a backlog item of its own; that is not this module's to file.

**The seven `*Input` and `*Result` types are deferred**, because each is composed of types
owned by 04–08 and those modules are Phase 0. Three properties of them are settled now:

1. **Every `*Input` carries the aggregate root id**, supplied by the caller as a UUIDv7, never
   generated inside the flow. That id is the idempotency key `[AC-E02]`.
2. **Every `*Result` carries the ids it created and a `replayed: bool`**, so the route can
   answer `201` or `200` `[AC-E03]` without a second query, and `sync` can report per item.
3. **No `*Input` field is a permission, a role or a token.** An authorised exception arrives as
   a plain already-granted parameter — `discount_override_granted: bool` rather than a slug to
   check `[AC-U05]`, `[D-12]`.

### Dependencies, and the assertion that keeps them one-directional

```
server / sync / jobs  ──►  workflows  ──►  pricing procurement inventory sales customers
                                                        │
                                                        ▼
                                                     db   core
```

| `workflows` may depend on | `workflows` may **not** depend on |
|---|---|
| `core`, `db` | `api` — it has no HTTP surface (§2) |
| `pricing`, `procurement`, `inventory`, `sales`, `customers` | `cache` — the idempotency guard is the caller's, the primary key is the record (`SPEC.md` §3) |
| `tenancy`, `catalog` only if a flow provably needs a command there | `sync`, `reporting`, `server`, `jobs`, `migration` |

Two `grep`s over `Cargo.toml` files assert the whole of `[AC-U01]`, in the form module 00 and
01 already use — one proving no module crate names `workflows`, one proving `workflows` names
neither `api` nor `cache` nor `sync`. They become `VERIFY:` lines at Gate 3; they are recorded
here because they are the contract, not a test idea.

**Called by:** `crates/server` (the seven routes, Q6), `10-sync` (replaying what a phone sold
offline `[D-24]`), `12-jobs` (sweeps that need a cross-module write — and `[D-2]`'s merge move
if **Q2** lands on (a)). **By nothing else, ever** `[D-97]`.

### What this module needs from `00-foundation`

| Need | Why | Status |
|---|---|---|
| A **resolver returning the branch set** a caller holds a slug at — not a `require_at_both` | `transfer` writes at a source and a destination branch, and `api::require_at` resolves one branch from the path. 06's Q4 (2026-10-25) put the two-branch assertion **inside the flow** `[AC-U05a]`, so what `api` owes is not a second gate but a way to hand the flow the set the caller already holds — `AuthUser::branches_with(slug)`, which is also what module 05 asked for | **Gap, narrowed.** One accessor on `AuthUser`, not a new middleware |
| `Tx` exposing the tenant and user it was opened with | `[AC-W03]` compares the `TenantContext` against the transaction; `Tx::tenant_id()` and `Tx::user_id()` already return them | **Exists.** `Tx` does **not** expose the branch ids, which is why `&TenantContext` survives as a parameter (Q6) |
| Nothing else | No new extractor, no new error code, no new envelope | — |

---

## 4. Events and jobs

| Trigger | Job | Schedule | Idempotent? |
|---|---|---|---|
| — | — | — | — |

**None, and a workflow may not create one** `[AC-U07]`. An `apalis` job enqueued inside a
transaction can be picked up before the commit — the worker would read rows that do not exist
yet — and would survive a rollback as work scheduled for a sale that never happened. So
anything a flow implies (a push, a receipt email, a low-stock alert) is enqueued by the
**caller, after the commit returns**, and belongs to `12-jobs`.

`12-jobs` calling *into* a workflow is the allowed direction and is how `[D-2]`'s merge move
runs (**Q2**, answered 2026-10-25): the job finds the tenants that stocked the duplicate and
calls a per-tenant workflow once each, under that tenant's `TenantContext` and `Tx`, so RLS
stays on `[AC-U06]`. Atomic per tenant, not overall, so the job is resumable and reports which
tenants it moved `[AC-E05]`.

---

## 5. Offline and sync

| Question | Answer |
|---|---|
| Can the mobile app create this offline? | Not this crate — it has no entities. It is the layer that makes an offline-origin write **replayable**, and **only `sell` is** (Q4, answered 2026-10-25: `[D-24]`'s "sales only" is strict). A return, a collection, a purchase posting, a transfer and a void all require a connection — `[D-42]` already says a void is online |
| Client-generated id? | **UUIDv7, generated on the phone, for every aggregate root a flow creates** ([`../rules/tenancy.md`](../rules/tenancy.md) rule 10). A flow never generates a root id. Child rows — movements, ledger entries, allocations — may take server-generated ids, because they are reachable only through a root that `[AC-E02]` refuses to write twice |
| What makes an upload idempotent | The **primary key of the root row**, checked inside the flow's own transaction before any write. Not a Redis key: `cache::idempotency_guard` expires, and a replay arrives hours `[D-26]` or days `[D-48]` later. The guard stays useful to the caller as a concurrency short-circuit; it is not the record |
| What a replay returns | The first application's result with `replayed: true`, and `200` on the wire — never `201`, never a duplicate, never a conflict `[AC-E02]`, `[AC-E03]` |
| Unit of atomicity | **One flow, one transaction.** Twelve uploaded sales are twelve transactions; the eleventh failing leaves ten committed. Batching and per-item reporting are `10-sync`'s |
| Can it be flagged rather than rejected? | **For four cases, yes and it must be** (Q5, answered 2026-10-25): oversell `[D-25]`, credit over the limit `[D-46]`, a batch that expired between sale and upload `[D-17]`, and a unit price changed server-side after the phone quoted it — the invoice keeps the phone's frozen copy. Everything else rejects `[AC-S03]`, including a sale naming a customer deleted since: a credit sale with no customer has nowhere to post its ledger entry. **The last two flag cases are not named by any decision and need amending into Notion** |
| Clock | A flow uses the timestamps the caller gives it. A sale made offline at 18:40 and uploaded at 22:10 happened at 18:40, in Asia/Dhaka; which field carries that is 07's to declare |
