# CONTRACT: 05 — Procurement

**Status:** Draft — Gate 1
**Crate:** `crates/procurement`
**Spec:** [`SPEC.md`](SPEC.md)
**Bounded context:** Context 5 — Procurement

> Gate 1 artifact. The tables this module owns and the HTTP surface it exposes, copied from
> Data Model Context 5 and the API Design Guide — this is the module's slice of them, not a
> second version. Where they disagree, they win and this is corrected.
>
> The envelope, the error shape, the headers, cursor pagination and the shared error codes are
> [`../00-foundation/CONTRACT.md`](../00-foundation/CONTRACT.md) §2 and are **not restated**.
>
> **Two routes that a reader will look for are deliberately absent**, and §2 says where each
> one went: both `POST /purchases/{id}/post` and `POST /supplier-returns/{id}/post` belong to
> `crates/server`, with their wire types in `workflows` (09 Q6). The second became a seventh
> flow when **Q1** was answered on 2026-10-25.

---

## 1. Tables owned

Nine tables, **all tenant-owned**, all with `tenant_id`, `ENABLE` + `FORCE ROW LEVEL SECURITY`
and a tenant policy. No shared table, no platform-owned table, no platform-write policy.

| Table | Keys | Tenant-scoped uniques | Append-only | Notes |
|---|---|---|---|---|
| `suppliers` | `id` PK, `(tenant_id, id)` UK | `(tenant_id, name)` **among active rows** | no | Pharmacy-wide. `name`, `contact_person`, `phone`, `email`, `address`, `is_active`, `note` |
| `purchases` | `id` PK, `(tenant_id, id)` UK | `(tenant_id, supplier_id, supplier_invoice_number)` **among non-cancelled rows** · `(tenant_id, ledger_entry_id)` | no | Branch-scoped. `status` ∈ `draft \| posted \| cancelled`; `other_charges`, `invoice_discount` default `0.00`; `vat_amount` **nullable** `[D-41]`; `posted_at`, `ledger_entry_id` nullable until posted |
| `purchase_items` | `id` PK, `(tenant_id, id)` UK | — | no | `batch_number`, `expiry_date` nullable `[D-8]`; `quantity`, `free_quantity`, `unit_cost`, `unit_mrp`; `batch_id`, `stock_movement_id` **nullable, set at posting** — FKs into **06**, deferred |
| `supplier_returns` | `id` PK, `(tenant_id, id)` UK | `(tenant_id, ledger_entry_id)` | no | Branch-scoped. `purchase_id` **nullable** ("the original purchase, if known"); `reason` ∈ `expired \| damaged \| wrong_item \| other`; same three-state `status` |
| `supplier_return_items` | `id` PK, `(tenant_id, id)` UK | — | no | `batch_id` FK into **06** (deferred), `quantity`, `unit_cost` (value credited), `stock_movement_id` set at posting |
| `supplier_payments` | `id` PK, `(tenant_id, id)` UK | `(tenant_id, ledger_entry_id)` only | no | Pharmacy-wide. `branch_id` **nullable** ("which shop paid"); `method` ∈ `cash \| bank \| bkash \| cheque`; `amount`, `reference`, `paid_at`, `note`. **`status` ∈ `active \| cancelled`**, default `active` — added by Q4 (2026-10-25). Context 5 has no such column, so it is a **Notion amendment**, flagged in [`../INDEX.md`](../INDEX.md) *Requirement drift*. A cancelled payment leaves the balance, the allocation check and the outstanding-bills read `[AC-S07]` |
| `supplier_payment_allocations` | `(tenant_id, payment_id, purchase_id)` PK | the key | no | `amount`. The only table here with `DELETE` granted, because an allocation is a *match* and not money `[AC-E11]` |
| `supplier_ledger_entries` | `id` PK, `(tenant_id, id)` UK | `(tenant_id, supplier_id)` **where `type = 'opening'`** | **YES** | `type` ∈ `opening \| purchase \| purchase_return \| payment \| adjustment`; `amount` signed (positive = we owe more); `note` and `created_by_user_id` for `opening` and `adjustment` **only**; `occurred_at` |
| `supplier_balances` | `(tenant_id, supplier_id)` PK | the key | no (but **unwritable**) | `balance`, running total. `pos_app` holds **`SELECT` only**; the trigger is the only writer `[AC-U03]` |

**`ledger_entry_id` on the three source tables is required by the page's prose, not by its ERD
attribute list** — *"the purchase, return or payment points at its entry"*. It is the shape that
keeps `supplier_ledger_entries` free of the polymorphic FK the cross-context rules forbid, and
it is flagged for a Notion amendment in [`SPEC.md`](SPEC.md) §5.

Every `status`, `method`, `reason` and `type` is **`VARCHAR` + `CHECK`**, not a Postgres enum —
the convention `02-tenancy` and `01-catalog` already use, so adding a value is a migration and
not a type rewrite.

### Keys and constraints specific to this module

**Tenant-scoped uniques** — all as partial indexes where the data model says "among":

| Index | Why |
|---|---|
| `UNIQUE (tenant_id, name) WHERE is_active` on `suppliers` | Two pharmacies may both have a "Square"; one pharmacy may not have two active ones. Case-sensitive, exactly as the data model states |
| `UNIQUE (tenant_id, supplier_id, supplier_invoice_number) WHERE status <> 'cancelled'` on `purchases` | **The same supplier bill cannot be entered twice**, by the database and not by a search-then-insert check `[AC-E05]`. It covers **drafts**, which is what makes an abandoned draft something to cancel rather than ignore `[AC-E14]` |
| `UNIQUE (tenant_id, supplier_id) WHERE type = 'opening'` on `supplier_ledger_entries` | "Money owed before the system started is **one** `opening` ledger entry" `[AC-E12]` |
| `UNIQUE (tenant_id, ledger_entry_id)` on `purchases`, `supplier_returns`, `supplier_payments` | One ledger entry has at most one source. Without it, two bills could point at one charge. Implied by the ERD's `o|` cardinality rather than stated as a constraint |

