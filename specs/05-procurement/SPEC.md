# SPEC: 05 — Procurement

**Status:** Draft — Gate 1. **Does not advance to Gate 2:** four open questions (§5), two of
which (Q1, Q2, answered 2026-10-25) decided whether a route in [`CONTRACT.md`](CONTRACT.md) §2 exists at all, plus
one deferred handoff from **06 Inventory**, which is Phase 0 (§3, *Deferred*).
**Crate:** `crates/procurement`
**Bounded context:** Context 5 — Procurement
**Owner:** `backend-architect` role, drafted 2026-10-25

> **Build order note.** [`../INDEX.md`](../INDEX.md) puts **06 Inventory before 05**. This
> module is being specified first at the user's request, so every place where a batch or a
> stock movement is needed is a **named deferral** in §3 rather than a guess at 06's surface.
> Nothing in this spec or in `CONTRACT.md` invents a table, a command or an error code
> belonging to 06.

---

## 1. Purpose

The supply side of the pharmacy: **who it buys from, what arrived on which bill, what went
back, what it has paid, and what it still owes.** It owns nine tables — the suppliers, the
purchase and its lines, the supplier return and its lines, the payment and the allocations
that match it to bills, and the **append-only supplier ledger** with the running balance a
trigger maintains from it.

Two properties shape everything below.

1. **It is online only.** Purchases, returns and payments never reach a phone (Data Model
   Context 5, opening line; `[D-24]`). Nothing here is replayed by `10-sync` (09 Q4).
2. **It is the first place money and stock meet.** Cost, printed MRP, batch number and expiry
   enter the system on a purchase line and are then *frozen* there forever `[D-38]`, because
   that line is the record of what the supplier's bill said — not a view of what the batch
   became.

**Not this module's job:**

| The nearest wrong assumption | Who owns it |
|---|---|
| Batches, stock on hand, stock movements, expiry sweeps, batch average cost | **06 Inventory.** `purchase_items.batch_id` / `.stock_movement_id` and `supplier_return_items.batch_id` are FKs *into* 06 and are written by the posting flow, not by this module alone (§3 *Deferred*) |
| **Posting a purchase** — creating the batch, the stock movement and the ledger entry together | **09 Workflows** `post_purchase`, 09 `[AC-U03]`, in one caller-owned transaction. The **route** for it lives in `crates/server` with its wire types in `workflows` (09 Q6), so [`CONTRACT.md`](CONTRACT.md) §2 **does not claim that path**. This module owns the purchase tables and the commands the flow calls |
| What the pharmacy *sells* at — selling price, MRP change requests, VAT rate per product | **04 Pricing.** `purchase_items.unit_mrp` is what the pack was printed with on this bill, not a price anyone sells at |
| Changing a batch's MRP after the fact | **Super admin, through 04/06** `[D-38]`. It never edits a purchase line, and this module exposes no endpoint for it |
| Whether a product exists, and whether it is batch-tracked | **01 Catalogue** `[D-8]` |
| The branch the stock arrived at, the pharmacy's VAT BIN, the near-expiry window `[D-5]` | **02 Tenancy** |
| The purchase VAT register and the period VAT return `[D-69]` | **11 Reporting**, reading these tables. This module exposes no report endpoint |
| Purchase orders, expected deliveries, goods-received notes | **Nobody** `[D-40]`. Not in v1, and there is no table for them |
| Who may enter a delivery | **03 Identity** owns the grant `[D-65]`. This module names the slug and nothing else |
| Customer dues, sales, credit notes | **07 Sales** / **08 Customers**. A supplier ledger and a customer ledger are different tables with the same shape, and neither reads the other |

---

## 2. Decisions this module implements

Cite, never restate. The reasoning lives in Notion.

