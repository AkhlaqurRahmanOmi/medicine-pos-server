# SPEC: 06 — Inventory

**Status:** Draft — Gate 1. **Does not advance to Gate 2:** five open questions (§5), each
scoped to one column, one route or one write. Nothing in the ledger's shape, the commands
05 and 09 are blocked on, or the transaction boundaries is open.
**Crate:** `crates/inventory`
**Bounded context:** Context 6 — Inventory
**Owner:** `backend-architect` role, drafted 2026-10-25

> **This module is the one the others wait on.** Data Model Context 6 opens with *"Designed
> before procurement and sales, because both of them write into this ledger."*
> [`../05-procurement/SPEC.md`](../05-procurement/SPEC.md) §3 names seven things it needs from
> here and [`../09-workflows/SPEC.md`](../09-workflows/SPEC.md) §3 names six;
> [`CONTRACT.md`](CONTRACT.md) §3 answers every one of them, and §3 of this file records where
> an answer differs from what was asked for.

---

## 1. Purpose

**Where the medicine physically is, and what it cost.** This module owns eleven tables, and
one of them — `stock_movements` — is the system's central append-only ledger: every purchase
received, every unit sold, every return, transfer, write-off and count difference in the
pharmacy is one row in it, inserted and never changed. `stock_balances` is the running total
that ledger implies, maintained by a trigger and unwritable by the application. Everything
else here hangs off those two: the batch a movement belongs to, the adjustment or transfer or
count that caused it, the reorder level that watches it, and the platform-reviewed request to
change a batch's MRP `[D-38]`.

Three properties shape everything below.

1. **The ledger is the record; the balance is a convenience.** A mistake is corrected by a new
   movement, never by editing one, and the balance can always be rebuilt with
   `SUM(quantity)`. This is also what lets an offline sale arrive as a new row hours later
   instead of fighting over a single number `[D-24]`.
2. **State is derived wherever a date can derive it.** Expired is `expiry_date < today` in
   Asia/Dhaka and near-expiry is within the pharmacy's window `[D-5]` — no nightly job flips a
   column, so no state can be stale.
3. **Money here is a *basis*, not a price.** A batch's cost is the money that was actually
   paid for the units that actually arrived. §3, *The cost basis*, is the longest part of this
   spec because getting it wrong is a 9% margin error that no test about stock levels would
   catch.

**Not this module's job:**

| The nearest wrong assumption | Who owns it |
|---|---|
| A sale, a sale line, an invoice number, the batch *choice* on a line `[D-4]` `[D-9]` | **07 Sales.** 06 validates and records the movement for a line someone else chose; it allocates nothing (§3, *There is no allocation command*) |
| A purchase, a supplier return, a supplier's bill or ledger | **05 Procurement.** `purchase_items.batch_id` / `.stock_movement_id` are FKs *into* here, stamped by `workflows::post_purchase` |
| **Posting** a purchase or a supplier return, selling, refunding, voiding | **09 Workflows**, in one caller-owned transaction. Those routes live in `crates/server` with their wire types in `workflows` (09 Q6), and [`CONTRACT.md`](CONTRACT.md) §2 **claims none of them** |
| A pharmacy's *selling* price, its VAT rate, a price-change history | **04 Pricing**, on `tenant_products`. Data Model Context 4 is explicit the other way too: *"Batch MRP and cost price belong to `batches` (context 6)"*, which is why `batch_mrp_change_requests` is here and not there `[D-38]` |
| Whether a product exists, whether it is approved, and **whether it requires a batch** `[D-8]` | **01 Catalogue**, on the shared `products` table. 06 enforces the rule; 01 owns the fact (§3, `[AC-W03]`) |
| The near-expiry window `[D-5]`, the zero-stock-sale setting, the branch's existence | **02 Tenancy**, on `tenant_settings` and `branches`. Both settings reach 06 as **parameters**, never as a read it performs itself ([`CONTRACT.md`](CONTRACT.md) §3, *What this module needs*) |
| The **oversell flag** `[D-25]` and the expired-at-upload flag `[D-17]` | **Cross-cutting `sync_flags`**, almost certainly `10-sync`'s (§5 Q1). **06 declares no flag column**; it returns the facts a flag is written from (§3, *Flags are facts here, rows elsewhere*) |
| A near-expiry or low-stock **alert**, and the sweep that sends it | **12 Jobs**, writing cross-cutting `notifications`. 06 owns the two reads the sweep runs and schedules nothing ([`CONTRACT.md`](CONTRACT.md) §4) |
| Stock valuation, movement and expiry **reports**, the VAT register | **11 Reporting**, reading these tables with `query_as!` `[D-99]`. 06's lists are screens, not reports |
| `audit_logs` — including the row an approved MRP change must write | **Cross-cutting, owner unassigned** (§5 Q3). It is `tenant_id`-nullable and covers every table, so it cannot be 06's |
| Deciding *what* `sync` replays, batching an upload, reconciliation screens | **10 Sync** `[D-24]` |

---

## 2. Decisions this module implements

Cite, never restate. The reasoning lives in Notion.