**There is deliberately no uniqueness on `supplier_payments.reference`.** Context 5's
*Constraints* section is itemised table by table and names none, and ends *"Nothing open here"* —
so the same cheque number may be entered twice, and the guard against a double-submitted payment
is the caller-supplied UUIDv7 `id` (§5). `02-tenancy` does constrain
`subscription_payments (method, reference)`, and a reviewer may well want the same here; that is a
Notion edit and a decision, not something this contract may add on the strength of a
neighbouring table.

**Composite foreign keys** — `(tenant_id, …)` throughout, Art. IV.3:

| From | To | Owner |
|---|---|---|
| `purchases (tenant_id, supplier_id)` · `supplier_returns` · `supplier_payments` | `suppliers (tenant_id, id)` | 05 |
| `purchases (tenant_id, branch_id)` · `supplier_returns` · `supplier_payments` (nullable) | `branches (tenant_id, id)` | 02 |
| `purchase_items (tenant_id, purchase_id)` | `purchases (tenant_id, id)` | 05 |
| `supplier_return_items (tenant_id, supplier_return_id)` | `supplier_returns (tenant_id, id)` | 05 |
| `supplier_returns (tenant_id, purchase_id)` nullable | `purchases (tenant_id, id)` | 05 |
| `supplier_payment_allocations (tenant_id, payment_id)` / `(tenant_id, purchase_id)` | `supplier_payments` / `purchases` | 05 |
| `purchases (tenant_id, ledger_entry_id)` · `supplier_returns` · `supplier_payments` | `supplier_ledger_entries (tenant_id, id)` | 05 |
| `supplier_ledger_entries (tenant_id, supplier_id)` · `supplier_balances` | `suppliers (tenant_id, id)` | 05 |
| `purchase_items.product_id` | `products (id)` — **plain, not composite**, because `products` has no `tenant_id` | 01 |
| `purchase_items (tenant_id, batch_id)` · `(tenant_id, stock_movement_id)` · `supplier_return_items (tenant_id, batch_id)` · `(tenant_id, stock_movement_id)` | **06's tables — deferred.** The columns exist as nullable `UUID`; the `REFERENCES` clause is added by **06's** migration, because the table it points at does not exist yet | 06 |
| `created_by_user_id` on `purchases`, `supplier_returns`, `supplier_payments`, `supplier_ledger_entries` | `users (tenant_id, id)` | 03 |

**Check constraints** — each in the database because each is a single-row fact:

| Constraint | Table | Why in the database |
|---|---|---|
| `CHECK (quantity > 0 AND free_quantity >= 0 AND unit_cost > 0 AND unit_mrp > 0)` | `purchase_items` | "positive quantities and costs" `[AC-W13]`. Free goods are a count, not a zero-cost line |
| `CHECK (quantity > 0 AND unit_cost > 0)` | `supplier_return_items` | as above |
| `CHECK (other_charges >= 0 AND invoice_discount >= 0 AND (vat_amount IS NULL OR vat_amount >= 0))` | `purchases` | `[D-41]`'s VAT is optional, never negative |
| `CHECK (amount > 0)` | `supplier_payments`, `supplier_payment_allocations` | "positive amounts". A refund from a supplier is a `purchase_return`, not a negative payment |
| `CHECK (amount <> 0)` | `supplier_ledger_entries` | Signed, but a zero-value entry says nothing and would still fire the trigger |
| `CHECK ((type IN ('opening','adjustment')) = (note IS NOT NULL AND created_by_user_id IS NOT NULL))` | `supplier_ledger_entries` | The column comments made structural: a correction always records **who** and **why**, and an automatic entry records neither `[AC-E13]` |
| `CHECK ((status = 'posted') = (posted_at IS NOT NULL))` and `CHECK (status <> 'posted' OR ledger_entry_id IS NOT NULL)` | `purchases`, `supplier_returns` | Two columns that must agree, written by one flow and read by three modules |
| `CHECK (status IN ('draft','posted','cancelled'))`, `reason IN (…)`, `method IN (…)`, `type IN (…)` | each | enum discipline without a Postgres enum |

**Two rules that cannot be check constraints, and what carries them instead:**

| Rule | Mechanism | Why not a `CHECK` |
|---|---|---|
| A line's `expiry_date` must be **later than the purchase's `invoice_date`** | A trigger on `purchase_items` (`BEFORE INSERT OR UPDATE`) reading the parent's `invoice_date`, **plus** a trigger on `purchases` re-validating its lines when a draft's `invoice_date` changes. Rust raises `400 validation_failed` first, so the trigger is the backstop `[AC-W05]` | `invoice_date` is on the parent row; a row-level `CHECK` cannot see it |
| No `UPDATE` or `DELETE` of a line whose parent is **not a draft** | A trigger on `purchase_items` and `supplier_return_items` refusing it. `complete_posting` stamps `batch_id`/`stock_movement_id` **before** flipping `status` `[AC-E08]`, which is what lets the refusal be unconditional rather than an exception list | `[AC-U05]` is about the parent's state |
| A batch-tracked product's line requires `batch_number` and `expiry_date` `[D-8]` | **Rust, in the command**, via a 01 read | The fact lives in 01's shared `products` table, and a trigger joining across a bounded context is the coupling `[D-97]` removes (`SPEC.md` §3 *Deferred*) |

**Triggers**