| Ref | Decision | Consequence here |
|---|---|---|
| `[D-38]` | Changing a batch's MRP after purchase is super-admin only, on the pharmacy's request | **A posted purchase is never edited, not even its MRP.** `purchase_items.unit_mrp` is immutable once posted; the correction path is 04/06's and leaves these rows untouched `[AC-U06]` `[AC-U16]` |
| `[D-39]` | Supplier payments are matched to the bills they settle; unpicked money is kept as an advance | `supplier_payment_allocations` carries the match. **The advance is stored nowhere** — it is `amount − Σ allocations`, and the ledger already carries the *full* payment, so an advance is simply a balance that has gone the other way `[AC-E09]` `[AC-E10]` `[AC-S05]` |
| `[D-40]` | No purchase orders in v1 | No PO table, no PO endpoint, no expected-delivery state. A purchase begins as a **draft** when the goods are in front of someone `[AC-U11]` |
| `[D-41]` | VAT on a supplier bill is optional and recorded for reporting only | `purchases.vat_amount` is nullable and **enters no cost, no ledger amount and no purchase total** `[AC-U07]`. It is read by 11 alone |
| `[D-69]` | The VAT return needs a purchase VAT register | `purchases` keeps `invoice_date`, `supplier_invoice_number` and `vat_amount`, and a posted purchase is immutable, so the register is reproducible for a closed period `[AC-S02]`. The register itself is **11's**, reading these tables |
| `[D-10]` | A new batch with a higher MRP prompts and never updates automatically | Posting a line into a batch that already exists **never changes that batch's MRP** `[AC-U16]`. The prompt is the client's, from a read 06 must expose (§3 *Deferred*) |
| `[D-8]` | Batch and expiry are optional for non-medicines | `purchase_items.batch_number` and `.expiry_date` are nullable in the database and **required in Rust for a batch-tracked product**, because the tracked-ness is a fact in 01's shared `products` table and a trigger reaching across a bounded context is the coupling `[D-97]` removes `[AC-W04]` |
| `[D-5]` | The near-expiry window is a tenant setting, default 90 days | Not used here. It is why a user *chooses* to return a batch; this module records `supplier_returns.reason = 'expired'` and nothing else |
| `[D-65]` | Who enters a delivery is a permission, not a fixed role | `purchase.create` is a branch-scoped slug; no route checks a role name |
| `[D-85]` | Stable error codes plus English | Every row of `CONTRACT.md` §2's error table is a code; the Bengali wording is the client's |
| `[D-97]` | Cross-module flows live in a `workflows` crate; modules never call each other | `procurement` depends on no module crate and no module crate depends on it. Posting a purchase is `workflows::post_purchase`; the commands it calls are `CONTRACT.md` §3's |
| `[D-98]` | `commands.rs` takes a transaction, `queries.rs` takes a connection | **As amended by 09 Q3: every function in `queries.rs` is generic over `C: ConnectionTrait`**, so `post_purchase` reads inside its own transaction. `[AC-U15]` carries 09 `[AC-U10]`'s `FOR UPDATE` rule |
| `[D-24]` | Offline scope is sales only | Nothing here is creatable offline, nothing here is replayed (09 Q4), and no table carries a sync flag |
| `[D-99]` | sqlx `query_as!` for reports and non-trivial queries | The outstanding-bills and ledger-statement reads are `query_as!`; the simple row reads are SeaORM |

---

## 3. Behaviour

EARS acceptance criteria, each one a test name at Gate 2.

- **Ubiquitous (always true)**
  - `[AC-U01]` All nine tables SHALL carry `tenant_id`, SHALL have `FORCE ROW LEVEL SECURITY`
    with a tenant policy, SHALL expose a `(tenant_id, id)` unique key, and SHALL reference
    every other tenant-owned row by a **composite** `(tenant_id, …)` foreign key.
  - `[AC-U02]` `supplier_ledger_entries` SHALL be append-only **by grant**: no role SHALL hold
    `UPDATE` or `DELETE` on it, and no code path SHALL exist that attempts either. A wrong
    entry SHALL be corrected by a **new** entry of type `adjustment` carrying a note and the
    acting user, never by changing the entry that was wrong.
  - `[AC-U03]` `supplier_balances` SHALL change **only** as a consequence of inserting a ledger
    entry: `pos_app` SHALL hold `SELECT` only on it, and the balance SHALL be maintained by a
    trigger on `supplier_ledger_entries` that runs in the **same transaction** as the insert.
    For every supplier, `balance` SHALL equal the sum of that supplier's ledger amounts.
  - `[AC-U04]` `supplier_balances.balance` SHALL be allowed to go **negative**, and no check
    constraint SHALL forbid it. A negative balance is money held with the supplier — an
    advance `[D-39]`.
  - `[AC-U05]` A **posted** purchase or supplier return SHALL never be edited **and SHALL never
    be cancelled** (Q2, answered 2026-10-25: posted is final in v1). No endpoint SHALL change
    any column of a posted purchase, and the database SHALL refuse an `UPDATE` or `DELETE` of a
    line whose parent is not a draft. A mistake is corrected by a **supplier return** for goods
    going back, or a **stock adjustment plus a ledger `adjustment`** for an entry error — never
    by rewriting history in either append-only table `[AC-U07]`.
  - `[AC-U06]` `purchase_items.batch_number`, `.expiry_date`, `.unit_cost` and `.unit_mrp`
    SHALL keep the values entered from the supplier's bill for the life of the row, including
    after the batch exists and after the batch is corrected `[D-38]`.
  - `[AC-U07]` `purchases.vat_amount` SHALL NOT enter any cost, any ledger amount, any purchase
    total or any outstanding figure `[D-41]`. It SHALL be read only by `11-reporting` `[D-69]`.
  - `[AC-U08]` For a purchase line, the **units received** SHALL be
    `quantity + free_quantity`, the **money owed** SHALL be `quantity × unit_cost`, and the
    **cost basis handed to 06** SHALL be the line's money owed together with its unit count —
    never a per-unit cost rounded to two decimal places (§*Free goods*, below).
  - `[AC-U09]` Every money value SHALL be `rust_decimal::Decimal` in Rust and
    `numeric(12,2)` in the database, at every layer including serialization
    (constitution Art. IV.5).
  - `[AC-U10]` A purchase total, a return credit and a bill's outstanding amount SHALL be
    **derived, never stored**. The one frozen total in this module is
    `supplier_ledger_entries.amount`, which is why a posted purchase's lines may not change
    `[AC-U05]`.
  - `[AC-U11]` There SHALL be no purchase-order table, column, state or endpoint `[D-40]`.
  - `[AC-U12]` A payment, every bill it is allocated to, and the ledger entry it writes SHALL
    all belong to the **same supplier**, enforced in the command and asserted by test.
  - `[AC-U13]` The allocations of one payment SHALL NOT exceed that payment's `amount`, and the
    allocations against one bill SHALL NOT exceed that bill's total; **a bill SHALL NOT be
    overpaid.**
  - `[AC-U14]` `suppliers`, `supplier_payments`, `supplier_ledger_entries` and
    `supplier_balances` SHALL be **pharmacy-wide** — no branch predicate anywhere.
    `purchases` and `supplier_returns` SHALL additionally be **branch-scoped**, checked in the
    handler and in the command and **not** by row-level security (00 Q2) `[AC-W09]`.
  - `[AC-U15]` Every function in `queries.rs` SHALL be generic over `C: ConnectionTrait`
    (09 Q3), and **every `FOR UPDATE` read SHALL sit in such a function** — never in a
    non-generic one (09 `[AC-U10]`).
  - `[AC-U16]` Posting a line into a batch that already exists SHALL NOT change that batch's
    MRP `[D-38]` `[D-10]`. `purchase_items.unit_mrp` and the batch's MRP MAY therefore differ,
    and that is correct rather than a defect to reconcile.
  - `[AC-U17]` This module SHALL write no table it does not own. Every batch and stock movement
    is written by 06's command, called by `workflows` `[D-97]`.

