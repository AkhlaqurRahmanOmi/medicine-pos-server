# SPEC: 09 — Workflows

**Status:** Draft — Gate 1. **All six questions answered 2026-10-25.** It still does not
advance to Gate 2, for one reason only: `05`–`08` are Phase 0, so the six flows' internals
cannot be written yet (§3, *Deferred*). Two of the answers — Q3 and Q6 — are engineering
decisions with no number in Notion yet (`../INDEX.md`, *Requirement drift*).
**Crate:** `crates/workflows`
**Bounded context:** none — this module sits *above* the contexts, the way `00-foundation`
sits below them. It has no domain of its own.
**Owner:** Claude (`backend-architect` pass)

---

## 1. Purpose

Seven cross-module write flows, each one transaction. A workflow takes `&Tx` and
`&TenantContext`, calls module commands in the right order, and returns plain data `[D-97]`.
It exists so that a sale can touch pricing, inventory, customers and sales atomically
**without `sales` becoming the module that imports everyone** — the orchestration is lifted
one layer up and the crate graph stays acyclic in the direction Cargo can enforce.

The seven flows, and no eighth without a decision `[D-97]`:

| File | Flow | Spans |
|---|---|---|
| `sell.rs` | sale + lines + payments + stock movements + customer ledger | 04 · 06 · 07 · 08 |
| `refund.rs` | return + stock back in + credit note or ledger credit | 06 · 07 · 08 |
| `post_purchase.rs` | batch created + stock in + supplier ledger entry | 05 · 06 |
| `collect_due.rs` | collection + allocations to invoices + ledger | 07 · 08 |
| `transfer.rs` | stock out of one branch, in to another — **and the only thing it adds over 06's own command: the two-branch permission assertion** `[AC-U05a]` | 06 (two branches) |
| `void_sale.rs` | sale cancelled + stock back in + ledger reversed — **Q1, answered 2026-10-25** | 06 · 07 · 08 |
| `post_supplier_return.rs` | return posted + stock out + supplier ledger credited — **Q7, answered 2026-10-25** | 05 · 06 |

**Not this module's job:**

| Thing | Belongs to |
|---|---|
| Any table. This module owns **none** — see [`CONTRACT.md`](CONTRACT.md) §1 | the module that owns the row being written |
| Any HTTP route, status code, extractor or `utoipa` annotation | `crates/server` (routes) and `api` (envelope) |
| Any permission, branch-scope or access-mode check | `api::permission`, `api::auth`, `02-tenancy`'s access mode — **decided before the call** |
| Any single-module write — add a customer, set a price, adjust stock, create a supplier | that module's `commands.rs`, called directly `[D-97]` |
| Any read, list or report | that module's `queries.rs`, or `11-reporting` `[D-98]` |
| Invoice numbering, batch allocation order, ledger arithmetic, VAT rounding | the owning module. A workflow calls; it does not compute |
| Deciding *what* `sync` replays, batching an upload, the 90-day history `[D-48]` | `10-sync` |
| Scheduling, sweeps, retries, push | `12-jobs` |
| Raw SQL, an `entity` import, a `sqlx::query!` | the owning module's private `repo` |

A one-line test: **if the flow writes to tables owned by one module, it is not a workflow.**

---

## 2. Decisions this module implements

Cite, never restate. The reasoning lives in Notion.