| Trigger | On | Does |
|---|---|---|
| `trg_supplier_ledger_balance` | `AFTER INSERT ON supplier_ledger_entries FOR EACH ROW` | `INSERT INTO supplier_balances (tenant_id, supplier_id, balance) VALUES (NEW.tenant_id, NEW.supplier_id, NEW.amount) ON CONFLICT (tenant_id, supplier_id) DO UPDATE SET balance = supplier_balances.balance + NEW.amount`. **`SECURITY DEFINER` with `SET search_path = pg_catalog, public`**, so `pos_app` needs no write grant on `supplier_balances` and the balance is unreachable except through a ledger insert `[AC-U03]` `[AC-E15]`. It writes exactly `NEW.tenant_id`, so definer rights widen nothing |
| `trg_purchase_item_expiry` | `BEFORE INSERT OR UPDATE ON purchase_items` | the expiry-after-invoice-date rule above `[AC-W05]` |
| `trg_purchase_invoice_date` | `BEFORE UPDATE OF invoice_date ON purchases` | re-validates the draft's lines against the new date |
| `trg_purchase_item_frozen` | `BEFORE UPDATE OR DELETE ON purchase_items`, `supplier_return_items` | refuses when the parent is not a draft `[AC-U05]` |

**There is no trigger that recomputes a balance**, and no Rust function that does either. The
rebuild statement —
`SELECT supplier_id, SUM(amount) FROM supplier_ledger_entries GROUP BY 1` against
`supplier_balances` — is a **test and a reconciliation**, never a repair the application runs.

**Indexes the queries actually need** (every one leads with `tenant_id`):

| Index | Query it serves |
|---|---|
| `suppliers (tenant_id, is_active, name)` | the supplier list. **No trigram** — a pharmacy has tens of suppliers, so prefix `ILIKE` is cheaper than a GIN index to maintain |
| `purchases (tenant_id, branch_id, id DESC)` | the branch-filtered purchase list, cursor-paginated on `id` |
| `purchases (tenant_id, supplier_id, status)` | outstanding bills for a supplier |
| `purchases (tenant_id, invoice_date)` | date filters and 11's purchase VAT register `[D-69]` |
| `purchase_items (tenant_id, purchase_id)` | a purchase with its lines |
| `purchase_items (tenant_id, product_id)` | "what did we last pay for this product" |
| `purchase_items (tenant_id, batch_id)` | **06 → 05**: which bill created this batch |
| `supplier_returns (tenant_id, branch_id, id DESC)` · `(tenant_id, supplier_id)` · `(tenant_id, purchase_id)` | the return list, a supplier's returns, a bill's credits |
| `supplier_return_items (tenant_id, supplier_return_id)` · `(tenant_id, batch_id)` | a return with its lines; a batch's return history |
| `supplier_payments (tenant_id, supplier_id, paid_at DESC, id DESC)` | the payment list and the supplier statement |
| `supplier_payment_allocations (tenant_id, purchase_id)` | **bill → payments**, which is the outstanding calculation. The PK already serves payment → bills |
| `supplier_ledger_entries (tenant_id, supplier_id, occurred_at DESC, id DESC)` | the supplier statement, ordered by business moment then insertion order |

### Grants

| Role | Rights |
|---|---|
| `pos_app` | `SELECT, INSERT, UPDATE` on `suppliers`, `purchases`, `purchase_items`, `supplier_returns`, `supplier_return_items` · `SELECT, INSERT` on `supplier_payments` (**no `UPDATE`** — a payment is never edited, and there is no void: `SPEC.md` **Q4**) · `SELECT, INSERT, DELETE` on `supplier_payment_allocations` · **`SELECT, INSERT` only** on `supplier_ledger_entries` · **`SELECT` only** on `supplier_balances` |
| `pos_app` + `app.is_platform` | **nothing extra.** No platform policy on any of the nine tables; platform support reads a pharmacy's procurement through impersonation, under a real `app.tenant_id` |
| `pos_jobs` | `SELECT` only, on all nine. No job in this module writes (§4) |
| nobody | `UPDATE` or `DELETE` on `supplier_ledger_entries`; `INSERT`/`UPDATE`/`DELETE` on `supplier_balances`; `DELETE` on `purchases`, `supplier_returns`, `suppliers`, `supplier_payments` |

`DELETE` exists on exactly two things: a **draft's lines** (replaced by a whole-document `PUT`,
and a trigger refuses it once the parent is posted) and an **allocation** (a match, not money).
Everything else is cancelled or reversed `[AC-U02]` `[AC-E14]`.

### Historical copies

This is the module where history is frozen, so the list is long and every row of it is load
bearing.

| Column | Frozen at | Why re-deriving it later would be wrong |
|---|---|---|
| `purchase_items.unit_cost` | the bill | It is what the supplier charged on **this** bill. An average cost recomputed later would make the margin on an old sale depend on a purchase made afterwards |
| `purchase_items.unit_mrp` | the bill | The printed price on **this** pack. The batch's MRP may legitimately differ later `[D-38]` `[AC-U16]`, and the bill is not a view of the batch |
| `purchase_items.batch_number`, `.expiry_date` | the bill | The record of what the pack said, kept *even after* `batch_id` is set. "It is the record of what the supplier's bill said, and must not change when the batch is corrected later" |
| `supplier_return_items.unit_cost` | the return | The value credited, agreed with the supplier at that moment. Today's cost for that batch is a different number |
| `supplier_ledger_entries.amount` | the transaction | The **only stored total** in this module. A purchase total is derived from its lines `[AC-U10]`, which is precisely why a posted purchase may never be edited `[AC-U05]` — the derived total would walk away from the frozen one |
| `supplier_payments.amount`, `.method`, `.reference`, `.paid_at` | the payment | What was actually handed over, on what instrument |

**Deliberately not stored**, from the data model's own list: the purchase total
(`Σ quantity × unit_cost + other_charges − invoice_discount`), the effective cost per unit with
free goods, what we owe (that is `supplier_balances`, itself rebuildable), what is unpaid on one
bill, and the unallocated remainder of a payment.