| Ref | Decision | Consequence here |
|---|---|---|
| `[D-4]` | The salesperson picks the batch; earliest expiry is pre-selected | 06 exposes `sellable_batches` ordered by `expiry_date` and **chooses nothing**. There is no FIFO allocator and no auto-pick `[AC-U17]` |
| `[D-9]` | Two batches on one invoice are two separate lines | So one sale line is one batch is **one movement**. 06 needs no "split across batches" command; what 09 §3 called "the allocation command" is `record_stock_out_many`, a plural write with one lock order, not an allocator (§3) |
| `[D-8]` | Batch and expiry are optional for non-medicines | One **untracked batch** per `(branch, product)` with `batch_number`, `expiry_date` and `unit_mrp` all null, so the ledger is identical for every product `[AC-U13]`. A product that is not `non_medicine` **must** have a tracked batch, enforced in Rust against 01's fact `[AC-W03]` |
| `[D-5]` | The near-expiry window is a tenant setting, default 90 days | Near-expiry is a **query parameter**, not a column and not a job-set flag. The window arrives from the caller; 06 reads no setting `[AC-U07]` |
| `[D-10]` | A new batch with a higher MRP prompts and never updates automatically | `find_or_create_batch` returns an existing batch **unchanged**, MRP included, and the client prompts from `batch_lookup` `[AC-E01]`. This is also 05's `[AC-U16]` |
| `[D-38]` | Changing a batch's MRP after purchase is super-admin only, on the pharmacy's request | `batches.unit_mrp` is **unreachable to every pharmacy role by column-level grant** `[AC-U05]` `[AC-W02]`. The only path is a `pending` request a platform super admin approves `[AC-E13]` |
| `[D-17]` | A return into an expired batch is accepted and marked non-sellable | Accepted as a `sale_return_in` movement, and **nothing is written to mark it** — the batch is already outside every sellable read because `expiry_date < today` `[AC-E07]` `[AC-S01]`. §3, *What "non-sellable" is* |
| `[D-25]` | Offline oversell: accept and flag | `stock_balances.quantity` may go negative and **no check constraint forbids it** `[AC-U04]`. The stock-out command's short-stock behaviour is a caller-supplied policy, and the result carries the shortfall `[AC-E05]` |
| `[D-36]` | A branch transfer is instant, with no in-transit state | One transaction: `transfer_out` at the source, create-or-find the target batch, `transfer_in` at the target `[AC-E11]`. No in-transit row, no second step, nothing to reconcile. **Every table it writes is 06's**, which is why §5 Q4 asks whether `workflows::transfer` should exist at all |
| `[D-24]` | Offline scope is sales only | Only `sale_out` and (through a replayed sale) nothing else arrives from a phone. Counts, adjustments and transfers are online `[AC-S07]` |
| `[D-99]` | sqlx `query_as!` for reports and non-trivial queries | The low-stock, expiry and valuation-shaped reads are `query_as!`; single-row batch and count reads are SeaORM |
| `[D-97]` | Cross-module flows live in `workflows`; modules never call each other | 06 depends on no module crate except **`catalog`**, for the one integrity fact it must enforce rather than trust (`[AC-W03]`), exactly as 05 does. Nothing depends on 06 but `server`, `workflows` and `reporting` |
| `[D-98]` | `commands.rs` takes a transaction, `queries.rs` takes a connection | **As amended by 09 Q3: every function in `queries.rs` is generic over `C: ConnectionTrait`**, and every `FOR UPDATE` read sits in such a function (09 `[AC-U10]`) `[AC-U08]` `[AC-U12]` |
| `[D-85]` | Stable error codes plus English | Eight 409 codes, two of them already reserved in `00-foundation` ([`CONTRACT.md`](CONTRACT.md) §2) |
| `[D-20]` | A lapsed subscription is read-only | Every write is behind the access-mode gate; **every read still succeeds** `[AC-W14]` — a pharmacy that cannot pay must still be able to see what is expiring on its shelf |
| `[D-65]` | Who may do a thing is a permission, not a fixed role | Fourteen branch-scoped slugs, module `inventory`. **No slug for a movement** (§3, *A movement is never authorised on its own*) |

---

## 3. Behaviour

EARS acceptance criteria, each one a test name at Gate 2.

- **Ubiquitous (always true)**
  - `[AC-U01]` All eleven tables SHALL carry `tenant_id`, SHALL have `ENABLE` **and**
    `FORCE ROW LEVEL SECURITY` with a tenant policy, SHALL expose a `(tenant_id, id)` unique
    key (or a tenant-leading composite primary key where the data model gives a composite), and
    SHALL reference every other tenant-owned row by a composite `(tenant_id, …)` foreign key
    ([`../rules/tenancy.md`](../rules/tenancy.md) rules 1–6).
  - `[AC-U02]` `stock_movements` SHALL be append-only **by grant**: no role SHALL hold `UPDATE`
    or `DELETE` on it, and no code path SHALL exist that attempts either (rule 7). A wrong
    movement SHALL be corrected by a **new** movement — a `stock_adjustment` with reason
    `correction` — carrying a note and the acting user, never by changing the row that was
    wrong.
  - `[AC-U03]` `stock_balances` SHALL change **only** as a consequence of inserting a row into
    `stock_movements`: `pos_app` SHALL hold `SELECT` only on it, and every column of it SHALL
    be maintained by a `SECURITY DEFINER` trigger on `stock_movements` that runs in the **same
    transaction** as the insert. For every batch, `quantity` SHALL equal
    `SUM(stock_movements.quantity)`, and the cost basis SHALL equal the same sum restricted to
    the acquiring movement types (*The cost basis*, below).
  - `[AC-U04]` `stock_balances.quantity` SHALL be allowed to go **negative**, and no check
    constraint SHALL forbid it. A negative balance is a real, recorded oversell `[D-25]`, and
    the repair is an adjustment a human makes, never an automatic correction `[AC-S03]`.
  - `[AC-U05]` No pharmacy role SHALL be able to change `batches.unit_mrp`, `batch_number`,
    `expiry_date`, `product_id` or `branch_id` — by grant, not by convention. `pos_app`'s
    `UPDATE` on `batches` SHALL be **column-level**, naming `status`, `quarantine_reason` and
    `updated_at` and nothing else `[D-38]` `[AC-W02]`.
  - `[AC-U06]` **No money column in this module SHALL hold a per-unit cost.** The cost basis
    SHALL be an exact `(total, unit count)` pair and the per-unit average SHALL be derived at
    read time (*The cost basis*, below) `[AC-U09]`.
  - `[AC-U07]` **Expired SHALL never be stored.** No column, trigger or job SHALL record that a
    batch has expired; every read SHALL derive it as `expiry_date < today` in Asia/Dhaka, and
    near-expiry from a window the caller supplies `[D-5]`.
  - `[AC-U08]` Every read whose result decides whether a write is allowed SHALL take the row
    locks it needs in **one order shared by every caller** — ascending `(tenant_id, batch_id)`
    — and SHALL sit in a function generic over `C: ConnectionTrait` (09 `[AC-U10]`,
    `[AC-U08]`). The lock SHALL be taken on the **`batches`** row, not on `stock_balances`
    (*Why the lock is on the batch*, below) `[AC-W12]`.
  - `[AC-U09]` Every money value SHALL be `rust_decimal::Decimal` in Rust and `numeric(12,2)`
    in the database, at every layer including serialization (constitution Art. IV.5). Every
    quantity SHALL be a signed integer in **base units**.
  - `[AC-U10]` Every `stock_movements` row's `branch_id` SHALL equal its batch's `branch_id`,
    enforced by the composite foreign key to `batches (tenant_id, branch_id, id)` and not by
    application code.
  - `[AC-U11]` This module SHALL write no table it does not own: no sale, no purchase, no
    `sync_flags` row, no `notifications` row, no `audit_logs` row, no `tenant_products` row.
  - `[AC-U12]` Every function in `queries.rs` SHALL be generic over `C: ConnectionTrait`
    (09 Q3), and **no `FOR UPDATE` query SHALL sit in a function that is not** (09 `[AC-U10]`).
  - `[AC-U13]` A product without batch tracking SHALL use exactly **one** batch per
    `(tenant_id, branch_id, product_id)`, with `batch_number`, `expiry_date` and `unit_mrp` all
    null, held by a unique index with `NULLS NOT DISTINCT` `[D-8]`.
  - `[AC-U14]` Branch scoping SHALL be an authorisation and query concern, **not** row-level
    security: no policy in this module SHALL read a branch list (00 Q2, closed as void). A read
    SHALL filter to the branches the caller holds the slug at and a write SHALL check before the
    command runs `[AC-W08]`.
  - `[AC-U15]` A batch's identifying facts — `branch_id`, `product_id`, `batch_number`,
    `expiry_date` — SHALL be immutable for the life of the row. A batch entered wrongly is
    emptied by an adjustment and a new batch is created; it is never edited into a different
    batch.
  - `[AC-U16]` Every read that offers stock for sale SHALL exclude a batch that is expired, that
    is `quarantined`, or whose balance is not positive, and SHALL order by `expiry_date`
    ascending `[D-4]`.
  - `[AC-U17]` This module SHALL declare **no permission slug for a stock movement**. A movement
    is authorised by the business action that causes it — `purchase.post`, a sale, an
    adjustment, a transfer — never on its own (*A movement is never authorised on its own*).