- **Event-driven (when)**
  - `[AC-E01]` WHEN a supplier is created, the system SHALL store it active, with
    `(tenant_id, name)` unique **among active rows only**.
  - `[AC-E02]` WHEN a supplier name collides with an active supplier, the system SHALL answer
    `409 supplier_name_taken`.
  - `[AC-E03]` WHEN an inactive supplier is reactivated and its name has since been taken by
    another active supplier, the system SHALL answer `409 supplier_name_taken` and leave it
    inactive.
  - `[AC-E04]` WHEN a purchase draft is saved, the system SHALL store the header and its lines
    in one transaction with `status = 'draft'`, and SHALL create **no batch, no stock movement,
    no ledger entry and no change to any balance**.
  - `[AC-E05]` WHEN a purchase draft is saved with a `(supplier_id, supplier_invoice_number)`
    that an existing non-cancelled purchase already has, the **database** SHALL refuse it and
    the system SHALL answer `409 duplicate_supplier_invoice` — a search-then-insert check SHALL
    NOT be the enforcement.
  - `[AC-E06]` WHEN `workflows::post_purchase` calls `commands::begin_posting`, the system
    SHALL lock the purchase and its lines `FOR UPDATE`, validate every posting precondition,
    and return a plan carrying, per line, the product, the batch number, the expiry, the
    **units** (`quantity + free_quantity`), the **line cost total** and the `unit_mrp` — plus
    the invoice total. All arithmetic SHALL happen here, so the workflow computes nothing
    (09 `[AC-U09]`).
  - `[AC-E07]` WHEN `begin_posting` is called for a purchase that is **already posted**, the
    system SHALL return the first posting's result and write nothing — the replay decision
    made in the database, in the caller's transaction, against the purchase's primary key and
    status (09 `[AC-E02]`).
  - `[AC-E08]` WHEN `commands::complete_posting` runs, the system SHALL stamp each line's
    `batch_id` and `stock_movement_id`, **then** set `status = 'posted'` and `posted_at`, and
    SHALL insert **exactly one** ledger entry of type `purchase` for the invoice total, whose
    id is stored in `purchases.ledger_entry_id`. The order matters: the lines are stamped while
    the purchase is still a draft, which is what lets `[AC-U05]`'s refusal be unconditional.
  - `[AC-E09]` WHEN a payment is recorded, the system SHALL insert the payment, its
    allocations and **exactly one** ledger entry of type `payment` for the **full payment
    amount** — not the allocated part — in one transaction `[D-39]`.
  - `[AC-E10]` WHEN an existing payment's unallocated remainder is later applied to a bill, the
    system SHALL insert allocation rows **and no ledger entry**, and the supplier's balance
    SHALL be unchanged. The money was recorded when it was paid; recording it again would
    double-count it.
  - `[AC-E11]` WHEN an allocation is removed, the system SHALL delete the allocation row, write
    no ledger entry, and leave the balance unchanged. An allocation is a *match*, not money,
    which is why it may be deleted while a ledger entry may not `[AC-U02]`.
  - `[AC-E12]` WHEN a supplier's opening balance is recorded, the system SHALL insert **one**
    ledger entry of type `opening`, and a second attempt SHALL be refused by a partial unique
    index with `409 opening_balance_exists`.
  - `[AC-E13]` WHEN a ledger adjustment is recorded, the system SHALL require a note and the
    acting user, and SHALL move the balance by the signed amount.
  - `[AC-E14]` WHEN a **draft** purchase or return is cancelled, the system SHALL set
    `status = 'cancelled'`, SHALL delete nothing, and the supplier invoice number SHALL become
    available again — because the unique index covers non-cancelled rows only.
  - `[AC-E15]` WHEN any ledger entry is inserted, the supplier's balance SHALL be updated in
    the **same transaction**, so a caller reading the balance back inside its own `Tx` sees the
    new value (09 `CONTRACT.md` §1).