**One subtlety in "what is unpaid on one bill":** it is the bill's total, minus allocations
against it, minus **a return credited against it**. A `supplier_returns` row with
`purchase_id = NULL` lowers the *balance* but lowers no particular bill's outstanding — so a
credit note taken against an unknown bill shows up as an advance, not as a paid invoice. That is
correct and is a thing a reader will otherwise assume is a bug.

**`occurred_at` is the business moment, not `now()`:** the purchase's `invoice_date` at the start
of that day in Asia/Dhaka, the payment's `paid_at` likewise, and the command's `now` for
`opening` and `adjustment`. A running total is order-independent, so backdating costs nothing and
an aged-payables view needs it; insertion order (`id`, UUIDv7) is the tiebreaker.

---

## 2. HTTP surface

All paths under `/api/v1`. Every route is authenticated, every **write** additionally passes
`api`'s access-mode gate (`403 subscription_read_only` while read-only `[AC-W11]`), and every
list is **cursor-paginated** — `ListPage<T>` with `next_cursor` + `has_more`, no total count
(`../00-foundation/CONTRACT.md` §2).

**No route here uses `api::require_at`.** No path carries a branch segment, so
`branch_from_path` would resolve nothing and the check would silently widen to tenant-wide.
Branch scoping is done in the handler against `AuthUser.branch_permissions`
([`SPEC.md`](SPEC.md) §4.2).

### Suppliers — pharmacy-wide

| Method | Path | Permission | Request | Response | Errors |
|---|---|---|---|---|---|
| `GET` | `/suppliers` | `supplier.view` | `q`, `is_active`, `cursor`, `limit` | `ListPage<SupplierSummary>` — **no money column** | — |
| `POST` | `/suppliers` | `supplier.create` | `NewSupplier` (optional client UUIDv7 `id`) | `201 Supplier` · `200` on a repeat with the same id | `409 supplier_name_taken`, `400 validation_failed` |
| `GET` | `/suppliers/{id}` | `supplier.view` | — | `Supplier` | `404 not_found` |
| `PATCH` | `/suppliers/{id}` | `supplier.update` | partial `SupplierPatch` | `200 Supplier` | `404`, `409 supplier_name_taken` |
| `POST` | `/suppliers/{id}/deactivate` | `supplier.update` | — | `200 Supplier` | `404` |
| `POST` | `/suppliers/{id}/reactivate` | `supplier.update` | — | `200 Supplier` | `404`, `409 supplier_name_taken` `[AC-E03]` |

### Purchases — branch-scoped

| Method | Path | Permission | Request | Response | Errors |
|---|---|---|---|---|---|
| `GET` | `/purchases` | `purchase.view` | `branch_id?`, `supplier_id?`, `status?`, `invoice_date_from/to?`, `q?` (invoice number), `cursor`, `limit` | `ListPage<PurchaseSummary>` — filtered to the branches the caller holds the slug at | `403 branch_not_allowed` for an explicit `branch_id` it lacks |
| `POST` | `/purchases` | `purchase.create` | `DraftPurchase` — header **and all lines**, optional client UUIDv7 `id` | `201 Purchase` · `200` on a repeat. Each line may carry `warnings[]` `[AC-S06]` | `409 duplicate_supplier_invoice`, `400 validation_failed`, `404 not_found` (unknown supplier, branch or product), `403 branch_not_allowed`. **No error for an MRP below cost** — it is a warning, not a refusal (Q3) |
| `GET` | `/purchases/{id}` | `purchase.view` | — | `Purchase` — lines, derived total, paid, outstanding, return credits | `404`, `403 branch_not_allowed` |
| `PUT` | `/purchases/{id}` | `purchase.create` | `DraftPurchase` — replaces header **and** line set | `200 Purchase` | `409 purchase_not_draft`, `409 duplicate_supplier_invoice`, `400`, `404`, `403` |
| `POST` | `/purchases/{id}/cancel` | `purchase.cancel` | — | `200 Purchase` | `409 purchase_not_draft` — **a draft only. A posted purchase is never cancelled** (Q2, answered 2026-10-25): goods going back are a supplier return, an entry error is a stock adjustment plus a ledger `adjustment` `[AC-U05]` |

**A line priced below cost is a warning, not an error** (Q3, answered 2026-10-25). `PurchaseLine`
carries `warnings: Vec<LineWarning>`, where `LineWarning` is a plain unit enum — `mrp_below_cost`
first, and the field exists so a later one costs no shape change. The line is saved either way
`[AC-S06]`. The reasoning is in [`SPEC.md`](SPEC.md) §5: a pack really printed below what was
paid for it exists, so a `400` would push the pharmacy into misstating a figure to get a true
bill in, which is worse data than the typo — and silence hides a guaranteed loss until a margin
report. `warnings` is advisory: no handler reads it back, and nothing refuses a post because of
it.

**A draft is one whole document.** There is no `/purchases/{id}/items` sub-resource: the purchase
screen submits a bill, and a line-at-a-time API would be a round trip per row on a counter
connection (`docs/PRODUCT_SPEC.md` §6, the one behavioural thing the data model does not carry).
The rejected alternative — `POST`/`PATCH`/`DELETE` per line — also makes `[AC-W03]`'s
discount check true only momentarily.

**`POST /purchases/{id}/post` is not in this table, and not this module's.** Posting creates a
batch and a stock movement (06) and a supplier ledger entry (05) in one transaction, so it is
`workflows::post_purchase`; by module 09's **Q6** the route lives in `crates/server` with its
request and response structs in `workflows`. What this module provides for it is §3's
`begin_posting` / `complete_posting`, and the permission slugs that route must require:
`purchase.post` **and no second slug** — 06 declares none for a movement
(`../06-inventory/SPEC.md` `[AC-U17]`, 2026-10-25: nobody can post a purchase without stock
arriving, so a second slug would either deny an already-authorised action or be granted by
default and mean nothing).