- **Event-driven (when)**
  - `[AC-E01]` WHEN `find_or_create_batch` is called and a batch with that
    `(branch, product, batch_number)` exists, the system SHALL return it **unchanged** — MRP,
    expiry and status included — and SHALL NOT update any column of it `[D-10]` `[D-38]`,
    which is also 05 `[AC-U16]`.
  - `[AC-E02]` WHEN no such batch exists, the system SHALL create it and SHALL create its
    `stock_balances` row at zero in the same transaction, so that every batch has exactly one
    balance row from the moment it exists.
  - `[AC-E03]` WHEN stock is received, the system SHALL insert **exactly one** movement per
    received line, of type `purchase_in` or `transfer_in`, carrying the **exact** line cost
    total and unit count it was given, and the batch's cost basis SHALL move by exactly those
    two figures in the same transaction `[AC-U06]`.
  - `[AC-E04]` WHEN stock goes out, the caller's short-stock policy is `Reject`, and the locked
    balance is less than the units requested, the system SHALL answer
    `409 insufficient_stock` and SHALL write nothing.
  - `[AC-E05]` WHEN stock goes out and the policy is `Accept`, the system SHALL write the
    movement, SHALL allow the balance to go negative, and SHALL return the resulting balance
    and the shortfall so the caller can raise its flag `[D-25]`.
  - `[AC-E06]` WHEN a `sale_out` movement names a batch that is expired at `occurred_at`, or
    whose status is `quarantined`, the system SHALL answer `409 batch_expired` or
    `409 batch_quarantined` and SHALL write nothing — unless the policy is `Accept`, which is
    how a replayed offline sale is admitted `[AC-S08]`.
  - `[AC-E07]` WHEN a `sale_return_in` movement names an **expired** batch, the system SHALL
    accept it, SHALL write no status change, and SHALL report that the batch was expired so the
    caller can flag it `[D-17]` (*What "non-sellable" is*).
  - `[AC-E08]` WHEN a stock adjustment is created, the system SHALL write the header, its items
    and **one `adjustment` movement per item** in one transaction, with each movement's sign
    equal to its item's `quantity`, and SHALL stamp each item with its movement's id.
  - `[AC-E09]` WHEN a stock count is completed, the system SHALL create **one** adjustment with
    reason `stock_count` and `stock_count_id` set, holding one item for each batch whose
    `counted_quantity` differs from its `system_quantity`, and SHALL set the count's `status`,
    `completed_by_user_id` and `completed_at` in the same transaction. WHEN no quantity
    differs, it SHALL create **no adjustment** and SHALL still complete the count.
  - `[AC-E10]` WHEN a batch is first recorded on an open count, the system SHALL freeze
    `system_quantity` from that batch's balance at that moment and SHALL never update it, so
    the count's result does not move while selling continues.
  - `[AC-E11]` WHEN a transfer is created, the system SHALL in **one** transaction write a
    `transfer_out` at the source, find or create the target batch with the **same** product,
    batch number, expiry and MRP, and write a `transfer_in` at the target whose cost total is
    equal and opposite to the out movement's `[D-36]`. There SHALL be no in-transit state and
    no second request.
  - `[AC-E12]` WHEN an MRP change request is created, the system SHALL store it `pending` with
    `current_unit_mrp` copied from the batch, and a second pending request for that batch SHALL
    be refused by a partial unique index with `409 batch_mrp_request_pending`.
  - `[AC-E13]` WHEN a platform super admin approves a request `[D-38]`, the system SHALL set
    `batches.unit_mrp` to `requested_unit_mrp`, set the request's `status`,
    `reviewed_by_user_id`, `reviewed_at` and `review_note`, and write one `audit_logs` row — all
    in one platform transaction. **The audit row is blocked on §5 Q3**; the rest is not.
  - `[AC-E14]` WHEN a request is rejected, the system SHALL leave the batch **entirely
    unchanged** and SHALL record the reviewer, the time and the note.
  - `[AC-E15]` WHEN a reorder level is set, the system SHALL upsert one row per
    `(branch, product)`; WHEN it is cleared, the system SHALL delete the row, because **no row
    means no alert**.
  - `[AC-E16]` WHEN a batch is quarantined or released, the system SHALL change `status` and
    `quarantine_reason` only, and SHALL write **no movement** — quarantine moves no stock and
    the quantity on hand does not change.
  - `[AC-E17]` WHEN any movement is inserted, `recorded_at` SHALL be set by the **database**
    and SHALL never be accepted from a caller, while `occurred_at` SHALL be the business moment
    the caller supplies — the device clock for an offline sale `[D-24]`.