- **State-driven (while)**
  - `[AC-S01]` WHILE a purchase or return is a **draft**, it SHALL be replaceable in full —
    header and lines — SHALL be invisible to stock, to payables and to the VAT register, and
    SHALL hold its supplier-invoice-number slot against a second draft for the same bill.
  - `[AC-S02]` WHILE a purchase is **posted**, it SHALL be immutable `[AC-U05]`, SHALL appear
    in the purchase VAT register for its `invoice_date` `[D-69]`, and SHALL be allocatable by a
    payment.
  - `[AC-S03]` WHILE a purchase is **cancelled** — which it can become only from `draft`
    (Q2) — it SHALL be excluded from the supplier-invoice unique index, from payables, from the
    VAT register and from any new allocation.
  - `[AC-S04]` WHILE a supplier is **inactive**, no new purchase, return or payment SHALL name
    it, and its ledger and balance SHALL remain readable — a pharmacy that stops buying from a
    supplier still owes it money.
  - `[AC-S06]` WHILE a purchase line's `unit_mrp` is **below its `unit_cost`**, the system SHALL
    accept the line and SHALL return a warning on it that the client shows prominently (Q3,
    answered 2026-10-25). It SHALL NOT refuse the line: a pack genuinely printed below what was
    paid for it exists, and a `400` would force the pharmacy to misstate either figure to enter
    a real bill. It SHALL NOT be silent either — the loss is otherwise invisible until a margin
    report.
  - `[AC-S07]` WHILE a supplier payment is **cancelled**, it SHALL be excluded from the
    allocation check `[AC-W06]`, from the outstanding-bills read and from the supplier's balance
    (Q4, answered 2026-10-25), and its allocations SHALL be removed in the same transaction. A
    cancelled payment SHALL remain readable and SHALL be visibly cancelled in any payments list.
  - `[AC-S05]` WHILE a supplier's balance is **negative**, the pharmacy holds an advance with
    that supplier `[D-39]`. It is read back by `GET /suppliers/{id}` and
    `GET /supplier-balances`, and the unallocated remainder per payment by
    `GET /supplier-payments/{id}`.