### Supplier returns — branch-scoped

| Method | Path | Permission | Request | Response | Errors |
|---|---|---|---|---|---|
| `GET` | `/supplier-returns` | `supplier_return.view` | `branch_id?`, `supplier_id?`, `purchase_id?`, `status?`, `reason?`, `cursor`, `limit` | `ListPage<SupplierReturnSummary>` | `403 branch_not_allowed` |
| `POST` | `/supplier-returns` | `supplier_return.create` | `DraftSupplierReturn` — header and lines, optional client UUIDv7 `id` | `201 SupplierReturn` · `200` on a repeat | `400 validation_failed`, `404 not_found`, `403 branch_not_allowed` |
| `GET` | `/supplier-returns/{id}` | `supplier_return.view` | — | `SupplierReturn` with lines and derived credit | `404`, `403` |
| `PUT` | `/supplier-returns/{id}` | `supplier_return.create` | `DraftSupplierReturn` | `200 SupplierReturn` | `409 supplier_return_not_draft`, `400`, `404`, `403` |
| `POST` | `/supplier-returns/{id}/cancel` | `supplier_return.cancel` | — | `200 SupplierReturn` | `409 supplier_return_not_draft` — **a draft only**, as above |

**`POST /supplier-returns/{id}/post` is not in this table, and not this module's.** It writes a
stock movement (06) and a credit ledger entry (05) in one transaction, which makes it a
cross-module flow by module 09's own test. **Q1, answered 2026-10-25: it is `workflows`' seventh
flow**, `post_supplier_return`, and its route is `crates/server`'s with its wire types in
`workflows` (09 Q6). What this module provides is the commands that flow calls — the return's
ledger credit and the posting stamp — in the same shape as `begin_posting` / `complete_posting`
for a purchase. `supplier_returns.status = 'posted'` becomes reachable once 06 Inventory exists
and supplies the reversing movement ([`SPEC.md`](SPEC.md) §3, *Deferred*).

### Payments and allocations — pharmacy-wide `[D-39]`

| Method | Path | Permission | Request | Response | Errors |
|---|---|---|---|---|---|
| `GET` | `/supplier-payments` | `supplier_payment.view` | `supplier_id?`, `branch_id?`, `method?`, `paid_at_from/to?`, `cursor`, `limit` | `ListPage<SupplierPaymentSummary>` with `allocated` and `unallocated` | — |
| `POST` | `/supplier-payments` | `supplier_payment.create` | `NewSupplierPayment` — supplier, optional branch, amount, method, reference, `paid_at`, note, `allocations[]` (**may be empty** = wholly an advance), optional client UUIDv7 `id` | `201 SupplierPayment` · `200` on a repeat | `409 allocation_exceeds_payment`, `409 bill_already_settled`, `409 supplier_mismatch`, `404 not_found`, `400 validation_failed` |
| `GET` | `/supplier-payments/{id}` | `supplier_payment.view` | — | `SupplierPayment` with its allocations and `unallocated` | `404` |
| `POST` | `/supplier-payments/{id}/allocations` | `supplier_payment.allocate` | `allocations[]` | `200 SupplierPayment` — **writes no ledger entry** `[AC-E10]` | `409 allocation_exceeds_payment`, `409 bill_already_settled`, `409 supplier_mismatch`, `404` |
| `DELETE` | `/supplier-payments/{id}/allocations/{purchase_id}` | `supplier_payment.allocate` | — | `200 SupplierPayment` — balance unchanged `[AC-E11]` | `404` |
| `POST` | `/supplier-payments/{id}/cancel` | `supplier_payment.cancel` | — | `200 SupplierPayment`, `status = cancelled`, allocations removed and the balance corrected in one transaction `[AC-S07]` | `409 already_cancelled`, `404` |
| `GET` | `/suppliers/{id}/outstanding-bills` | `supplier_payment.view` | `cursor`, `limit` | `ListPage<OutstandingBill>` — posted purchases with total, allocated, credited, outstanding; outstanding > 0 only | `404` |

**One payment is one transaction over four of this module's tables** — the payment, its
allocations and one ledger entry — and it is therefore **not** a workflow: every table belongs to
this module, so `[D-97]`'s rule sends it to `commands.rs`, called directly from the handler.

**The advance is stored nowhere.** The ledger entry carries the **full** amount, so unallocated
money simply drives the balance down, possibly below zero `[AC-U04]` `[AC-S05]`. Applying it
later adds allocation rows and **no ledger entry** `[AC-E10]`; the three places it is read back
are `unallocated` on a payment, a negative `balance` on the supplier, and the absence of the bill
from `outstanding-bills`. There is no "advance" row, no allocation that points at nothing (the
PK forbids it), and no second ledger entry.

### Ledger and balances — pharmacy-wide, behind their own slug

| Method | Path | Permission | Request | Response | Errors |
|---|---|---|---|---|---|
| `GET` | `/supplier-balances` | `supplier_ledger.view` | `has_balance?`, `cursor`, `limit` | `ListPage<SupplierBalance>` — supplier, `balance` (negative = advance) | — |
| `GET` | `/suppliers/{id}/ledger` | `supplier_ledger.view` | `from?`, `to?`, `type?`, `cursor`, `limit` | `ListPage<LedgerEntry>` + the supplier's current `balance` | `404` |
| `POST` | `/suppliers/{id}/opening-balance` | `supplier_ledger.adjust` | `{ amount, note }` | `201 LedgerEntry` | `409 opening_balance_exists`, `400 validation_failed`, `404` |
| `POST` | `/suppliers/{id}/ledger-adjustments` | `supplier_ledger.adjust` | `{ amount (signed), note (required) }` | `201 LedgerEntry` | `400 validation_failed`, `404` |