- **State-driven (while)**
  - `[AC-S01]` WHILE a batch's `expiry_date` is before today in Asia/Dhaka, it SHALL be absent
    from every sellable read `[AC-U16]`, SHALL still accept a `sale_return_in` `[D-17]`, SHALL
    still be counted in stock on hand and in valuation, and SHALL carry **no stored marker**
    `[AC-U07]`.
  - `[AC-S02]` WHILE a batch's status is `quarantined`, it SHALL be absent from every sellable
    read, SHALL still be counted in stock on hand and valuation, and SHALL remain adjustable
    and transferable — quarantine stops selling, not accounting.
  - `[AC-S03]` WHILE a batch's balance is negative, every read SHALL show it as it is, no
    automatic correction SHALL run, and the repair SHALL be a stock adjustment a human records
    `[AC-U04]`.
  - `[AC-S04]` WHILE a stock count is `open`, its items SHALL be replaceable in full and it
    SHALL be completable or cancellable. WHILE it is `completed` or `cancelled`, the system
    SHALL answer `409 stock_count_not_open` and the **database** SHALL refuse a write to its
    items by trigger.
  - `[AC-S05]` WHILE a batch is untracked `[D-8]`, it SHALL have no MRP, no expiry and no MRP
    change request — a request against it SHALL be refused `409 batch_untracked`.
  - `[AC-S06]` WHILE an MRP request is not `pending`, no review SHALL be accepted
    (`409 mrp_request_not_pending`), and the request SHALL remain readable with its historical
    `current_unit_mrp` intact.
  - `[AC-S07]` WHILE a stock count, an adjustment or a transfer is being created, the system
    SHALL require a connection: none of the three is creatable offline `[D-24]`.
  - `[AC-S08]` WHILE a movement originates from a replayed offline sale, a precondition the
    phone could not check — short stock `[D-25]`, a batch that expired between the sale and the
    upload `[D-17]` — SHALL be **recorded and reported**, never rejected (09 `[AC-S02]`). The
    recording of it is the caller's; 06 supplies the facts (*Flags are facts here*).

- **Unwanted (if / then)**
  - `[AC-W01]` IF an `UPDATE` or `DELETE` is attempted against `stock_movements`, or any
    `INSERT`, `UPDATE` or `DELETE` against `stock_balances`, THEN the **database** SHALL refuse
    it for lack of a grant, regardless of what any code path intends `[AC-U02]` `[AC-U03]`.
  - `[AC-W02]` IF any role other than the migration owner, or `pos_app` with
    `app.is_platform` set, attempts to update `batches.unit_mrp`, THEN the **database** SHALL
    refuse it for lack of a column grant `[D-38]`.
  - `[AC-W03]` IF a batch is created for a product that is not `non_medicine` without a batch
    number or without an expiry date, THEN the system SHALL answer `400 validation_failed`
    naming the field `[D-8]`. The fact comes from **01's** `products.product_type` through a
    named read, because a trigger joining across a bounded context is the coupling `[D-97]`
    removes.
  - `[AC-W04]` IF a transfer names the same branch as source and target, THEN the system SHALL
    answer `400 validation_failed` **and** the database SHALL refuse it by check constraint.
  - `[AC-W05]` IF a transfer's target batch does not match the source's product, batch number
    and expiry, THEN the **database** SHALL refuse it by trigger — the command creates the
    target itself, so this is a backstop against a future caller, not a user-facing path.
  - `[AC-W06]` IF an adjustment or a transfer is submitted with no items, THEN the system SHALL
    answer `409 adjustment_empty` / `409 transfer_empty`.
  - `[AC-W07]` IF a movement quantity is zero, or its sign disagrees with its type, THEN the
    **database** SHALL refuse it by check constraint and the system SHALL answer
    `400 validation_failed`.
  - `[AC-W08]` IF a caller reads or writes stock at a branch it holds no permission at, THEN
    the system SHALL answer `403 branch_not_allowed`; IF the row belongs to another
    **pharmacy**, THEN it SHALL answer `404 not_found`
    ([`../00-foundation/CONTRACT.md`](../00-foundation/CONTRACT.md) §2).
  - `[AC-W09]` IF a transfer is attempted and the caller lacks the slug at **either** branch,
    THEN the system SHALL answer `403 branch_not_allowed`. A single-branch check is not enough,
    and `api::require_at` resolves one branch from the path — the gap 09 `CONTRACT.md` §3
    already recorded.
  - `[AC-W10]` IF an MRP change request asks for a value that is not greater than zero, or that
    equals the batch's current MRP, THEN the system SHALL answer `400 validation_failed`.
  - `[AC-W11]` IF a caller supplies a per-unit cost, THEN there SHALL be no parameter to carry
    it: the cost basis type accepts a line total and a unit count only `[AC-U06]`. This is
    enforced by the type, not by validation.
  - `[AC-W12]` IF `SELECT … FOR UPDATE` is attempted against `stock_balances` as `pos_app`,
    THEN **PostgreSQL SHALL refuse it**, because a row lock requires `UPDATE` privilege on at
    least one column and `pos_app` holds `SELECT` only `[AC-U03]`. This is why the zero-stock
    lock is taken on `batches` (*Why the lock is on the batch*).
  - `[AC-W13]` IF the same movement id is inserted twice, THEN the primary key SHALL refuse the
    second. This is a **backstop, not the idempotency mechanism**: a replayed upload is
    refused by the root sale's primary key before any movement is attempted
    (09 `[AC-E02]`) (*Offline behaviour*).
  - `[AC-W14]` IF a write is attempted while the pharmacy's access mode is read-only, THEN the
    route SHALL answer `403 subscription_read_only` before any command runs `[D-20]`, AND every
    read in this module SHALL still succeed.
  - `[AC-W15]` IF any step of a multi-row write fails — an adjustment's third item, a
    transfer's in-movement — THEN the caller's transaction SHALL roll back whole and the client
    SHALL be able to retry the identical payload. There is no partial adjustment and no
    half-transfer to repair.

### The cost basis — the judgement 05 asked for, and what it costs

[`../05-procurement/SPEC.md`](../05-procurement/SPEC.md) §3 asked this module for *"the shape
of the cost basis … and the scale of the column it is stored in. If 06 chooses a per-unit
cost, it owns the rounding residue and must say where it goes."* Three statements in the
requirement cannot all be true at once:

| Where | What it says |
|---|---|
| Data Model Context 6, `batches` | `unit_cost` is a *"average cost per base unit"*, and the BCNF note calls it *"a running average, rebuildable from the incoming movements' costs"* |
| *Rules that hold across every context* | *"Money is `numeric(12,2)`"* |
| Data Model Context 5, BCNF | the *"effective cost per unit with free goods"* is in the **deliberately not stored** list |

A `10+1 @ 8.50` line settles it arithmetically: eleven units cost ৳85.00, and
`85.00 ÷ 11 = 7.7272…`. At `numeric(12,2)` that is `7.73`, and `7.73 × 11 = 85.03` — the batch
is valued three paisa above what was owed, every margin taken from it is wrong, and the error
is per batch, silent, and in the pharmacy's favour on paper. **No decimal scale fixes it**,
because one eleventh is not a terminating decimal: at six places the residue is three
*millionths* of a taka rather than zero, and the column would break the money rule to get
there.