- **Unwanted (if / then)**
  - `[AC-W01]` IF a non-draft purchase or return is edited or posted, THEN the system SHALL
    answer `409 purchase_not_draft` / `409 supplier_return_not_draft` and write nothing.
  - `[AC-W02]` IF a purchase with no lines is posted, THEN the system SHALL answer
    `409 purchase_empty`.
  - `[AC-W03]` IF `invoice_discount` exceeds the lines plus `other_charges`, THEN the system
    SHALL answer `400 validation_failed` — both when the draft is saved and again under the
    lock in `begin_posting`, because the lines may have changed since.
  - `[AC-W04]` IF a line names a batch-tracked product without a batch number or without an
    expiry date, THEN the system SHALL answer `400 validation_failed` naming the field
    `[D-8]`.
  - `[AC-W05]` IF a line's `expiry_date` is not later than the purchase's `invoice_date`, THEN
    the system SHALL answer `400 validation_failed`, **and** the database SHALL refuse it by
    trigger — a check constraint cannot express it, because `invoice_date` is on the parent row.
  - `[AC-W06]` IF a payment's allocations exceed the payment, THEN the system SHALL answer
    `409 allocation_exceeds_payment`.
  - `[AC-W07]` IF an allocation would take a bill past its total, THEN the system SHALL answer
    `409 bill_already_settled`, having read the bill's outstanding amount `FOR UPDATE`
    `[AC-U15]`.
  - `[AC-W08]` IF a payment is allocated to a bill belonging to a different supplier, THEN the
    system SHALL answer `409 supplier_mismatch`.
  - `[AC-W09]` IF a caller reads or writes a purchase or return at a branch it holds no
    permission at, THEN the system SHALL answer `403 branch_not_allowed`; IF the row belongs to
    another **pharmacy**, THEN it SHALL answer `404 not_found`
    ([`../00-foundation/CONTRACT.md`](../00-foundation/CONTRACT.md) §2).
  - `[AC-W10]` IF an `UPDATE` or `DELETE` is attempted against `supplier_ledger_entries`, or an
    `INSERT`/`UPDATE`/`DELETE` against `supplier_balances`, THEN the **database** SHALL refuse
    it for lack of a grant, regardless of what any code path intends `[AC-U02]` `[AC-U03]`.
  - `[AC-W11]` IF a write is attempted while the pharmacy's access mode is read-only, THEN the
    route SHALL answer `403 subscription_read_only` before any command runs `[D-20]`, AND every
    read in this module SHALL still succeed — a pharmacy that cannot pay its subscription must
    still be able to see what it owes its suppliers.
  - `[AC-W12]` IF a return line exceeds what is on hand in that batch, THEN the system SHALL
    answer with **06's** code, deferred (§*Deferred*). This module SHALL NOT invent one.
  - `[AC-W13]` IF a quantity, cost or MRP is zero or negative, or `free_quantity` is negative,
    THEN the system SHALL answer `400 validation_failed`, **and** the database SHALL refuse it
    by check constraint.

**Offline behaviour.** **None, and this is the strongest "no" in the system.** Purchases,
returns and payments are online only (Data Model Context 5, opening line; `[D-24]`), nothing
here reaches a phone, no table carries a sync flag or a device id, and `10-sync` replays `sell`
alone (09 Q4). Identifiers are UUIDv7 but **server-generated**, because
[`../rules/tenancy.md`](../rules/tenancy.md) rule 10 ties client generation to offline
creation and there is none here.

One nuance, and it is the only idempotency this module needs: `workflows::post_purchase`'s
root id is the **existing draft purchase's id**, not a new id the flow mints — the root row was
created by a prior, single-module `POST /purchases`. So 09 `[AC-E02]`'s replay check is
"is this purchase already posted", made on the primary key under `FOR UPDATE` inside the
caller's transaction `[AC-E07]`. Separately, and for the same reason a double-tap on a flaky
admin connection must not pay a supplier twice, `POST /purchases`, `POST /supplier-returns` and
`POST /supplier-payments` **accept the caller's UUIDv7 `id`** and answer `200` with the existing
row on a repeat — the idiom `00-foundation/CONTRACT.md` §2 already documents as "a repeat create
with the same id".

### Free goods, and where the quiet money bug lives

A `10+1` line is `quantity = 10`, `free_quantity = 1`, `unit_cost = 8.50`. Three figures follow,
and confusing any two of them is a money bug that no test about stock levels would catch:

| Figure | Value | Who uses it |
|---|---|---|
| Units received | `11` | 06's stock movement — `quantity + free_quantity` |
| Money owed | `85.00` = `10 × 8.50` | the purchase total, the ledger entry, the bill's outstanding |
| Cost basis of the batch | `85.00` over `11` units | 06's batch cost, and therefore every margin ever computed on this stock |

**`purchase_items.unit_cost` is the invoice unit cost and is never the batch's cost basis when
`free_quantity > 0`.** If 06 copies it straight onto the batch, inventory is valued at
`11 × 8.50 = 93.50` against `85.00` actually owed, and the margin on every sale from that batch
is understated by 9%. The data model states the effective cost as
`quantity × unit_cost ÷ (quantity + free_quantity)` — `7.7272…` here, which **does not fit
`numeric(12,2)`**, and `7.73 × 11 = 85.03 ≠ 85.00`.

So this module hands 06 the **line cost total (`85.00`) and the unit count (`11`)**, not a
rounded per-unit cost `[AC-U08]`. Inventory valuation then reconciles to the ledger exactly, and
any per-unit figure a screen wants is derived at display time. This is an **engineering
proposal to 06**, recorded in *Deferred* below rather than decided for it; what is decided here
is that **this module never emits a rounded per-unit cost**, because a two-decimal division is
the one shape in which the error compounds silently across every batch in the pharmacy.

`[D-41]`'s VAT sits beside this and must not be mixed into it: `unit_cost` is already
VAT-inclusive, so the cost basis is VAT-inclusive, so margin is computed against a
VAT-inclusive cost — and `purchases.vat_amount` is a *separate, informational* figure for the
register `[AC-U07]`. Adding it to a cost, or subtracting it from one, double-counts it.