**No route updates or deletes a ledger entry**, and none exists to add — a correction is another
`POST /suppliers/{id}/ledger-adjustments` of the opposite sign `[AC-U02]`. **No route writes a
balance.** A per-row running balance is a window function over the whole history and belongs to
`11-reporting`; this list gives amounts plus the current total.

### Permission slugs this module adds

Seeded by **this module's migration** into `permissions`, the table `03-identity` owns, with
`module = 'procurement'` — the pattern `m0003_identity` already uses for `02-tenancy`'s slugs.

| Slug | Branch-scoped | Owner | Manager (shipped template) | Cashier |
|---|---|---|---|---|
| `supplier.view` | no | ✓ | ✓ | — |
| `supplier.create` · `supplier.update` | no | ✓ | ✓ | — |
| `purchase.view` | **yes** | ✓ | ✓ | — |
| `purchase.create` | **yes** | ✓ | ✓ `[D-65]` | — |
| `purchase.post` | **yes** | ✓ | ✓ | — |
| `purchase.cancel` | **yes** | ✓ | — | — |
| `supplier_return.view` · `.create` | **yes** | ✓ | ✓ | — |
| `supplier_return.post` · `.cancel` | **yes** | ✓ | — | — |
| `supplier_payment.view` · `.create` · `.allocate` | no | ✓ | — | — |
| `supplier_ledger.view` | no | ✓ | — | — |
| `supplier_ledger.adjust` | no | ✓ | — | — |

Every slug is `is_tenant_assignable = true`: nothing here is platform-only. **A cashier gets
none** — the deliberate consequence being that the supplier list carries no money column, so a
pharmacy may give `supplier.view` to whoever receives a delivery without showing them what it
owes.

> **Noted while reading, not this module's to fix:** `01-catalog`'s slugs (`catalog.view`,
> `catalog.submit`, `catalog.issue.report`) are declared in its contract but seeded by no
> migration, so they resolve for nobody today. Reported in the handback, not filed here.

### Error cases

| Condition | Status | Code | Message to the user |
|---|---|---|---|
| Another active supplier already has this name | 409 | `supplier_name_taken` | "You already have a supplier with this name." |
| This supplier's bill number is already entered | 409 | `duplicate_supplier_invoice` | "This bill from this supplier is already entered. Open it instead, or cancel the draft you started earlier." |
| Editing, posting or re-posting something not a draft | 409 | `purchase_not_draft` / `supplier_return_not_draft` | "This purchase has already been posted and cannot be changed. To correct it, return the goods to the supplier or adjust the stock." |
| Posting a purchase with no lines | 409 | `purchase_empty` | "Add at least one item before posting this purchase." |
| Cancelling a posted purchase | 409 | `purchase_not_cancellable` | "Only a draft can be cancelled." — **provisional, pending Q2** |
| Allocations exceed the payment | 409 | `allocation_exceeds_payment` | "You are matching more than this payment is worth. Lower the amounts, or leave the rest as an advance." |
| Allocation would overpay a bill | 409 | `bill_already_settled` | "That bill is already fully paid. Pick another bill, or leave the money as an advance." |
| Payment and bill belong to different suppliers | 409 | `supplier_mismatch` | "That bill belongs to a different supplier." |
| A second opening balance | 409 | `opening_balance_exists` | "This supplier already has an opening balance. Record a correction instead." |
| Discount larger than the bill, zero or negative quantity or cost, missing batch number or expiry on a tracked product, expiry on or before the invoice date, adjustment with no note | 400 | `validation_failed` | Plain language naming the field, in `details.fields` |
| A purchase or return at a branch the caller has no permission at | 403 | `branch_not_allowed` | "You do not have access to that branch." `details.required` names the slug |
| Another **pharmacy's** row, or an unknown id | 404 | `not_found` | "Not found." — never 403, which would confirm the id exists |
| A write while the subscription is read-only | 403 | `subscription_read_only` | Raised by the route before any command runs `[AC-W11]`. Reads still succeed |
| Return line exceeding what is on hand | 409 | **06's `insufficient_stock`** | 06's sentence. Deferred — this module invents no code for it `[AC-W12]` |
| Anything unexpected | 500 | `internal_error` | The transaction rolls back whole; the detail goes to the log with the trace id |

`duplicate_supplier_invoice` is already reserved in `../00-foundation/CONTRACT.md` §2. **It is
raised by `POST /purchases` and `PUT /purchases/{id}` and never by the posting flow** — because
the unique index covers drafts, two drafts for one bill cannot coexist, so by the time
`workflows::post_purchase` runs the duplicate has already been refused. That is this module's
answer to the "supplier-invoice uniqueness rule" module 09 `SPEC.md` §3 asked for.

---

## 3. Public Rust surface