| Ref | Decision | Consequence here |
|---|---|---|
| `[D-97]` | Cross-module flows go in a `workflows` crate; modules never call each other, nothing imports `workflows` | This crate depends on module crates; **no module crate, and not `api`, `cache`, `reporting` or `sync`, may depend on it.** Asserted as a `grep` over `Cargo.toml` files (§3 `[AC-U01]`) |
| `[D-98]` | `commands.rs` takes a transaction, `queries.rs` takes a connection — CQS by signature, no command bus | A workflow is a *caller of commands*, so it takes `&Tx` and never a `Db`. **Q3, answered 2026-10-25: a module's `queries.rs` functions are generic over `C: ConnectionTrait`**, so one function serves a bare connection for a route and the flow's own transaction for a workflow — the pattern `identity/src/queries.rs` already uses in nine functions. The read file stays the read file. **This softens `[D-98]`'s "takes a connection" to "takes a connection or a transaction" and needs amending in Notion**; what it costs is `[D-98]`'s signature-tells-you-which-side property, which `[AC-U10]` replaces with an explicit rule |
| `[D-2]` | A product rejected as a duplicate merges stock and sales onto the surviving product | **Q2, answered 2026-10-25: one `Tx` per tenant, driven by `12-jobs`.** The job finds the tenants that stocked the duplicate and calls a per-tenant workflow once each, under that tenant's context, so RLS stays intact and every write goes through the owning module's command. **Atomic per tenant, not overall**, so the job must be resumable and must report which tenants it moved `[AC-E05]`. A `BYPASSRLS` sweep was rejected: it writes tenant-owned rows with isolation off |
| `[D-24]` | Offline scope is sales only | **Q4, answered 2026-10-25: strictly sales.** Only `sell` is replayed by `10-sync`; `refund`, `collect_due`, `post_purchase`, `transfer` and `void_sale` are online-only, and `[D-42]` already says a void is online. All six stay replay-safe by construction, because `[AC-E02]` costs nothing to hold |
| `[D-25]` | Offline oversell: accept and flag | A workflow must be able to *record a flag* and commit. An oversell replayed from a phone is data, not an error — the rejection path belongs to the online route, not to the flow |
| `[D-46]` | Offline credit over the limit: accept and flag | As `[D-25]`. The credit-limit check is a precondition the caller applies online; the flow accepts and flags what the phone already sold |
| `[D-36]` | Branch transfer is instant, with no in-transit state | `transfer.rs` is one `Tx`: stock out and stock in commit together. There is no in-transit row, no second step and nothing to reconcile |
| `[D-45]` | Due collections are matched to the invoices they settle | `collect_due.rs` writes the collection, its allocations and the ledger entry in one `Tx` — an allocation that outlived its collection would be unexplainable money |
| `[D-16]` | A sales return is a cash refund or a credit note, invoice required, override by permission | `refund.rs` spans 06/07/08 and therefore lives here. The *override permission* is checked by the route (§4), never inside the flow |
| `[D-42]` | Voiding a sale: same day, online, with a permission, if nothing returned | **Q1, answered 2026-10-25: a void is the sixth flow**, `void_sale.rs`. It reverses stock (06) and the customer ledger (08) against a sale (07), so it is cross-module by the one-line test in §1, and atomicity is the whole point — an invoice cancelled with stock not returned is the failure this layer exists to prevent. `[D-42]`'s own preconditions (same day, online, nothing already returned) are the **route's**, not the flow's `[AC-U05]`. **`[D-97]`'s list of five is therefore not exhaustive and needs amending in Notion** — flagged, not edited |
| `[D-85]` | Stable error codes plus English; each client owns its wording | A workflow returns `AppError` and never a status code. It introduces no new code of its own (`CONTRACT.md` §2) |

---

## 3. Behaviour

Written as EARS acceptance criteria so each one can become a test name. These are the
criteria for **the layer**; the criteria for each flow's steps wait on 05–08 (*Deferred*,
below).