### The ledger, and what a correction means

`supplier_ledger_entries` is append-only by grant ([`../rules/tenancy.md`](../rules/tenancy.md)
rule 7) and `supplier_balances` is a running total maintained by a trigger on it. Three
consequences, stated plainly because each is a thing someone will otherwise try:

1. **A wrong entry is reversed, never updated.** There is no edit endpoint, no `UPDATE` grant
   and no `DELETE` grant — not for `pos_app`, not for `pos_jobs`, not for anyone but the
   migration's owner role. A mistyped opening balance is corrected by an `adjustment` entry of
   the opposite sign carrying a note and the acting user, and both rows stay. The ledger is the
   history of what was believed, not a view of what is currently true `[AC-U02]`.
2. **The trigger is this module's**, because this module owns both tables. It is
   `AFTER INSERT … FOR EACH ROW` on `supplier_ledger_entries`, upserting
   `supplier_balances` by `balance = balance + NEW.amount` in the **same transaction**, and it
   is `SECURITY DEFINER` with a pinned `search_path` so that `pos_app` can be given
   `SELECT`-only rights on `supplier_balances` `[AC-U03]`. The balance is therefore unreachable
   except through a ledger insert: there is no code path, correct or buggy, that can set it to a
   number the ledger does not say.
3. **It is rebuildable, and that is the test.** `balance` must equal
   `SUM(amount) GROUP BY supplier_id` at all times; the rebuild statement is the Gate-5
   assertion for the trigger, and the reconciliation a human runs after any incident. Nothing
   recomputes the balance in Rust — a module that both maintains and recomputes a running total
   has two answers and no way to tell which is wrong.

### `draft` → `posted` → `cancelled`

| State | What it may do | What it has done to the world |
|---|---|---|
| `draft` | everything: the header and the whole line set are replaceable `[AC-S01]`. It is one `PUT`, not a line CRUD surface | nothing — no batch, no movement, no ledger entry, no balance change. It does hold the supplier-invoice-number slot `[AC-E05]` |
| `posted` | **nothing** `[AC-U05]`. Not a line, not a cost, not the MRP `[D-38]` | the batch exists, the stock is in, the invoice is owed. `posted_at` and `ledger_entry_id` are set |
| `cancelled` | nothing | reachable **only from `draft`** (Q2): a cancelled draft did nothing and leaves nothing. A posted purchase is never cancelled `[AC-U05]` |

**Posting is the workflow's transaction, not this module's** (09 `[AC-U03]`): `begin_posting`
and `complete_posting` are commands that take `&Tx`, and between them the workflow calls 06.
Nothing here opens, commits or rolls back. Partial failure therefore has no repair path and
needs none — the caller's transaction rolls back whole and the client retries the identical
request, which `[AC-E07]` makes safe.

**A draft is cancelled, never deleted.** `DELETE` is not granted on `purchases` or
`supplier_returns`, so an abandoned draft must be cancelled before the same supplier bill can be
entered again — the direct and intended consequence of the unique index covering non-cancelled
rows.

**A payment can be cancelled** (Q4, answered 2026-10-25). `supplier_payments` gains a status
column — the data model does not have one, so this is a **Notion amendment**, flagged in
[`../INDEX.md`](../INDEX.md) *Requirement drift* — and `POST /supplier-payments/{id}/cancel`
removes its allocations and takes it out of the balance in one transaction `[AC-S07]`. Leaving
the wrong row standing with a compensating `adjustment` was rejected: the payments list is what
a supplier argues from, and a report showing a payment that never happened is an argument the
pharmacy cannot win.

### Deferred — what waits on 06 Inventory

> **All seven rows were discharged on 2026-10-25** by `06-inventory`'s Gate 1, which is drafted
> and awaiting review. Kept here as the record of what was asked and what came back, because
> two of the answers were reasoned refusals rather than deliveries: there is **no allocation
> command** (`[D-4]` gives the batch choice to a person and `[D-9]` makes each choice its own
> line) and **no flag columns in 06** (the flag belongs to `10-sync`'s own table; 06 returns the
> facts on its result type instead). The cost basis came back as `CostBasis::Exact { line_cost_total, units }`
> — this module's proposal, accepted — and the reversing movement for a posted return is
> `record_stock_out` with `PurchaseReturnOut`, since a reversal in that ledger is a new opposite
> movement rather than an undo. The row about a posted purchase's cancellation is moot: **Q2**
> answered that it does not happen in v1.

**06 Inventory is Phase 0: no `SPEC.md`, no `CONTRACT.md`, no crate, no tables.** Three FK
columns in this module point into it, so the handoff is named here rather than guessed. Nothing
in this table is an open *question* about the product; each is settled work that is 06's to
specify.