`crates/procurement` depends on `core`, `db`, `api`, and on **`catalog`** for one read
(`[D-8]`'s batch-tracked fact — see *What this module needs*). It depends on **no other module
crate** and nothing depends on it except `server`, `workflows` and `reporting` `[D-97]`.
`repo.rs` and `entity/` are private.

```rust
// queries.rs — every function generic over C: ConnectionTrait (09 Q3, amending [D-98]),
//              so workflows::post_purchase reads inside its own transaction.

pub async fn suppliers<C: ConnectionTrait>(c: &C, ctx: &TenantContext, f: SupplierFilter, p: Cursor)
    -> Result<ListPage<SupplierSummary>, AppError>;
pub async fn supplier<C: ConnectionTrait>(c: &C, ctx: &TenantContext, id: SupplierId)
    -> Result<Supplier, AppError>;

pub async fn purchases<C: ConnectionTrait>(c: &C, ctx: &TenantContext, f: PurchaseFilter, p: Cursor)
    -> Result<ListPage<PurchaseSummary>, AppError>;
pub async fn purchase<C: ConnectionTrait>(c: &C, ctx: &TenantContext, id: PurchaseId)
    -> Result<Purchase, AppError>;                       // header + lines + derived figures

pub async fn supplier_returns<C: ConnectionTrait>(…) -> Result<ListPage<SupplierReturnSummary>, AppError>;
pub async fn supplier_return<C: ConnectionTrait>(…)  -> Result<SupplierReturn, AppError>;

pub async fn supplier_payments<C: ConnectionTrait>(…) -> Result<ListPage<SupplierPaymentSummary>, AppError>;
pub async fn supplier_payment<C: ConnectionTrait>(…)  -> Result<SupplierPayment, AppError>;

pub async fn supplier_balance<C: ConnectionTrait>(c: &C, ctx: &TenantContext, s: SupplierId)
    -> Result<Decimal, AppError>;                        // negative = advance held [AC-S05]
pub async fn supplier_balances<C: ConnectionTrait>(…) -> Result<ListPage<SupplierBalance>, AppError>;
pub async fn supplier_ledger<C: ConnectionTrait>(…)   -> Result<ListPage<LedgerEntry>, AppError>;

pub async fn outstanding_bills<C: ConnectionTrait>(c: &C, ctx: &TenantContext, s: SupplierId, p: Cursor)
    -> Result<ListPage<OutstandingBill>, AppError>;

// --- locking reads. FOR UPDATE, and therefore generic, by 09 [AC-U10] / [AC-U15] ---
pub async fn purchase_for_update<C: ConnectionTrait>(c: &C, ctx: &TenantContext, id: PurchaseId)
    -> Result<Purchase, AppError>;
pub async fn outstanding_bills_for_update<C: ConnectionTrait>(
    c: &C, ctx: &TenantContext, s: SupplierId, bills: &[PurchaseId])
    -> Result<Vec<OutstandingBill>, AppError>;           // decides [AC-W07]
pub async fn payment_for_update<C: ConnectionTrait>(c: &C, ctx: &TenantContext, id: PaymentId)
    -> Result<SupplierPayment, AppError>;                // decides [AC-W06]
```

```rust
// commands.rs — take &Tx. Single-module writes, called directly from the handler [D-97].

pub async fn create_supplier(tx: &Tx, ctx: &TenantContext, input: NewSupplier)   -> Result<Supplier, AppError>;
pub async fn update_supplier(tx: &Tx, ctx: &TenantContext, id: SupplierId, patch: SupplierPatch)
    -> Result<Supplier, AppError>;
pub async fn set_supplier_active(tx: &Tx, ctx: &TenantContext, id: SupplierId, active: bool)
    -> Result<Supplier, AppError>;

pub async fn save_purchase_draft(tx: &Tx, ctx: &TenantContext, input: DraftPurchase)
    -> Result<Purchase, AppError>;                       // create or replace, header + lines, one Tx
pub async fn cancel_purchase_draft(tx: &Tx, ctx: &TenantContext, id: PurchaseId) -> Result<Purchase, AppError>;

pub async fn save_supplier_return_draft(tx: &Tx, ctx: &TenantContext, input: DraftSupplierReturn)
    -> Result<SupplierReturn, AppError>;
pub async fn cancel_supplier_return_draft(tx: &Tx, ctx: &TenantContext, id: SupplierReturnId)
    -> Result<SupplierReturn, AppError>;

pub async fn record_payment(tx: &Tx, ctx: &TenantContext, input: NewSupplierPayment)
    -> Result<SupplierPayment, AppError>;                // payment + allocations + ONE ledger entry [AC-E09]
pub async fn allocate_payment(tx: &Tx, ctx: &TenantContext, id: PaymentId, allocs: Vec<Allocation>)
    -> Result<SupplierPayment, AppError>;                // allocations only, NO ledger entry [AC-E10]
pub async fn deallocate_payment(tx: &Tx, ctx: &TenantContext, id: PaymentId, bill: PurchaseId)
    -> Result<SupplierPayment, AppError>;                // [AC-E11]

pub async fn record_opening_balance(tx: &Tx, ctx: &TenantContext, s: SupplierId, amount: Decimal, note: String)
    -> Result<LedgerEntry, AppError>;                    // at most one per supplier [AC-E12]
pub async fn record_adjustment(tx: &Tx, ctx: &TenantContext, s: SupplierId, amount: Decimal, note: String)
    -> Result<LedgerEntry, AppError>;                    // the ONLY correction path [AC-U02]
```

### What `workflows::post_purchase` calls — the handoff module 09 §3 asked for

Two commands and a plan type. The division of labour is the one 09 `[AC-U09]` requires: **this
module does every piece of arithmetic**, the workflow passes frozen values through untouched, and
06 writes the batch and the movement.

```rust
pub enum Posting {
    Ready(PurchasePosting),            // the plan: lock held, preconditions passed
    AlreadyPosted(PostedPurchase),     // 09 [AC-E02] — a replay. Nothing written
}

pub struct PurchasePosting {
    pub purchase_id: PurchaseId,
    pub branch_id: BranchId,
    pub supplier_id: SupplierId,
    pub invoice_date: NaiveDate,
    pub invoice_total: Decimal,        // Σ(quantity × unit_cost) + other_charges − invoice_discount
    pub occurred_at: DateTime<Utc>,    // invoice_date, start of day Asia/Dhaka
    pub lines: Vec<PostingLine>,
}

pub struct PostingLine {
    pub purchase_item_id: PurchaseItemId,
    pub product_id: ProductId,
    pub batch_number: Option<String>,  // [D-8]
    pub expiry_date: Option<NaiveDate>,
    pub units: i32,                    // quantity + free_quantity — what 06 moves  [AC-U08]
    pub line_cost_total: Decimal,      // quantity × unit_cost — what is owed       [AC-U08]
    pub unit_mrp: Decimal,             // the bill's printed price. 06 must NOT overwrite
}                                      // an existing batch's MRP with it  [D-38] [D-10] [AC-U16]

/// Locks the purchase and its lines FOR UPDATE, validates every posting precondition,
/// and returns the plan — or AlreadyPosted, read from the primary key and status
/// inside the caller's transaction.  [AC-E06] [AC-E07]
pub async fn begin_posting(tx: &Tx, ctx: &TenantContext, id: PurchaseId)
    -> Result<Posting, AppError>;

/// Stamps each line's (batch_id, stock_movement_id) — returned by 06 — THEN sets
/// status = 'posted' and posted_at, inserts exactly one `purchase` ledger entry for
/// invoice_total, and stores its id in purchases.ledger_entry_id.  [AC-E08]
pub async fn complete_posting(
    tx: &Tx, ctx: &TenantContext, plan: &PurchasePosting, links: Vec<PostedLine>)
    -> Result<PostedPurchase, AppError>;

pub struct PostedLine { pub purchase_item_id: PurchaseItemId,
                        pub batch_id: BatchId, pub stock_movement_id: StockMovementId }
```

**Note `line_cost_total` + `units` rather than a per-unit cost.** `10 × 8.50 ÷ 11 = 7.7272…`
does not fit `numeric(12,2)`, and `7.73 × 11 = 85.03` against `85.00` owed. Handing 06 the exact
total and the unit count keeps inventory valuation reconciled to the supplier ledger to the
paisa. This is a **proposal to 06**, recorded in [`SPEC.md`](SPEC.md) §3 *Deferred*; what is
settled is that this module never emits a rounded per-unit cost `[AC-U08]`.

`perms.rs` exports every slug as a `&'static str` const, the way `catalog::perms` does, so
`crates/server`'s posting route can require `purchase.post` without re-typing the string.

**Called by:** `crates/server` (mounts the `OpenApiRouter`, and owns the posting route — 09 Q6)
· `workflows` (`post_purchase`, through `begin_posting` / `complete_posting`) · `11-reporting`
(the purchase VAT register `[D-69]` and the payables reports, reading these tables with
`query_as!` `[D-99]`). **By no other module crate** `[D-97]`.

### What this module needs from another

| Need | From | Status |
|---|---|---|
| A batch create-or-find + stock-in command, the cost-basis shape, batch uniqueness, an existing batch's MRP, the on-hand `FOR UPDATE` read, `insufficient_stock`, and the reversing movement | **06 Inventory** | **Blocked — Phase 0.** Named field by field in [`SPEC.md`](SPEC.md) §3 *Deferred*. No column, command or code is guessed here |
| "Is this product batch-tracked" `[D-8]`, as a read **generic over `C: ConnectionTrait`** | **01 Catalogue** (or 06 — part of the deferral) | **Gap.** `catalog`'s reads all take `&Db` today; module 09's Q3 already records that `catalog` has no generic reads and needs a backlog item of its own. Not this module's to file |
| A seventh `workflows` flow for posting a supplier return | **09 Workflows** / a `[D-97]` amendment | **Blocked —** [`SPEC.md`](SPEC.md) **Q1** |
| `AuthUser::branches_with(slug) -> Vec<BranchId>` | **00 Foundation** (`api`) | **Convenience, not a gap.** `branch_permissions` is already on `AuthUser`; the helper stops ten handlers re-deriving the same set, and `require_at` cannot serve these routes ([`SPEC.md`](SPEC.md) §4.2) |
| Nothing else | — | `Tx::begin`, `TenantTx`, `api::require`, the access-mode gate, `ListPage`, `Cursor`, `AppError` all exist |

---

## 4. Events and jobs

| Trigger | Job | Schedule | Idempotent? |
|---|---|---|---|
| — | **none** | — | — |

This module schedules nothing and is woken by nothing. Expiry and near-expiry sweeps `[D-5]` are
**06's**; a payables ageing view is a report, not a job; supplier payment reminders are not in
v1. `pos_jobs` therefore holds `SELECT` only on all nine tables (§1, *Grants*) — a job that could
write a ledger entry under `BYPASSRLS` is a risk with no feature behind it.

---

## 5. Offline and sync

| Question | Answer |
|---|---|
| Can the mobile app create this offline? | **No, and it cannot create it online either.** Purchases, returns and payments are online only (Data Model Context 5, opening line) and *"nothing here reaches a phone"*. `[D-24]`'s offline scope is sales; `10-sync` replays `sell` alone (09 Q4) |
| Client-generated id? | **UUIDv7, server-generated.** [`../rules/tenancy.md`](../rules/tenancy.md) rule 10 ties client generation to offline creation and there is none here. `POST /purchases`, `POST /supplier-returns` and `POST /supplier-payments` **accept** a caller-supplied UUIDv7 `id` so a double-tapped admin screen cannot pay a supplier twice — a repeat answers `200` with the existing row, the idiom `../00-foundation/CONTRACT.md` §2 documents |
| What makes an upload idempotent | Nothing is uploaded. For the one flow that touches this module, `workflows::post_purchase`, the idempotency key is the **existing draft purchase's primary key**, checked under `FOR UPDATE` inside the caller's transaction: already `posted` → `Posting::AlreadyPosted` and nothing written `[AC-E07]`, 09 `[AC-E02]`. Not a Redis key — a purchase can be posted hours after it is drafted |
| Can it be flagged rather than rejected? | **No. Everything rejects.** The four flag cases in 09 `[AC-S02]` are all sale-shaped — oversell, over-limit credit, a batch expired between sale and upload, a price changed server-side — and none applies to a purchase that someone is typing in front of the delivery. There is no flag column in any of the nine tables |
| Clock | `invoice_date` and `paid_at` are calendar dates in Asia/Dhaka, supplied by the caller. `posted_at` and `occurred_at` are `timestamptz`; `occurred_at` carries the **business moment** (§1, *Historical copies*), never `now()` for a backdated bill |