- **Ubiquitous (always true)**
  - `[AC-U01]` The crate graph SHALL stay one-directional: `workflows` may depend on module
    crates, `db` and `core`; **no crate other than `server`, `sync` and `jobs` SHALL depend
    on `workflows`**, and `workflows` SHALL NOT depend on `api`, `cache`, `reporting` or
    `sync` `[D-97]`.
  - `[AC-U02]` This crate SHALL contain no SQL, no `entity` import, no `sqlx::query*!`, no
    `sea_orm` model use and no `set_config`. Every row it writes is written by the command of
    the module that owns the table.
  - `[AC-U03]` Every workflow SHALL take `&Tx` and `&TenantContext` and SHALL NOT open,
    commit or roll back a transaction. The caller owns the transaction's lifetime —
    `api::TenantTx` for a route, `10-sync` for a replay, `12-jobs` for a sweep.
  - `[AC-U04]` A workflow SHALL return plain data — owned Rust structs — and SHALL NOT return
    an HTTP status, a response type, or anything from `api`.
  - `[AC-U05]` A workflow SHALL perform **no authorisation**: no permission slug, no branch
    check, no access-mode gate. Every such decision SHALL be made before the call, and an
    authorised exception (a discount over the role's limit `[D-12]`, a return override
    `[D-16]`) SHALL arrive as an explicit parameter the caller has already granted.
  - `[AC-U05a]` **One named exception, and no other** (06's Q4, answered 2026-10-25):
    `transfer` SHALL verify that the caller holds the transfer slug at **both** the source and
    the destination branch. It is the exception because the route layer provably cannot express
    it — `api::require_at` resolves a single branch from the path
    ([`../00-foundation/CONTRACT.md`](../00-foundation/CONTRACT.md) §3) — and because it is the
    only thing `transfer` adds over `inventory::commands::transfer_stock`, every table of which
    belongs to 06. The exception is shaped to keep `[AC-U05]` almost whole: the flow is **told**
    the branch set the caller holds the slug at, as a parameter the route has already resolved,
    and asserts both ends are in it. It SHALL NOT read `AuthUser`, resolve a permission or name
    a slug. **A second exception requires a decision**, not an appeal to this one.
  - `[AC-U06]` A workflow SHALL NOT use `Tx::begin_platform`, SHALL NOT run under a
    `BYPASSRLS` role, and SHALL NOT touch two tenants. Tenant isolation is the database's,
    under the caller's `Tx`.
  - `[AC-U07]` A workflow SHALL NOT enqueue a job, send mail, call a network service or
    `tokio::spawn`. Work that must happen after a flow is enqueued by the caller **after the
    commit returns** — a job enqueued inside the transaction can run before the commit, or
    survive a rollback.
  - `[AC-U08]` Every workflow SHALL acquire row locks in one documented order, shared by all
    seven flows, so that a sale, a transfer, a void and a return posting contending for the same batch cannot deadlock.
    (Which rows lock waits on 06/07/08 — *Deferred*.)
  - `[AC-U09]` Money crossing this layer SHALL be `rust_decimal::Decimal` (constitution
    Art. IV.5), and no workflow SHALL recompute a price, a VAT figure or a cost that a module
    command has already frozen onto a row.
  - `[AC-U10]` Every read a workflow performs SHALL be called with the caller's transaction,
    never a connection, and a read whose result decides whether a write is allowed — stock on
    hand before a sale, a customer's balance before a credit sale — SHALL lock the rows it
    reads with `FOR UPDATE`. This is the rule that replaces what `[D-98]`'s two-signature
    split used to say for free (Q3): once a module's `queries.rs` is generic over
    `C: ConnectionTrait`, the type no longer says which side the caller is on, so a read used
    for a locking decision and called on a bare connection compiles and silently permits the
    oversell `[D-25]` exists to describe. **Assertable as text:** no `FOR UPDATE` query in any
    module may sit in a function that is not generic over `C: ConnectionTrait`.
- **Event-driven (when)**
  - `[AC-E01]` WHEN a workflow is called, the system SHALL call the owning modules' commands
    in a fixed order and SHALL return the identifiers of every root row it created.
  - `[AC-E02]` WHEN a workflow is called with a root id that already exists for this tenant,
    the system SHALL write nothing and SHALL return the result of the first application
    (`[AC-E03]`), with a flag telling the caller it was a replay.
  - `[AC-E03]` WHEN the caller is a route, the repeat of `[AC-E02]` SHALL answer `200` with
    the existing resource, not `201` and not a conflict
    ([`../00-foundation/CONTRACT.md`](../00-foundation/CONTRACT.md) §2).
  - `[AC-E04]` WHEN the caller commits, every row of the flow SHALL become visible together;
    WHEN the caller rolls back, none SHALL.
  - `[AC-E05]` WHEN `12-jobs` runs `[D-2]`'s merge move, it SHALL call a per-tenant workflow
    once for each tenant that stocked the duplicate, each under that tenant's own
    `TenantContext` and `Tx` `[AC-U06]`, and SHALL be resumable: a run that fails partway
    SHALL leave each already-moved tenant committed and SHALL record which tenants remain. The
    move is **atomic per tenant and not overall**, which is a property of the answer to Q2 and
    not a defect to be fixed at this layer.
- **State-driven (while)**
  - `[AC-S01]` WHILE a workflow runs, it SHALL hold exactly one transaction — the caller's.
    There SHALL be no second connection and no `Db` read, so every read a flow performs sees
    the flow's own uncommitted writes (Q3).
  - `[AC-S02]` WHILE a flow originated offline, a precondition the phone could not check
    SHALL be recorded as a flag and committed, never rejected, for **four** cases — **Q5,
    answered 2026-10-25**: oversell `[D-25]`, credit over the limit `[D-46]`, a batch that
    expired between the sale and the upload `[D-17]`, and a unit price the pharmacy changed
    server-side after the phone quoted it. The reasoning is the same in all four: the medicine
    left the shop and the customer paid, so a rejection discards a transaction that already
    happened. In the price case the invoice SHALL keep the phone's frozen copy `[AC-U09]` —
    re-pricing on replay would restate a receipt the customer is holding.
  - `[AC-S03]` WHILE a flow originated offline, every *other* precondition failure SHALL
    reject (`[AC-W01]`) and SHALL be reported per item by `10-sync`. **A sale naming a
    customer deleted server-side since is a rejection**, decided 2026-10-25: a credit sale
    with no customer has nowhere to post its ledger entry, and inventing one would be worse
    than handing the row to a human. **Only `[D-25]` and `[D-46]` are named by a decision, so
    the expired-batch and changed-price cases need amending into Notion** — flagged, not
    edited.
- **Unwanted (if / then)**
  - `[AC-W01]` IF any step fails, THEN the workflow SHALL return `AppError` without having
    attempted compensation, the caller's transaction SHALL roll back whole, and the client
    SHALL be able to retry the identical payload — same ids — safely, by `[AC-E02]`. There is
    no partial state to repair, so there is no saga, no outbox and nothing to reconcile.
  - `[AC-W02]` IF a flow's precondition fails, THEN the error code SHALL be the owning
    module's (`insufficient_stock`, `batch_expired`, `over_credit_limit`), or
    `validation_failed` for a cross-module precondition no single module owns. This crate
    SHALL declare no error code of its own.
  - `[AC-W03]` IF a caller passes a `TenantContext` whose tenant differs from the one the
    `Tx` was opened with, THEN the flow SHALL fail rather than write; the two are redundant on
    purpose and disagreement is a programming error. The parameter **survives** Q6: `Tx`
    exposes `tenant_id()` and `user_id()` but not the branch ids, which `transfer` needs, and
    `[D-97]`'s own text specifies it.
  - `[AC-W04]` IF a new cross-module flow is proposed, THEN it SHALL NOT be added here until a
    decision names it `[D-97]`. A **seventh** file is a requirement question, not a refactor —
    the sixth (`void_sale.rs`, Q1) and the seventh (`post_supplier_return.rs`, Q7) were both
    added this way on 2026-10-25, and both are pending in Notion.

**Offline behaviour.** Nothing in this crate runs on a phone, and **only `sell` is replayed**
(Q4, answered 2026-10-25: `[D-24]`'s "sales only" is strict, so a return and a collection need
a connection). What it owes the phone is
**replay safety**: the aggregate root id of anything creatable offline is a UUIDv7 generated
on the device ([`../rules/tenancy.md`](../rules/tenancy.md) rule 10), so a repeated upload is the *same* sale, not a
second one. The replay decision is made in the database, inside the flow's own transaction,
against the primary key of the root row — not against a Redis key, because
`cache::idempotency_guard` expires and a replay can arrive hours `[D-26]` or days later
`[D-48]`. The guard remains useful to the caller as a concurrency short-circuit; it is not
the record. Child rows (stock movements, ledger entries, allocations) may take server-generated
ids, because they are reachable only through a root that `[AC-E02]` has already refused to
write twice. The unit of atomicity is **one flow**, so an upload of twelve sales is twelve
transactions and the eleventh failing leaves ten committed — how `10-sync` batches and reports
that is its decision, not this module's.

### Deferred — what waits on 05, 06, 07 and 08

Modules 05 Procurement, 06 Inventory, 07 Sales and 08 Customers are **Phase 0: no `SPEC.md`,
no `CONTRACT.md`, no crate, no tables.** Writing the six flows' steps now would mean
inventing their commands, their tables and their error codes, which Article III forbids.
Nothing in this table is an open *question* — each is settled work that is simply not this
module's to do first.

| Deferred until | Its Gate 1 must give this module | Without it we cannot write |
|---|---|---|
| **04 Pricing** Gate 1 | the command that resolves and freezes a line price, VAT rate and discount at sale time, plus the **changed-price flag** `[AC-S02]` now needs | `sell`'s first step and the historical-copy column list (`CONTRACT.md` §1) |
| ~~**05 Procurement** Gate 1~~ **provided 2026-10-25** | `begin_posting` / `complete_posting` and a `Posting` enum, plus the supplier-invoice rule — whose answer is sharper than this table asked for: the partial unique index covers **drafts**, so two drafts for one bill cannot coexist and `409 duplicate_supplier_invoice` is raised when a draft is saved, **never by the posting flow** | `post_purchase.rs`'s 05 half is unblocked; its 06 half still waits |
| ~~**06 Inventory** Gate 1~~ **answered 2026-10-25** | Batch creation, stock-in, stock-out and the generic `FOR UPDATE` read are provided, and 06 discharges `[AC-U08]` for its half with one lock order — ascending `(tenant_id, batch_id)`, taken on the **`batches`** row, because `SELECT … FOR UPDATE` needs an `UPDATE` privilege that `stock_balances` deliberately withholds. **Two requests came back refused, with reasons:** no allocation command — `[D-4]` gives the batch choice to a person and `[D-9]` makes each choice its own line, so an allocator would decide what a human decides — and no flag columns, because the flag is `10-sync`'s `sync_flags`, not 06's to write. `[AC-S02]` therefore reads the facts 06 returns (`shortfall`, `batch_expired_at_occurrence`) and the caller writes the flag row | 06's half of every flow's stock step is unblocked; 07's and 08's halves still wait |
| **07 Sales** Gate 1 | sale/line/payment/return/credit-note commands, invoice numbering, the void command and its same-day rule `[D-42]`, what a sale's root id is | `sell`, `refund`, `void_sale`, `collect_due` allocations, `[AC-E02]`'s lookup |
| **08 Customers** Gate 1 | customer-ledger entry, credit-limit check, collection and allocation commands `[D-45]`, the over-limit flag `[D-46]` | `sell`'s ledger step, `collect_due.rs` in full |
| **10 Sync** Gate 1 | its batch and per-item reporting, including how it reports `[AC-S03]`'s rejections and `[AC-S02]`'s four flags, and the 90-day retention `[D-48]`. **Which flows it replays is settled** — `sell` alone (Q4) | the replay contract's caller side (`CONTRACT.md` §5) |
| all of the above | each module's 409 codes | the error table of `CONTRACT.md` §2 |

**What this means for the gate.** This spec and `CONTRACT.md` describe the layer: the
dependency direction, the transaction and error boundary, idempotency, and what a workflow is
forbidden to be. `plan.md` (Gate 2) cannot be written against a guess
([`../rules/lifecycle.md`](../rules/lifecycle.md)), so module 09 stays at Phase 1 until
05–08 reach Gate 1 and §5 is answered.

---

## 4. Tenancy and permissions

| Question | Answer |
|---|---|
| Tenant-owned? | **No tables at all** — so nothing to own. Every row a flow writes is tenant-owned by its module, protected by that module's policy, under the caller's `Tx` |
| Branch-scoped? | The flows are: a sale happens at a branch, a transfer at two. This crate carries the branch ids in `TenantContext` and passes them through; it interprets nothing |
| Permissions required | **None declared here.** Each flow's route requires the owning module's slugs (05–08, deferred). A route that calls a workflow needs *every* slug the flow writes under — a sale is not authorised by a sales permission alone, and a void needs `[D-42]`'s own permission on top |
| Visible to a cashier? | Not applicable — no fields, no responses of its own |

Three things that are **specific** to this layer:

1. **The gate is the caller's, and it is strict.** Permission, branch scope and the access
   mode must all be decided before the flow is entered: a read-only subscription is `403`
   `subscription_read_only` at the route ([`../02-tenancy/SPEC.md`](../02-tenancy/SPEC.md)
   §3), never a flow that silently writes nothing. `10-sync` and `12-jobs` are callers too
   and owe the same gate — a replay is not an exemption from it.
2. **A transfer needs permission at two branches.** `api::require_at` resolves one branch
   from the path (`../00-foundation/CONTRACT.md` §3). The two-branch check is a gap in
   `00-foundation`, recorded in `CONTRACT.md` §3 under *What this module needs*.
3. **`[AC-U06]` is the isolation rule here.** This crate never widens tenant visibility: no
   `begin_platform`, no `pos_jobs` role, no second tenant in one flow. That is why `[D-2]`'s
   cross-tenant merge runs once per tenant under `12-jobs` (Q2) rather than as one flow.

---

## 5. Open questions

**None.** Q1–Q6 were answered on 2026-10-25 and Q7, raised the same day by module 05, was
answered with them. The module stays at Phase 1 because 05–08 are Phase 0 (§3, *Deferred*).

Two of the answers are **engineering decisions that Notion does not yet carry a number for** —
Q3 and Q6. They are recorded here and in [`../INDEX.md`](../INDEX.md) *Requirement drift*; no
number was minted in [`../DECISIONS.tsv`](../DECISIONS.tsv), because that file is a mirror of
Notion and a citation that resolves locally but not upstream is worse than none.

### Answered

**Q1 — 2026-10-25. A void is the sixth flow**, `void_sale.rs`. It spans 06, 07 and 08 and must
be atomic, so it belongs here rather than inside `07-sales`; reusing `refund.rs` was rejected
because `[D-42]` and `[D-16]` carry different preconditions — same-day-and-nothing-returned
against invoice-required-and-partial-allowed — and one flow holding both would check neither
clearly. **`[D-97]` names five files, so it is not exhaustive and needs amending in Notion.**

**Q2 — 2026-10-25. `[D-2]`'s merge move runs once per tenant, driven by `12-jobs`** — see
`[AC-E05]`. RLS stays on and every write goes through the owning module's command. The
consequence accepted with it: the move is atomic per tenant and **not** overall, so the job is
resumable and reports which tenants it moved. The single `BYPASSRLS` sweep was rejected for
writing tenant-owned rows with isolation off, and "move nothing, follow the pointer" for
contradicting `[D-2]`'s own text.

**Q3 — 2026-10-25. A module's `queries.rs` is generic over `C: ConnectionTrait`**, so one
function serves both a route's bare connection and a flow's transaction. `identity` already
does this in nine functions; `tenancy` and `catalog` do it in none, so **both are
non-conforming and each needs a backlog item** to bring its read surface in line — neither is
module 09's to file. What this answer costs is the property `[D-98]` provided for free, that
the signature tells you which side of the command/query split you are on; `[AC-U10]` replaces
it with an explicit rule and a textual assertion, because the failure it prevents is silent: a
locking read called on a bare connection compiles, takes no lock, and permits the oversell.
The alternatives were a `&Tx` read in `commands.rs` (rejected: a read in the command file) and
a third `locks.rs` file per module (rejected: a new convention in every module).
**`[D-98]` needs amending in Notion** to say a query takes a connection *or* a transaction.

**Q4 — 2026-10-25. `[D-24]`'s "sales only" is strict.** `10-sync` replays `sell` and nothing
else; a return, a collection and a void need a connection. This keeps the replay surface at
one flow and avoids an ordering rule between an unsynced sale and a refund against it — at the
cost, accepted knowingly, that a shop on a bad line cannot take a return until it reconnects.

**Q5 — 2026-10-25. Four cases are flagged, not four plus one** — see `[AC-S02]` and
`[AC-S03]`. Added to oversell `[D-25]` and over-limit credit `[D-46]`: a batch that **expired
between the sale and the upload** `[D-17]`, and a **unit price changed server-side** after the
phone quoted it. Rejected for flagging: a sale naming a **customer deleted** server-side,
because a credit sale with no customer has nowhere to post its ledger entry. **Only the first
two are named by a decision; the other two need amending into Notion.**

**Q6 — 2026-10-25. The route lives in `crates/server`, the wire types in `workflows`.** The
route, its `#[utoipa::path]`, the permission layers and the access-mode gate are composition,
which is what `crates/server` is for and why Gate 0 exempts it; the request and response
structs are defined in `workflows`, because no single module owns "a sale plus its payments
plus its ledger effect" and a duplicate in `server` would be a second definition of the wire
shape that drifts. Accepted cost: `crates/server` stops being pure wiring and holds six
endpoints. Rejected: DTOs in `server` too (`10-sync` would have to import the binary or
duplicate them), and the owning module keeping its route and calling the flow through a
`core::ports` trait — which does avoid the cycle, as `AccessModeSource` proves, but buys a
trait and dynamic dispatch per flow to move six routes. **This settles `[AC-W03]`'s
`&TenantContext`**: it survives, because `Tx` does not expose the branch ids `transfer` needs.
Consequence for the modules below: **no module's `CONTRACT.md` may claim one of the six paths
as its own**, and 07's may not declare `POST /v1/sales`.

**Q7 — 2026-10-25. `post_supplier_return` is the seventh flow.** Raised by
[`../05-procurement/SPEC.md`](../05-procurement/SPEC.md) Q1: posting a supplier return writes a
reversing stock movement (06) and a credit on the supplier ledger (05) in one transaction, so it
spans modules and must be atomic. Folding it into `post_purchase` behind a direction flag was
rejected — the two share almost nothing, with different tables, an opposite ledger sign,
different preconditions, and `[D-17]`'s expired-batch rule applying to the return alone. Leaving
`supplier_returns.status = 'posted'` unreachable was rejected as shipping a state nothing can
enter.

**`[D-97]` has now been amended twice in two days, for the same reason each time** — a flow that
provably spans modules was missing from a list of files. The amendment to ask for is therefore
not "seven files": it is that `[D-97]` **stop enumerating files and state the rule**, which
`[AC-W04]` already carries. Recorded in `../INDEX.md` *Requirement drift*.