| 06's Gate 1 must give this module | Without it we cannot write |
|---|---|
| A **create-or-find batch + stock-in** command, taking `(branch_id, product_id, batch_number, expiry_date, unit_mrp)` plus the cost basis, writing `quantity + free_quantity` units as one `purchase_in` movement, and returning `(batch_id, stock_movement_id)` per line | `workflows::post_purchase`'s middle step, and the type of `PostingLine` in `CONTRACT.md` §3 |
| The **shape of the cost basis**: a line cost total plus a unit count, per the proposal above, and the scale of the column it is stored in. If 06 chooses a per-unit cost, it owns the rounding residue and must say where it goes | `[AC-U08]`, and whether inventory valuation reconciles to the supplier ledger at all |
| What makes a batch **unique** — presumably `(tenant_id, branch_id, product_id, batch_number)` — which is what decides whether a second bill for the same batch number finds the batch or creates a second one | `[AC-U16]`, and the "create or find" in the step above |
| A read exposing an **existing batch's MRP** for a `(branch, product, batch_number)`, generic over `C: ConnectionTrait`, so the client can prompt on a higher MRP `[D-10]` and the posting can refuse to change it `[D-38]` `[AC-U16]` | the prompt on the purchase-entry screen, and `[AC-U16]`'s assertion |
| The **on-hand `FOR UPDATE` read** and the error code for a short batch (`insufficient_stock` is already reserved in `00-foundation/CONTRACT.md` §2) | `[AC-W12]` and `post_supplier_return`, now a seventh flow (Q1). Cancellation of a posted purchase is **out of v1** (Q2), so nothing waits on a reversing movement |
| Whether **06 or 01** owns "is this product batch-tracked" `[D-8]`, and a read for it generic over `C: ConnectionTrait` | `[AC-W04]`. If it is 01's `products.product_type`, `catalog`'s reads are not generic today (09 Q3) and that module needs a backlog item — not this module's to file |
| The **reversing movement** for a cancelled posting, if posted-purchase cancellation survives **Q2** | `POST /purchases/{id}/cancel` for a posted purchase |

**What this means for the gate.** `CONTRACT.md` §1 (the nine tables) and the single-module half
of §2 are complete and reviewable now. The posting handoff in §3 is named to the field but its
*counterpart* in 06 is not written, so `plan.md` cannot be written against it
([`../rules/lifecycle.md`](../rules/lifecycle.md)).

---

## 4. Tenancy and permissions

| Question | Answer |
|---|---|
| Tenant-owned? | **Yes, all nine tables.** There is no shared or platform-owned table in this context and no platform-write policy on any of them — a super admin acts through impersonation under a real tenant context, and `[D-38]`'s MRP change lands in 04/06, not here |
| Branch-scoped? | **Split, deliberately.** `suppliers`, `supplier_payments`, `supplier_ledger_entries`, `supplier_balances` are **pharmacy-wide**: a pharmacy owes one supplier one amount no matter which shop took the delivery. `purchases` and `supplier_returns` are **branch-scoped**, because stock belongs to a branch `[AC-U14]` |
| Permissions required | Sixteen slugs, module `procurement`, listed in `CONTRACT.md` §2. `purchase.*` and `supplier_return.*` are `is_branch_scoped = true`; `supplier.*`, `supplier_payment.*` and `supplier_ledger.*` are not |
| Visible to a cashier? | **Nothing in this module, by default.** The shipped cashier template gets none of these slugs; no response here is reachable without one, and the supplier list carries **no money column** so that `supplier.view` can be given to someone who may receive a delivery without also showing them what the pharmacy owes. Payables are behind `supplier_ledger.view` alone |

Four things **specific** to this module:

1. **Branch scoping here is not row-level security.** Module 00's Q2 was closed as void — no
   `app.branch_ids` setting exists and no policy reads a branch list. So a purchase at another
   branch is excluded by a `WHERE` clause in the query and by a check in the command, and the
   test for it is an HTTP test, not an RLS test `[AC-U14]` `[AC-W09]`.
2. **`api::require_at` cannot be used by any route here**, because no path in this module carries
   a branch segment: `branch_from_path` would find nothing and `check_permission_at` would fall
   back to the tenant-wide check. Every route therefore uses `api::require(slug)` and does its
   branch scoping in the handler against `AuthUser.branch_permissions` — a list filtered to the
   branches the caller holds the slug at, a single read checked after it (403
   `branch_not_allowed`), a write checked before the command runs. This is recorded as a
   convenience request to 00 in `CONTRACT.md` §3, not as a gap: the data the handler needs is
   already on `AuthUser`.