**Decision: the rounding is avoided, not absorbed and not carried. The basis is stored as the
exact rational it is — a money total and a unit count — and the per-unit average is derived.**

| | Before (the ERD's columns) | After (this contract) |
|---|---|---|
| On the movement | `stock_movements.unit_cost` *"cost of these units"* | **`cost_total numeric(12,2)`**, signed with `quantity`. The ERD's own comment already reads as a total; the name does not |
| On the batch | `batches.unit_cost`, a stored running average | **gone.** `batches` holds no money but `unit_mrp` |
| The running total | `stock_balances.quantity` | `stock_balances` gains **`cost_basis_total numeric(12,2)`** and **`cost_basis_units integer`** |
| Average cost per unit | a stored, rounded column | **derived**: `cost_basis_total / cost_basis_units`, in `Decimal`, rounded **once, at the edge**, for display only |

Four consequences, each of which is the reason:

1. **Valuation reconciles to the supplier ledger to the paisa.** A batch nothing has left is
   valued at exactly the money that was owed for it. Valuation of a part-sold batch is
   `balance × (total ÷ units)`, multiplied before rounding, so the error is one rounding at
   the end rather than one per batch compounded by every sale.
2. **The basis counts only movements that bring money-bearing stock in** — `purchase_in` and
   `transfer_in`. A sale out, a sale return in, a purchase return out and an adjustment leave
   the basis alone, so the average cost of what remains is never disturbed by selling,
   returning or finding stock. Found stock is therefore valued at the batch's average, which is
   the only honest answer for units whose cost nobody knows.
3. **Every other movement still carries money**, as the ERD intends: `cost_total` for a
   `sale_out`, `adjustment` or `purchase_return_out` is the batch's average times the quantity,
   **rounded to two places and frozen at the movement**. That is a historical copy, and it must
   be frozen for exactly 05's reason — re-deriving it later would make the margin on an old sale
   depend on a purchase made afterwards. A report's cost of goods sold is therefore the sum of
   these frozen figures and may differ from the basis by a few paisa across thousands of lines;
   that difference is visible, bounded and in the report, not in the stored basis.
   **`SUM(cost_total)` is not the valuation** — valuation is `balance × average`, and a report
   that adds the ledger's money column instead will be wrong by the rounding.
4. **A transfer conserves money exactly** `[D-36]`: the in movement's `cost_total` is the out
   movement's, equal and opposite, so the two branches' valuations sum to the one the source
   had. The *averages* then differ in the third decimal place, which is the only place the
   rounding lands and it is stated rather than hidden.

**What it costs.** Three deviations from the ERD's column list, all of them Notion amendments
and none of them a behaviour change (§5, *Three places the Notion page needs amending*). A
client that expected `batches.unit_cost` still receives a `unit_cost` field — derived, two
decimal places, display-only — so no wire shape changes. And `11-reporting` must be told once,
in writing, that the stock-value query multiplies before it divides.

**What is settled either way:** 05 never emits a rounded per-unit cost (05 `[AC-U08]`) and 06
has no parameter that could accept one `[AC-W11]`. The handoff type is
`CostBasis::Exact { line_cost_total, units }` ([`CONTRACT.md`](CONTRACT.md) §3), which is
precisely the shape 05 proposed.

### The ledger, and what a correction means

`stock_movements` is append-only by grant ([`../rules/tenancy.md`](../rules/tenancy.md) rule 7)
and `stock_balances` is the running total a trigger maintains from it.
[`../05-procurement/SPEC.md`](../05-procurement/SPEC.md) settled the same question for the
supplier ledger, and **this module follows that shape exactly** — deliberately, because two
ledgers in one system with two different correction stories is how an incident becomes
unarguable:

1. **A wrong movement is reversed, never updated.** There is no edit route, no `UPDATE` grant
   and no `DELETE` grant — not for `pos_app`, not for `pos_jobs`, not for anyone but the
   migration's owner role. A miscounted write-off is corrected by a second adjustment with
   reason `correction`, carrying a note and the acting user, and both movements stay. The
   ledger is the history of what was believed, not a view of what is currently true
   `[AC-U02]`.
2. **The trigger is this module's**, because this module owns both tables. It is
   `AFTER INSERT … FOR EACH ROW` on `stock_movements`, upserting `stock_balances` by
   `quantity = quantity + NEW.quantity` — and the two basis columns for an acquiring type — in
   the **same transaction**, and it is `SECURITY DEFINER` with a pinned `search_path` so that
   `pos_app` can be given `SELECT`-only rights on `stock_balances` `[AC-U03]`. The balance is
   therefore unreachable except through a movement insert: **there is no code path, correct or
   buggy, that can set a figure the ledger does not say.** A second definer trigger on
   `batches` creates the zero row `[AC-E02]`.
3. **It is rebuildable, and that is the test.** `quantity` must equal
   `SUM(quantity) GROUP BY batch_id` at all times, and the basis the same sum restricted to the
   acquiring types; the rebuild statement is the Gate-5 assertion and the reconciliation a
   human runs after an incident. Nothing recomputes a balance in Rust — a module that both
   maintains and recomputes a running total has two answers and no way to tell which is wrong.

**One difference from 05, forced by PostgreSQL.** See below: the balance row cannot be locked,
so the lock is on the batch.

### Why the lock is on the batch, not on the balance

Data Model Context 6 says *"Zero-stock blocking locks the balance row and checks it"*. It
cannot: **PostgreSQL requires `UPDATE` privilege on at least one column for
`SELECT … FOR UPDATE`**, and `pos_app` holds `SELECT` only on `stock_balances` by `[AC-U03]`.
The lock attempt fails with a privilege error, not with a missing lock, so it would be caught —
but it would be caught at runtime, by whoever shipped the sale path.

So the lock is taken on the **`batches`** row, where `pos_app` holds a column-level `UPDATE`
`[AC-U05]`, and the balance is read under it `[AC-U08]`. The effect is the same and slightly
stronger: concurrent sellers of one batch serialise on the batch row, which is also the row a
quarantine would change. The order is ascending `(tenant_id, batch_id)` for every caller, which
is what discharges 09 `[AC-U08]`'s deadlock rule for 06's half — a sale, a transfer, a void
and a return posting contending for the same two batches take them in the same order.

Granting `pos_app` `UPDATE (quantity)` on `stock_balances` to make the documented lock legal was
rejected: it is exactly the grant `[AC-U03]` exists to withhold.

### What "non-sellable" is — `[D-17]`, `batches.status` and 01's `is_sellable`

Three different questions get confused into one, so each gets a name:

| Question | Where it lives | Who asks it |
|---|---|---|
| May this **product** be stocked and sold at all? | `01-catalog`, `is_sellable(product)` — approved, not rejected or merged | 06 when creating a batch; 07 on a sale line |
| Has this **batch** been taken out of selling by a person? | `batches.status = 'quarantined'` with a `quarantine_reason` — a recall, water damage, a dispute | 06's quarantine route `[AC-E16]` |
| Has this batch **expired**? | **Nowhere.** `expiry_date < today`, derived on every read `[AC-U07]` | every sellable read `[AC-U16]` |

**`[D-17]`'s "marked non-sellable" is therefore not a write.** A return into an expired batch is
accepted as a `sale_return_in` and the stock sits in a batch that no sellable read will offer,
because the date says so. Setting `status = 'quarantined'` on the return was rejected for three
reasons: it is redundant while the batch is expired, it would **outlive** the condition if an
expiry date were ever corrected, and nothing in the requirement says who may un-quarantine it.
Data Model Context 6's own state table agrees — *"Quarantined: `status = quarantined`, **or**
expired"* — which is a derivation, not an assignment.

What the pharmacy sees instead is a read: expired stock with a balance still on hand is exactly
`GET /stock/expiring?state=expired`, which is the "supplier-return pile" the Product
Specification §9 describes.

### Flags are facts here, rows elsewhere

09 `SPEC.md` §3 asked 06 for *"the flag columns `[AC-S02]` now needs: oversell `[D-25]` and
expired-at-upload `[D-17]`"*. **There are none, and there should not be**: the requirement puts
the flag in a cross-cutting table — `sync_flags`, with `type ∈ oversell |
credit_limit_exceeded`, a `batch_id`, a `shortfall_quantity` and a `stock_adjustment_id` for
the adjustment that resolves it. It is a sale-shaped row that outlives the movement and is
cleared on a reconciliation screen, and 06 may not write a table it does not own `[AC-U11]`.

What 06 provides instead, and what the flag is written from:

| Fact | Where it comes from |
|---|---|
| `resulting_balance`, and `shortfall` when it went negative | `record_stock_out`'s result `[AC-E05]` |
| `batch_expired_at_occurrence` | the same result and `record_stock_in`'s, compared against `occurred_at` `[AC-E07]` |
| the batch and branch the flag names | the movement's own columns |

Two things follow that are not 06's to settle: **who owns `sync_flags`** and **whether its
`type` enum gains a value for `[D-17]`** — §5 Q1, which is 10-sync's question and already half
flagged by 09 Q5. Nothing in this contract waits on the answer.

### There is no allocation command

09 `SPEC.md` §3 asked for *"the allocation command `[D-9]`"*. **It does not exist, because the
requirement gives the decision to a person.** `[D-4]` makes the salesperson pick the batch with
the earliest expiry pre-selected, and `[D-9]` makes each chosen batch its own sale line. An
allocator that split a quantity across batches would be deciding what `[D-4]` says a human
decides, and it would produce one line where `[D-9]` wants two.

What stands in its place is two things:

- `queries::sellable_batches` — active, unexpired, positive balance, **ordered by expiry**
  `[AC-U16]`. That ordering *is* the pre-selection `[D-4]`.
- `commands::record_stock_out_many` — the plural write, which takes the whole set of lines and
  locks their batches in one order `[AC-U08]`. It is a lock-ordering device, not an allocator:
  it never chooses a batch and never moves a unit the caller did not name.

### A movement is never authorised on its own

05 `CONTRACT.md` §2 ends its posting note with *"the permission slugs that route must require:
`purchase.post` **and** 06's stock-in slug."* **There is no such slug**, and that is the answer
rather than an omission: nobody can post a purchase without stock arriving, so a second slug
would deny an action the first already authorised, or — worse — be granted to the manager
template by default and mean nothing. `[AC-U17]` states it as an invariant. Stock moves under
`purchase.post`, under 07's sale slug, under `stock_adjustment.create` and under
`branch_transfer.create`, and under nothing else.

### A transfer is one of this module's commands

`[D-36]` makes a transfer one transaction, and **every table it writes is 06's** —
`branch_transfers`, `branch_transfer_items`, `batches`, `stock_movements`, `stock_balances`. By
09's own one-line test — *"if the flow writes to tables owned by one module, it is not a
workflow"* — it is `commands::transfer_stock`, not a flow. What `workflows::transfer` would add
is nothing: it computes no total, touches no second module's table, and the only cross-cutting
concern in sight is the **two-branch permission check**, which 09 `[AC-U05]` puts outside the
flow anyway and 09 `CONTRACT.md` §3 already filed as an `api` gap `[AC-W09]`.

So the command is here and is listed in [`CONTRACT.md`](CONTRACT.md) §3 either way. Where the
**route** lives is §5 Q4, and until it is answered `CONTRACT.md` §2 claims the transfer reads
and **not** `POST /branch-transfers`, because 09 Q6 forbids a module to claim a path the
`workflows` list still carries.

### Offline behaviour

**One movement type arrives from a phone, and only inside a replayed sale.** `[D-24]` makes the
offline scope sales only, so `sale_out` (and a sale's own `sale_return_in`, which is online by
`[D-16]`) is the whole of it; counts, adjustments and transfers need a connection `[AC-S07]`.

What that means for idempotency here is narrower than the ERD's comment suggests.
`stock_movements.id` is *"created on the phone for offline sales"*, and 06 accepts a
caller-supplied UUIDv7 movement id for exactly that reason — it keeps the row traceable to the
device's own record. **But the movement id is not what makes an upload idempotent.** 09
`[AC-E02]` makes the *root* row the key: the replay is refused by the sale's primary key, inside
the flow's own transaction, before any movement is attempted, and child rows may take
server-generated ids. The movement's primary key is a second line of defence `[AC-W13]`, not the
mechanism — and it must not be relied on as one, because a client that regenerated a movement id
on retry would pass it while the sale's key still correctly refuses.

Two clocks, and they are not interchangeable: `occurred_at` is the business moment, the device's
clock, Asia/Dhaka — a sale made at 18:40 and uploaded at 22:10 happened at 18:40. `recorded_at`
is the server's, set by the database `[AC-E17]`. A running total is order-independent, so a late
upload needs no reordering and the balance is correct whatever order the day's movements arrive
in; `occurred_at` is what the ledger view and every report order by, with `id` (UUIDv7) as the
tiebreaker.

### Deferred — nothing

Every module this one needs is at Gate 1 or beyond: `00`, `01`, `02`, `03` are implemented and
`05` and `09` are at Gate 1 with their halves of both flows specified. The two reads 06 wants
from elsewhere — 01's batch-tracked fact and 02's two settings — are **gaps in an existing
surface**, not unwritten contexts, and both are recorded in
[`CONTRACT.md`](CONTRACT.md) §3, *What this module needs*. The five open questions in §5 are
each scoped to one column, one route or one write.

---

## 4. Tenancy and permissions

| Question | Answer |
|---|---|
| Tenant-owned? | **Yes, all eleven tables.** There is no shared and no platform-owned table here. `batch_mrp_change_requests` is **reviewed** by platform staff but **owned** by the pharmacy — it carries `tenant_id`, lives under the tenant policy, and gains a second policy for `app.is_platform`, the shape `02-tenancy` uses for the billing tables |
| Branch-scoped? | **Every table, and more strictly than anywhere else in the system.** Stock is physically at a branch: `batches`, `stock_movements`, `stock_balances`, `reorder_levels`, `stock_adjustments`, `stock_counts` and both transfer tables all carry a `branch_id`, and the child tables reach one through their parent. **A transfer is visible if *either* of its branches is** |
| Permissions required | Fifteen slugs, module `inventory`, listed in [`CONTRACT.md`](CONTRACT.md) §2. **All fifteen are branch-scoped**, and the cashier template gets exactly one — `stock.view` |
| Visible to a cashier? | `stock.view` and nothing else. It is the only module where a cashier needs a read: the sale screen must list the batches, their expiry and their quantity on hand `[D-4]`, and the phone downloads its own branch's batches and balances. **No cost figure is in a cashier-reachable response** — the derived `unit_cost` and the cost basis are behind `stock.cost.view`, so what the pharmacy paid is not on the counter phone |

Five things **specific** to this module:

1. **Branch scoping here is not row-level security, and the requirement says it is.** Data
   Model Context 6's *Visibility* section says every table filters on *"`branch_id` being in
   `app.branch_ids`"*. **No such setting exists**: module 00's Q2 was closed as void on
   2026-10-24 — `Tx` sets `app.tenant_id`, `app.user_id` and `app.is_platform` only, and no
   policy reads a branch list. So the branch predicate is a `WHERE` clause in the query and a
   check in the command, and the test for it is an HTTP test rather than an RLS test
   `[AC-U14]` `[AC-W08]`. This is the same resolution 05 `SPEC.md` §4.1 took, and it is the
   largest single gap between this contract and the Notion page — recorded in §5,
   *Three places the Notion page needs amending*, as a documentation fix rather than a design
   question, because the architecture is already decided and implemented.
2. **This module can use `api::require_at`, and 05 could not.** Every write names exactly one
   branch, so it takes it as a **path segment** — `POST /branches/{branch_id}/stock-adjustments`
   — and `require_at` resolves and checks it. Lists take `branch_id` as a filter and use
   `require(slug)` plus handler filtering against `AuthUser.branch_permissions`. One rule,
   stated once: **a write names its branch in the path; a read names it in the query.**
3. **A transfer needs the slug at two branches** `[AC-W09]`, which `require_at` cannot express.
   This is the gap 09 `CONTRACT.md` §3 recorded for `workflows::transfer`; it is the same gap
   whichever crate ends up holding the route (§5 Q4).
4. **`batches.unit_mrp` is protected by a grant, not by a route.** There is no endpoint that
   updates it, and if one were written tomorrow the database would still refuse it: the
   `UPDATE` grant on `batches` names three columns and `unit_mrp` is not among them
   `[AC-U05]` `[AC-W02]`. Data Model Context 6 states the requirement as *"no pharmacy role can
   update `batches.unit_mrp` at all"*, and a column-level grant is the only mechanism that makes
   that true of code nobody has written yet.
5. **The MRP review queue is the one place a read deliberately crosses tenants.** A platform
   super admin lists pending requests from every pharmacy `[D-38]`, under `Tx::begin_platform`
   and the `app.is_platform` policy, through `/api/v1/admin` with a platform token — never
   through a tenant route, and never with a tenant slug. Its index is the one in this module
   that does **not** lead with `tenant_id`, for exactly that reason
   ([`CONTRACT.md`](CONTRACT.md) §1).

---

## 5. Open questions

**One, and it blocks nothing here.**

| # | Question | Blocks | Raised |
|---|---|---|---|
| Q1 | **Who owns `sync_flags`, and does its `type` enum gain a value for `[D-17]`?** The oversell flag `[D-25]` is a cross-cutting `sync_flags` row with `batch_id`, `shortfall_quantity` and `stock_adjustment_id` — not a column here `[AC-U11]`. Its `type` enum has `oversell` and `credit_limit_exceeded` only, so the expired-at-upload case 09 Q5 added has nowhere to go. 06 supplies the facts either way (§3, *Flags are facts here*) | `10-sync`'s Gate 1, and the `REFERENCES` clause on `sync_flags.stock_adjustment_id` → `stock_adjustments`. **Nothing in 06's own tables, commands or routes** | 2026-10-25 |

**A spec with open questions blocking Gate 2 does not advance** — but Q1 blocks nothing in this
module, so it travels with `10-sync` rather than holding 06 at Gate 1. What holds 06 is that
07 and 08 are Phase 0, which §3's *Deferred* table names.

### Answered

**Q2 — 2026-10-25: `reorder_levels.product_id` references `products (id)`, module 01.** Settled
by reading the page rather than choosing: Context 6's ERD draws `products ||--o{ reorder_levels`,
and **its *Constraints* block names no `reorder_levels` foreign key at all**, so the stale
export's claim of `tenant_products (tenant_id, product_id)` has nothing behind it in Notion and
Article III makes Notion win. Two further facts settle it: Context 4 says *"**First purchase of
a product** creates the `tenant_products` row"*, so a `tenant_products` FK would forbid a
reorder level on a product the pharmacy has not yet bought — which is the one time an alert is
most wanted — and Context 6's own derived list defines low stock per *product* per branch, never
per priced product. The key is `(tenant_id, branch_id, product_id)`; the FK to `products` is
**plain, not composite**, because `products` is a shared table with no `tenant_id` and
[`../rules/tenancy.md`](../rules/tenancy.md) rule 5 governs foreign keys *between tenant-owned
tables*. Writable now: 01 is at Phase 5. The same answer applies to `batches.product_id`.

**Q3 — 2026-10-25: `audit_logs` belongs to 00 Foundation.** It is infrastructure, like the error
envelope and `Tx`: every module writes it, none owns it, and it is `tenant_id`-nullable and
polymorphic by documented exception — none of which fits a bounded context. 06 calls a
foundation helper inside its own transaction, so `approve_mrp_change` writes the batch and the
audit row together `[AC-E13]`. **This reopens module 00**, which is at Phase 5 awaiting an
independent Gate 5: the table, its insert-only grant and a helper are a new phase there, not a
quiet addition. 12 Jobs was rejected because it is Phase 0 and would block this write behind a
module nothing else needs yet; a module of its own was rejected as a spec directory, a crate and
a build-order row for one table.

**Q4 — 2026-10-25: `workflows::transfer` stays, and earns its place.** The flow is kept rather
than collapsed into this module's command, and what it adds is the thing a single-module command
cannot do: **it checks that the caller holds the required slug at *both* branches.**
`api::require_at` resolves one branch from the path, so the route layer provably cannot express
it. `POST /branch-transfers` therefore stays `crates/server`'s (09 Q6) and this contract claims
only the reads. `commands::transfer_stock` is unchanged and is what the flow calls.

**This is a narrow, named exception to 09 `[AC-U05]`**, which otherwise forbids a flow to
perform any authorisation — recorded in [`../09-workflows/SPEC.md`](../09-workflows/SPEC.md)
rather than here, because it is that spec's rule to bend. The shape that keeps the rule almost
intact: the flow is **told** the set of branches the caller holds the slug at, as a parameter
the route has already resolved, and asserts both ends are in it. It resolves no permission
itself, reads no `AuthUser` and looks up no slug.

**Q5 — 2026-10-25: a pharmacy may not withdraw a pending MRP request.** No fourth enum value and
no cancel route. A pharmacy that asked for the wrong price asks the platform to reject it, which
matches `[D-38]`'s framing of this as a conversation with the platform. The cost is accepted
knowingly: a self-inflicted typo waits on a reviewer who has no way of knowing it was a mistake,
and the partial unique index blocks a second request on that batch until they act. If MRP
requests turn out to be common, this is the first thing to revisit.



**What is *not* open, although a reader may expect it to be.** The cost basis (§3) is an
**engineering** decision and is taken here, with its reasoning and its price written down, per
[`../rules/requirements.md`](../rules/requirements.md) — *a requirement question is asked, an
engineering question is proposed with its alternatives.* It needs three Notion amendments, below,
but it does not need a requirement answer and it does not block the gate. If a reviewer prefers
a stored per-unit average, the residue is theirs to place and `[AC-U06]`, `[AC-W11]` and 05
`[AC-U08]` all change together.

### Three places the Notion page needs amending

Flagged, not edited. None is a question; each is a place where this contract and the page would
otherwise read as disagreeing, and a reader three weeks from now would not be able to tell which
was right.

1. **The cost basis columns** (§3). `stock_movements.unit_cost` → `cost_total`, documented as a
   **line total** rather than a per-unit figure — which the page's own comment, *"cost of these
   units"*, already says and its column name contradicts. `batches.unit_cost` moves from the
   attribute list to the **derived, not stored** list, where Context 5 already puts *"the
   effective cost per unit with free goods"*. `stock_balances` gains `cost_basis_total` and
   `cost_basis_units`, both running totals of the same ledger, maintained by the same trigger
   and rebuildable by the same `SUM` — which is why they belong on the table that is already
   `SELECT`-only to the application rather than on `batches`, which the application updates.
2. **"Zero-stock blocking locks the balance row"** is not executable as written (§3, *Why the
   lock is on the batch*): PostgreSQL requires `UPDATE` privilege for a row lock and `pos_app`
   holds `SELECT` only on `stock_balances` by the page's own grant rule. The lock is on the
   `batches` row. This is a correction to one sentence, not a design change.
3. **"Every table filters on `tenant_id` and `branch_id` being in `app.branch_ids`"** (§4.1).
   There is no `app.branch_ids` setting and no policy reads one — module 00's Q2, closed as void
   2026-10-24. Branch scoping is `api::require_at` plus a `WHERE` clause. The page describes a
   mechanism this system considered and did not build, and it reads as an RLS guarantee that
   does not exist.

### Where this spec relied on the stale export

[`../../docs/PRODUCT_SPEC.md`](../../docs/PRODUCT_SPEC.md) is an export and Notion wins
(Art. III). The Notion extract of Context 6 is complete and nothing below contradicts it; each
item is something the extract does not carry, used because the alternative was to invent it:

1. **The oversell flag is a cross-cutting `sync_flags` row** — §20's *Cross-cutting tables* ERD,
   with its `type`, `batch_id`, `shortfall_quantity` and `stock_adjustment_id` columns. Notion's
   Context 6 extract says only *"a balance that goes negative on sync raises a flag"*, which is
   what made the flag look like a column here. It is §5 Q1 and `[AC-U11]`.
2. **Near-expiry and low-stock alerts go to cross-cutting `notifications` with a `dedupe_key`**,
   sent by a sweep — which is why [`CONTRACT.md`](CONTRACT.md) §4 declares no job of 06's own and
   exposes two reads instead.
3. **"Stock counts … online only"** (§7) — `[AC-S07]`.
4. **"A manager may approve a blocked action on the cashier's phone with their PIN — a discount
   over the cap, a return without an invoice, a sale at zero stock"** (§8). This is why the
   short-stock behaviour is a **caller-supplied policy** rather than a lookup of
   `allow_zero_stock_sale`: the setting, the manager's approval and an offline replay are three
   routes to the same `Accept`, and 06 must not care which (09 `[AC-U05]`).
5. **`batch_mrp_change_requests`' extra constraints** — `requested_unit_mrp > 0` and differing
   from `current_unit_mrp`, tracked batches only, and platform review *"reading across tenants
   through `app.is_platform`, as with the billing tables"*. Notion's extract carries the partial
   unique index alone.
6. **Three uniques the extract omits**: `batches` unique `(tenant_id, branch_id, id)` for the
   composite FK, and the `UK` marks on `stock_adjustment_items.stock_movement_id` and both
   `branch_transfer_items` movement columns.
7. **`reorder_level >= 0`**, and Context 4's *"Batch MRP and cost price belong to `batches`
   (context 6)"*, which is what settles `batch_mrp_change_requests` being here rather than in 04
   despite `../INDEX.md`'s one-line summary of module 04.

Everything else comes from the Notion Data Model (Context 6 plus *Rules that hold across every
context*), the decision index, and modules 00/01/02/05/09's own contracts.