3. **Every write sits behind the access-mode gate, every read does not** `[AC-W11]`. A read-only
   subscription `[D-20]` must still let a pharmacy see its payables; it must not let it record a
   payment.
4. **The ledger's isolation is a grant, not a policy.** The tenant policy stops pharmacy A
   reading pharmacy B's ledger; the **absence of an `UPDATE` grant** is what stops anyone, in
   any tenant, from rewriting history. Both are needed and they are different mechanisms
   ([`../rules/tenancy.md`](../rules/tenancy.md) rules 2 and 7).

---

## 5. Open questions

**None.** All four were answered on 2026-10-25. The module stays at Phase 1 because **06
Inventory is Phase 0** (§3, *Deferred*), not because anything here is undecided.

### Answered

**Q1 — `post_supplier_return` is a seventh `workflows` flow.** It spans 05 and 06 and must be
atomic, so it goes where `post_purchase` and `void_sale` went; `POST /supplier-returns/{id}/post`
is `crates/server`'s route with its wire types in `workflows` (09 Q6), and this contract still
claims neither. `supplier_returns.status = 'posted'` is therefore reachable once 06 exists.
Folding it into `post_purchase` behind a direction flag was rejected: the two share almost
nothing — different tables, opposite ledger sign, different preconditions, and `[D-17]`'s
expired-batch rule applies to the return and not the purchase. **`[D-97]` needs amending in
Notion for the third time in two days**, which is itself the finding: it should state the rule —
*a flow is added when it provably spans two or more modules* — and stop enumerating files.

**Q2 — cancelling a posted purchase is not in v1.** A posted purchase is final. A mistake is
corrected the way the data model already provides: goods going back are a **supplier return**,
and an entry error is a **stock adjustment** plus a ledger `adjustment` entry. Nothing is added:
no `cancellation` ledger type, no second pointer column on `purchases`, no cross-module reversal
flow, and both append-only invariants stay untouched `[AC-U07]`. `purchases.status = 'cancelled'`
is reachable **only from `draft`** `[AC-S03]`.

**Q3 — a line with `unit_mrp` below `unit_cost` is accepted with a warning** `[AC-S06]`. It is
saved, and `PurchaseLine` carries a warning the client shows prominently. Rejecting it was
rejected: a pack genuinely printed below what was paid for it does happen, and a `400` would
force the pharmacy to misstate either the cost or the MRP to enter a real bill — corrupting the
data worse than the typo does. Accepting silently was rejected because nobody would notice until
the margin report.

**Q4 — `supplier_payments` gains a status, and a cancelled payment is excluded from the
arithmetic** `[AC-E09]`, `[AC-S07]`. A wrong payment is cancelled rather than left standing with
a compensating ledger entry, because the payments list is the record a supplier will argue from:
a report showing a ৳5,000 payment that never happened is an argument the pharmacy cannot win.
The exclusion is threaded through the allocation check, the outstanding-bills read and the
balance.

**A spec with open questions blocking Gate 2 does not advance.** Ask; do not invent.

### Where this spec relied on the stale export

[`../../docs/PRODUCT_SPEC.md`](../../docs/PRODUCT_SPEC.md) §6 is an export and Notion wins
(Art. III). It was used for exactly two things the Data Model does not carry, both marked
`[Decided]` there and neither contradicting Notion:

1. **"Purchase entry is the most important screen in the system"** — which is why
   `CONTRACT.md` §2 makes a draft **one whole-document `PUT`** rather than a per-line CRUD
   surface. The screen submits one bill; a line-at-a-time API would be three round trips per
   row on a counter connection.
2. **`[D-65]`'s wording, "who may enter a delivery is a permission, not a fixed role"** —
   which is why `purchase.create` is a branch-scoped slug and no route names a role.

Everything else in this spec comes from the Data Model page (Context 5 plus *Rules that hold
across every context*), the decision index, and modules 00/02/03/09's own contracts.

### Two places the Notion page needs amending

Flagged, not edited, and neither is a question — the page's prose already settles both, while
its ERD attribute list does not say so:

1. **`purchases`, `supplier_returns` and `supplier_payments` each need a `ledger_entry_id`
   column.** The ERD's attribute lists omit it, but the relationship lines
   (`purchases ||--o| supplier_ledger_entries`) and the BCNF note — *"The ledger entry does not
   name its source. The purchase, return or payment points at its entry"* — require it, and it
   is the only shape that avoids the polymorphic FK the cross-context rules forbid.
2. **`invoice_date`, `expiry_date` and `paid_at` are `date`**, per Context 5's own column types,
   while *Rules that hold across every context* says "dates are `timestamptz`". The specific
   wins: a bill's date is a calendar date in Asia/Dhaka, not an instant. `posted_at` and
   `occurred_at` are `timestamptz`, which is what that rule is about.
