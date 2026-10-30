# CONTRACT: 06 — Inventory

**Status:** Draft — Gate 1
**Crate:** `crates/inventory`
**Spec:** [`SPEC.md`](SPEC.md)
**Bounded context:** Context 6 — Inventory

> Gate 1 artifact. The tables this module owns and the HTTP surface it exposes, copied from
> Data Model Context 6 and the API Design Guide — this is the module's slice of them, not a
> second version. Where they disagree, they win and this is corrected; the three places this
> contract knowingly departs from the page are listed in [`SPEC.md`](SPEC.md) §5 with their
> reasoning, flagged for amendment rather than edited.
>
> The envelope, the error shape, the headers, cursor pagination and the shared error codes are
> [`../00-foundation/CONTRACT.md`](../00-foundation/CONTRACT.md) §2 and are **not restated**.
>
> **Six routes a reader will look for are deliberately absent**, and §2 says where each went:
> the sale, the customer return, the purchase posting, the supplier-return posting, the void
> and — provisionally, pending [`SPEC.md`](SPEC.md) §5 Q4 — the branch transfer all belong to
> `crates/server` with their wire types in `workflows` (09 Q6).

---

## 1. Tables owned

Eleven tables, **all tenant-owned**, all with `tenant_id`, `ENABLE` + `FORCE ROW LEVEL
SECURITY` and a tenant policy. No shared table and no platform-owned table — `batch_mrp_change_requests`
is reviewed by platform staff but owned by the pharmacy, and carries a **second** policy for
`app.is_platform`, the shape `02-tenancy` uses for its billing tables.

| Table | Keys | Tenant-scoped uniques | Append-only | Notes |
|---|---|---|---|---|
| `batches` | `id` PK, `(tenant_id, id)` UK, **`(tenant_id, branch_id, id)` UK** | `(tenant_id, branch_id, product_id, batch_number)` **`NULLS NOT DISTINCT`** | no | `branch_id`, `product_id`, `batch_number`, `expiry_date`, `unit_mrp` (all three nullable together `[D-8]`), `status` ∈ `active \| quarantined`, `quarantine_reason`. **No cost column** — see *Historical copies* and [`SPEC.md`](SPEC.md) §3. The second UK exists so `stock_movements` can reference `(tenant_id, branch_id, id)` and the copied branch cannot be wrong |
| `stock_movements` | `id` PK, `(tenant_id, id)` UK | — | **YES** | `branch_id` (copied from the batch), `batch_id`, `type` ∈ `purchase_in \| purchase_return_out \| sale_out \| sale_return_in \| transfer_out \| transfer_in \| adjustment`, `quantity` (signed base units), **`cost_total`** (signed, `numeric(12,2)`), `user_id`, `device_id` nullable, `occurred_at`, `recorded_at`. `id` **may be the phone's UUIDv7** for an offline sale (§5) |
| `stock_balances` | `(tenant_id, batch_id)` PK | the key | no (but **unwritable**) | `branch_id`, `quantity`, **`cost_basis_total`**, **`cost_basis_units`**. Three running totals of one ledger. `pos_app` holds **`SELECT` only**; the trigger is the only writer `[AC-U03]` |
| `batch_mrp_change_requests` | `id` PK, `(tenant_id, id)` UK | `(tenant_id, batch_id)` **where `status = 'pending'`** | no | `branch_id`, `batch_id`, `current_unit_mrp` (historical copy), `requested_unit_mrp`, `reason`, `requested_by_user_id`, `status` ∈ `pending \| approved \| rejected`, `reviewed_by_user_id` → `platform_users`, `reviewed_at`, `review_note`. Tenant role: `SELECT`, `INSERT` only `[D-38]` |
| `reorder_levels` | `(tenant_id, branch_id, product_id)` PK | the key | no | `reorder_level` ≥ 0. **No row means no alert**, which is why `DELETE` is granted here and almost nowhere else |
| `stock_adjustments` | `id` PK, `(tenant_id, id)` UK | — | **effectively** | `branch_id`, `reason` ∈ `stock_count \| damaged \| expired \| lost \| correction`, `stock_count_id` nullable, `note`, `created_by_user_id`. `pos_app` holds `SELECT`, `INSERT` only: an adjustment is the correction mechanism, so correcting one is another adjustment |
| `stock_adjustment_items` | `(tenant_id, adjustment_id, batch_id)` PK | the key · `(tenant_id, stock_movement_id)` | **effectively** | `quantity` (positive = found, negative = removed), `stock_movement_id`. One line per batch per adjustment, by the key |
| `stock_counts` | `id` PK, `(tenant_id, id)` UK | — | no | `branch_id`, `status` ∈ `open \| completed \| cancelled`, `started_by_user_id`, `completed_by_user_id`, `completed_at`. **No `note` column** — Context 6 gives one to `stock_adjustments` and `branch_transfers` and not to a count, and the adjustment a count produces is where its explanation belongs |
| `stock_count_items` | `(tenant_id, stock_count_id, batch_id)` PK | the key | no | `system_quantity` (**historical copy**), `counted_quantity`. Writable while the count is `open` and refused by trigger after |
| `branch_transfers` | `id` PK, `(tenant_id, id)` UK | — | **effectively** | `from_branch_id`, `to_branch_id` (`CHECK` they differ), `created_by_user_id`, `note`. `SELECT`, `INSERT` only — a transfer is instant `[D-36]`, so there is no state to update |
| `branch_transfer_items` | `(tenant_id, transfer_id, from_batch_id)` PK | the key · `(tenant_id, out_movement_id)` · `(tenant_id, in_movement_id)` | **effectively** | `to_batch_id`, `quantity` (positive magnitude), `out_movement_id`, `in_movement_id` |

**"Effectively append-only"** means `pos_app` holds no `UPDATE` and no `DELETE`, so the rows
cannot change — but unlike `stock_movements` they are not *ledgers*, and
[`../rules/tenancy.md`](../rules/tenancy.md) rule 7 names only the four ledgers. The distinction
matters for one reason: a test asserting rule 7 should assert it of `stock_movements`, and a test
asserting these is asserting a grant this contract chose.

Every `status`, `type` and `reason` is **`VARCHAR` + `CHECK`**, not a Postgres enum — the
convention `01-catalog`, `02-tenancy` and `05-procurement` already use, so adding a value is a
migration and not a type rewrite.

### Keys and constraints specific to this module

**Tenant-scoped uniques**

| Index | Why |
|---|---|
| `UNIQUE (tenant_id, branch_id, product_id, batch_number) NULLS NOT DISTINCT` on `batches` | The page's own constraint. It is what makes "create or find" a single statement, what makes a second bill for the same batch number land in the same batch, and — through `NULLS NOT DISTINCT` — what gives each branch **exactly one** untracked batch per product `[D-8]` `[AC-U13]` |
| `UNIQUE (tenant_id, branch_id, id)` on `batches` | Not uniqueness for its own sake: it is the target of `stock_movements`' composite FK, which is how the copied `branch_id` is kept honest `[AC-U10]` |
| `UNIQUE (tenant_id, batch_id) WHERE status = 'pending'` on `batch_mrp_change_requests` | *"At most one pending MRP request per batch"*. It is also why [`SPEC.md`](SPEC.md) §5 Q5 exists — a pharmacy that asked wrongly cannot ask again until a reviewer acts |
| `UNIQUE (tenant_id, stock_movement_id)` on `stock_adjustment_items`; `UNIQUE (tenant_id, out_movement_id)` and `UNIQUE (tenant_id, in_movement_id)` on `branch_transfer_items` | One movement has at most one source row. Without them two adjustment items could claim one movement, and the ledger would no longer explain itself. The `UK` marks are in the export's ERD, not in Notion's extract ([`SPEC.md`](SPEC.md) §5) |

**Deliberately not unique:** there is **no** constraint stopping two `open` stock counts at one
branch. Context 6's constraint list is itemised and names none, and ends *"Nothing open here"*.
Two overlapping counts would each freeze their own `system_quantity` `[AC-E10]` and each produce
their own adjustment, which is wrong but is wrong in a way the data explains. A partial unique
index on `(tenant_id, branch_id) WHERE status = 'open'` is what a reviewer may well want; that is
a Notion edit and a decision, not something this contract may add on its own authority.

**Composite foreign keys** — `(tenant_id, …)` throughout, Art. IV.3:

| From | To | Owner |
|---|---|---|
| `batches (tenant_id, branch_id)` · `stock_movements` · `stock_balances` · `reorder_levels` · `stock_adjustments` · `stock_counts` · `batch_mrp_change_requests` · `branch_transfers (tenant_id, from_branch_id)` and `(tenant_id, to_branch_id)` | `branches (tenant_id, id)` | 02 |
| `stock_movements (tenant_id, branch_id, batch_id)` | **`batches (tenant_id, branch_id, id)`** — the three-column reference, not the two-column one `[AC-U10]` | 06 |
| `stock_balances (tenant_id, batch_id)` · `stock_adjustment_items` · `stock_count_items` · `batch_mrp_change_requests` · `branch_transfer_items (tenant_id, from_batch_id)` and `(tenant_id, to_batch_id)` | `batches (tenant_id, id)` | 06 |
| `stock_adjustment_items (tenant_id, adjustment_id)` | `stock_adjustments (tenant_id, id)` | 06 |
| `stock_count_items (tenant_id, stock_count_id)` · `stock_adjustments (tenant_id, stock_count_id)` nullable | `stock_counts (tenant_id, id)` | 06 |
| `branch_transfer_items (tenant_id, transfer_id)` | `branch_transfers (tenant_id, id)` | 06 |
| `stock_adjustment_items (tenant_id, stock_movement_id)` · `branch_transfer_items (tenant_id, out_movement_id)` and `(tenant_id, in_movement_id)` | `stock_movements (tenant_id, id)` | 06 |
| `batches.product_id` · `reorder_levels.product_id` | `products (id)` — **plain, not composite**, because `products` has no `tenant_id`. `reorder_levels` is [`SPEC.md`](SPEC.md) §5 **Q2** | 01 |
| `stock_movements.user_id` · `device_id` · `stock_adjustments.created_by_user_id` · `stock_counts.started_by_user_id` / `completed_by_user_id` · `branch_transfers.created_by_user_id` · `batch_mrp_change_requests.requested_by_user_id` | `users (tenant_id, id)`, `devices (tenant_id, id)` | 03 |
| `batch_mrp_change_requests.reviewed_by_user_id` | `platform_users (id)` — **plain**, because `platform_users` is not tenant-owned and *"never readable in a tenant context"* | 03 |

**Check constraints** — each in the database because each is a single-row fact:

| Constraint | Table | Why in the database |
|---|---|---|
| `CHECK ((batch_number IS NULL) = (expiry_date IS NULL) AND (batch_number IS NULL) = (unit_mrp IS NULL))` | `batches` | *"`batch_number`, `expiry_date` and `unit_mrp` are either all set or all null"* `[D-8]` `[AC-U13]` |
| `CHECK (unit_mrp IS NULL OR unit_mrp > 0)` | `batches` | A zero selling price is not a price |
| `CHECK (status IN ('active','quarantined'))` | `batches` | Enum discipline without a Postgres enum |
| `CHECK (status <> 'quarantined' OR quarantine_reason IS NOT NULL)` | `batches` | **This contract's addition**, not the page's: a batch taken out of selling with no reason recorded is a decision nobody can review. Flagged as such because it is a constraint the data model does not state |
| `CHECK (quantity <> 0)` | `stock_movements` | *"`quantity <> 0`"*. A movement of nothing still fires the trigger and still claims a source row |
| `CHECK (CASE WHEN type IN ('purchase_in','sale_return_in','transfer_in') THEN quantity > 0 WHEN type IN ('purchase_return_out','sale_out','transfer_out') THEN quantity < 0 ELSE TRUE END)` | `stock_movements` | *"with the sign matching the type (an adjustment may be either way)"* `[AC-W07]` |
| `CHECK (sign(cost_total) = sign(quantity) OR cost_total = 0)` | `stock_movements` | Money and units leave in the same direction. `0` is allowed for a found-stock adjustment on a batch with no basis yet |
| **No check on `stock_balances.quantity`** | `stock_balances` | Stated as a constraint by its absence: the balance **may go negative** `[D-25]` `[AC-U04]`, and a future `CHECK (quantity >= 0)` would reject a real, already-committed oversell on replay |
| `CHECK (quantity <> 0)` | `stock_adjustment_items` | An adjustment line of zero is a line that does nothing |
| `CHECK (quantity > 0)` | `branch_transfer_items`, `stock_count_items` (`counted_quantity >= 0`, `system_quantity` unconstrained) | A transfer has a direction already; a count may legitimately find zero, and the system quantity may be negative after an oversell |
| `CHECK (from_branch_id <> to_branch_id)` | `branch_transfers` | *"source and target differ"* `[AC-W04]` |
| `CHECK (reorder_level >= 0)` | `reorder_levels` | Zero is meaningful — *alert whenever it runs out* — and negative is not |
| `CHECK (requested_unit_mrp > 0 AND requested_unit_mrp <> current_unit_mrp)` | `batch_mrp_change_requests` | A request for the price it already has is a no-op that would still need reviewing `[AC-W10]` |
| `CHECK ((status = 'pending') = (reviewed_by_user_id IS NULL AND reviewed_at IS NULL))` | `batch_mrp_change_requests` | Two columns that must agree with a third, written by one command and read by two panels |
| `CHECK ((status = 'completed') = (completed_at IS NOT NULL AND completed_by_user_id IS NOT NULL))` | `stock_counts` | As above `[AC-E09]` |
| `CHECK (reason IN (…))`, `CHECK (status IN (…))`, `CHECK (type IN (…))` | each | Enum discipline |

**Four rules that cannot be check constraints, and what carries them instead:**

| Rule | Mechanism | Why not a `CHECK` |
|---|---|---|
| A product that is not `non_medicine` must use a **tracked** batch `[D-8]` | **Rust, in `find_or_create_batch`**, through a named read of 01's `products.product_type` `[AC-W03]` | The fact lives in 01's shared `products` table, and a trigger joining across a bounded context is the coupling `[D-97]` removes — the page says the same: *"checked in the service, since it spans two tables"* |
| A transfer's target batch must match the source's product, batch number and expiry | A trigger on `branch_transfer_items` (`BEFORE INSERT`) comparing the two batch rows `[AC-W05]` | It spans two rows of `batches`. The command creates the target itself, so the trigger is a backstop against a future caller |
| No write to a `stock_count_items` row whose parent is not `open` | A trigger on `stock_count_items` (`BEFORE INSERT OR UPDATE OR DELETE`) refusing it `[AC-S04]` | The state is on the parent row — the same shape as 05's `trg_purchase_item_frozen` |
| A movement's `occurred_at` may not precede its batch's creation, and a `sale_out` may not predate the stock arriving | **Nothing.** Deliberately unconstrained | A phone's clock can be wrong and the sale still happened `[D-24]`. Rejecting it would discard a transaction that is already in the customer's hand, and a running total is order-independent so nothing downstream needs the ordering to hold |

**Triggers**

| Trigger | On | Does |
|---|---|---|
| `trg_stock_movement_balance` | `AFTER INSERT ON stock_movements FOR EACH ROW` | `INSERT INTO stock_balances (tenant_id, batch_id, branch_id, quantity, cost_basis_total, cost_basis_units) VALUES (…) ON CONFLICT (tenant_id, batch_id) DO UPDATE SET quantity = stock_balances.quantity + NEW.quantity, cost_basis_total = stock_balances.cost_basis_total + <basis delta>, cost_basis_units = stock_balances.cost_basis_units + <basis delta>`, where the basis delta is `NEW.cost_total` / `NEW.quantity` **only when `NEW.type IN ('purchase_in','transfer_in')`** and zero otherwise ([`SPEC.md`](SPEC.md) §3). **`SECURITY DEFINER` with `SET search_path = pg_catalog, public`**, so `pos_app` needs no write grant on `stock_balances` and no code path can set a figure the ledger does not say `[AC-U03]`. It writes exactly `NEW.tenant_id`, so definer rights widen nothing |
| `trg_batch_balance_row` | `AFTER INSERT ON batches FOR EACH ROW` | Inserts the zero balance row, so `batches ||--|| stock_balances` is structurally true and every read can inner-join `[AC-E02]`. Also `SECURITY DEFINER`, for the same reason |
| `trg_transfer_item_match` | `BEFORE INSERT ON branch_transfer_items` | The product / batch number / expiry match above `[AC-W05]` |
| `trg_stock_count_item_frozen` | `BEFORE INSERT OR UPDATE OR DELETE ON stock_count_items` | Refuses when the parent count is not `open` `[AC-S04]` |

**There is no trigger that recomputes a balance**, and no Rust function that does either. The
rebuild statement —

```sql
SELECT batch_id,
       SUM(quantity)                                                        AS quantity,
       SUM(CASE WHEN type IN ('purchase_in','transfer_in') THEN cost_total  ELSE 0 END) AS cost_basis_total,
       SUM(CASE WHEN type IN ('purchase_in','transfer_in') THEN quantity    ELSE 0 END) AS cost_basis_units
FROM stock_movements GROUP BY 1
```

— compared against `stock_balances`, is a **test and a reconciliation**, never a repair the
application runs `[AC-U03]`.

**Indexes the queries actually need** (every one leads with `tenant_id`, with one stated
exception):

| Index | Query it serves |
|---|---|
| `batches (tenant_id, branch_id, product_id, batch_number)` | the unique above — create-or-find, and 05's batch lookup for the `[D-10]` prompt |
| `batches (tenant_id, branch_id, expiry_date)` | earliest-expiry ordering `[D-4]`, the near-expiry and expired lists `[D-5]`, and the stock list's default sort |
| `batches (tenant_id, product_id)` | "which branches hold this product", and 01's `[D-2]` merge move |
| `stock_movements (tenant_id, batch_id, occurred_at DESC, id DESC)` | a batch's ledger, the only screen that reads a movement by itself |
| `stock_movements (tenant_id, branch_id, occurred_at DESC, id DESC)` | the branch movement list, cursor-paginated, and 11's movement reports |
| `stock_movements (tenant_id, branch_id, type, occurred_at)` | "all the write-offs this month", and the cost-of-goods figures 11 sums |
| `stock_balances (tenant_id, branch_id, batch_id)` | branch valuation and the low-stock sum; the PK serves the per-batch read |
| `stock_adjustments (tenant_id, branch_id, id DESC)` · `(tenant_id, stock_count_id)` | the adjustment list; the adjustment a count produced |
| `stock_adjustment_items (tenant_id, batch_id)` | a batch's adjustment history, and `sync_flags`' resolution link (§5 Q1) |
| `stock_counts (tenant_id, branch_id, id DESC)` · `(tenant_id, branch_id) WHERE status = 'open'` | the count list; "is a count already running here" |
| `branch_transfers (tenant_id, from_branch_id, id DESC)` · `(tenant_id, to_branch_id, id DESC)` | **two indexes, not one**: *"a transfer is visible if either of its branches is"* |
| `branch_transfer_items (tenant_id, from_batch_id)` · `(tenant_id, to_batch_id)` | where a batch's stock went, and where a batch came from |
| `reorder_levels` PK `(tenant_id, branch_id, product_id)` | the whole table is its index |
| `batch_mrp_change_requests (tenant_id, status, id DESC)` | the pharmacy's own request list |
| **`batch_mrp_change_requests (status, id DESC)` — no `tenant_id` lead** | the **platform review queue** `[D-38]`, which spans every pharmacy. The stated exception to *"indexes lead with `tenant_id`"*: a leading `tenant_id` would make the one query that must scan all tenants scan all tenants' index too |

### Grants

| Role | Rights |
|---|---|
| `pos_app` | **`SELECT, INSERT`** on `stock_movements` · **`SELECT` only** on `stock_balances` · `SELECT, INSERT` on `batches` plus **`UPDATE (status, quarantine_reason, updated_at)`** and nothing more `[D-38]` `[AC-U05]` · `SELECT, INSERT` on `stock_adjustments`, `stock_adjustment_items`, `branch_transfers`, `branch_transfer_items`, `batch_mrp_change_requests` · `SELECT, INSERT, UPDATE` on `stock_counts` · `SELECT, INSERT, UPDATE, DELETE` on `stock_count_items` (a line removed from an open count) and on `reorder_levels` (**no row means no alert**) |
| `pos_app` + `app.is_platform` | the same, plus **`UPDATE (unit_mrp, updated_at)` on `batches`** and `UPDATE (status, reviewed_by_user_id, reviewed_at, review_note)` on `batch_mrp_change_requests`, under a second policy that does not filter by `tenant_id` — the review queue, and nothing else `[D-38]` `[AC-E13]` |
| `pos_jobs` | **`SELECT` only, on all eleven.** The near-expiry and low-stock sweeps read; the `notifications` rows they write belong to another module (§4). A job that could insert a movement under `BYPASSRLS` is a risk with no feature behind it |
| nobody | `UPDATE` or `DELETE` on `stock_movements` · `INSERT`, `UPDATE` or `DELETE` on `stock_balances` · `UPDATE` on any other column of `batches` · `UPDATE` or `DELETE` on `stock_adjustments`, `stock_adjustment_items`, `branch_transfers`, `branch_transfer_items` · `DELETE` on `batches`, `stock_counts` or `batch_mrp_change_requests` |

`DELETE` exists on exactly two things: a **reorder level**, because its absence is the feature,
and an **open count's item**, because a counter may remove a line before completing. Everything
else is corrected by a new movement `[AC-U02]`.

**Two grant facts that are load bearing and easy to lose in a refactor:**

1. **The `UPDATE` on `batches` is column-level.** `GRANT UPDATE (status, quarantine_reason,
   updated_at) ON batches TO pos_app`. A future `GRANT UPDATE ON batches` would silently undo
   `[D-38]`, so the assertion is textual as well as behavioural.
2. **`pos_app` cannot row-lock `stock_balances`.** PostgreSQL requires `UPDATE` privilege on at
   least one column for `SELECT … FOR UPDATE`, and `pos_app` has none here, so the lock is taken
   on the `batches` row instead `[AC-W12]` ([`SPEC.md`](SPEC.md) §3, *Why the lock is on the
   batch*). This is a correction to the Notion page, flagged in [`SPEC.md`](SPEC.md) §5.

### Historical copies

This module freezes four figures, and the reason is the same each time: a later purchase, a
later sale or a later price change must not reach backwards and restate what was true.

| Column | Frozen at | Why re-deriving it later would be wrong |
|---|---|---|
| `stock_movements.cost_total` | the movement | For an **acquiring** movement it is the exact money that arrived, handed over by 05 as a line total and a unit count — never a rounded per-unit cost (05 `[AC-U08]`). For every other type it is the batch's average at that moment, which is the only correct cost of goods for that sale: an average recomputed later would make the margin on an old sale depend on a purchase made afterwards |
| `stock_movements.occurred_at` | the movement | The business moment — the phone's clock for an offline sale `[D-24]`. `recorded_at` is the server's and the two are different facts, so neither can be derived from the other |
| `stock_count_items.system_quantity` | the moment the batch is counted | *"so a count's result does not shift while selling continues"*. Re-reading the balance at completion time would turn a real discrepancy into whatever had been sold since `[AC-E10]` |
| `batch_mrp_change_requests.current_unit_mrp` | the request | *"so an approved request still shows the old price"*. After approval the batch holds the new one, and the request is the record of what was asked for `[D-38]` |
| `batches.unit_mrp` | the **purchase** | Frozen by `[D-38]` rather than by denormalisation: it is the price this batch sells at, set when the stock arrived, changeable only by platform approval. A batch created by a transfer copies the source's MRP, which is why *"after a transfer the same batch number, expiry and MRP exist in two branches"* |
| `stock_movements.branch_id` | the movement | Copied from the batch so the branch rule needs no join, exactly as `tenant_id` is. The three-column composite FK keeps it honest `[AC-U10]` |

**Deliberately not stored**, from the data model's own list and this contract's §3 decision:
**expired** and **near expiry** (dates, not states `[AC-U07]`), the **batch state** of Product
Specification §7 (derived from status, date and balance), **stock value** (`balance × average`),
**low stock** (a sum against `reorder_level`), the **earliest-expiry pre-selection** (an
`ORDER BY`) — and **the per-unit average cost itself**, which is
`cost_basis_total ÷ cost_basis_units` computed in `Decimal` and rounded once for display
([`SPEC.md`](SPEC.md) §3, *The cost basis*).

---

## 2. HTTP surface

All paths under `/api/v1`. Every route is authenticated, every **write** additionally passes
`api`'s access-mode gate (`403 subscription_read_only` while read-only `[AC-W14]`), and every
list is **cursor-paginated** — `ListPage<T>` with `next_cursor` + `has_more`, no total count
(`../00-foundation/CONTRACT.md` §2).

**One rule for the branch, stated once:** a **write** names its branch as a **path segment** and
uses `api::require_at`, which resolves and checks it; a **read** names it as an optional query
filter and uses `api::require`, filtering to the branches the caller holds the slug at
`[AC-U14]` `[AC-W08]`. This module is the first that can use `require_at` at all —
`05-procurement` could not, because no path there carries a branch.

### Stock reads

| Method | Path | Permission | Request | Response | Errors |
|---|---|---|---|---|---|
| `GET` | `/stock/batches` | `stock.view` | `branch_id?`, `product_id?`, `batch_number?`, `status?`, `state?` ∈ `sellable \| expired \| near_expiry \| any`, `has_stock?`, `order?` ∈ `expiry \| recent`, `cursor`, `limit` | `ListPage<BatchStock>` — batch, on-hand, derived state. **`unit_cost` and `stock_value` present only with `stock.cost.view`** | `403 branch_not_allowed` for an explicit `branch_id` the caller lacks |
| `GET` | `/stock/batches/{id}` | `stock.view` | — | `BatchStock` | `404 not_found`, `403` |
| `GET` | `/stock/batches/{id}/movements` | `stock.view` | `type?`, `occurred_from/to?`, `cursor`, `limit` | `ListPage<StockMovement>` — the batch's ledger, newest first | `404`, `403` |
| `GET` | `/stock/movements` | `stock.view` | `branch_id?`, `batch_id?`, `product_id?`, `type?`, `occurred_from/to?`, `cursor`, `limit` | `ListPage<StockMovement>` | `403` |
| `GET` | `/stock/sellable-batches` | `stock.view` | **`branch_id` and `product_id` required** | `ListPage<SellableBatch>` — active, unexpired, positive balance, **ordered by `expiry_date` ascending** `[D-4]` `[AC-U16]` | `400 validation_failed` if either is missing, `403` |
| `GET` | `/stock/batch-lookup` | `stock.view` | **`branch_id`, `product_id`, `batch_number` required** | `BatchStock` — the existing batch's MRP and on-hand for the purchase-entry prompt `[D-10]` | `404 not_found` when the batch does not exist yet, `400`, `403` |
| `GET` | `/stock/expiring` | `stock.view` | `branch_id?`, `state?` ∈ `near \| expired \| both` (default `both`), `window_days?` (default: the pharmacy's `near_expiry_days` `[D-5]`), `has_stock?` (default `true`), `cursor`, `limit` | `ListPage<ExpiringBatch>` — soonest first. This is also the "send it back to the supplier" pile `[D-17]` | `403` |
| `GET` | `/stock/low` | `stock.view` | `branch_id?`, `cursor`, `limit` | `ListPage<LowStockProduct>` — product, branch, sellable on-hand, `reorder_level`. **Only products with a reorder level row** | `403` |

**`window_days` is a parameter, not a lookup.** 06 reads no setting `[AC-U07]`; the handler
resolves the pharmacy's `near_expiry_days` before the query and the client may override it for a
one-off view. The same shape, for the same reason, carries the zero-stock policy into the
stock-out command (§3).

**There is no `/stock/valuation`.** A valuation total across a branch or the pharmacy is a report
and belongs to `11-reporting` `[D-99]`; what this module gives it is `stock_value` per batch and
the rule that it multiplies before it divides ([`SPEC.md`](SPEC.md) §3).

### Batch state

| Method | Path | Permission | Request | Response | Errors |
|---|---|---|---|---|---|
| `POST` | `/stock/batches/{id}/quarantine` | `batch.quarantine` | `{ reason }` — **required** | `200 Batch` | `404`, `403`, `400 validation_failed` (empty reason), `409 batch_already_quarantined` |
| `POST` | `/stock/batches/{id}/release` | `batch.quarantine` | — | `200 Batch` | `404`, `403`, `409 batch_not_quarantined` |

**Neither route is named by the Notion page**, and both are here because the page gives
`batches.status` and `quarantine_reason` as tenant-writable columns `[AC-U05]`: without an
endpoint, `quarantined` is a state nothing can enter, which is the thing 05 Q1 refused to ship.
**Quarantine writes no movement** `[AC-E16]` — the stock has not moved, it has stopped being
offered. Releasing an **expired** batch changes nothing a customer can see: it is still outside
every sellable read, because the date says so `[AC-S01]`.

### Stock adjustments — the correction mechanism

| Method | Path | Permission | Request | Response | Errors |
|---|---|---|---|---|---|
| `GET` | `/stock-adjustments` | `stock_adjustment.view` | `branch_id?`, `reason?`, `batch_id?`, `stock_count_id?`, `created_from/to?`, `cursor`, `limit` | `ListPage<AdjustmentSummary>` | `403` |
| `GET` | `/stock-adjustments/{id}` | `stock_adjustment.view` | — | `Adjustment` with its items and their movements | `404`, `403` |
| `POST` | `/branches/{branch_id}/stock-adjustments` | `stock_adjustment.create` (`require_at`) | `NewAdjustment` — optional client UUIDv7 `id`, `reason`, `note?`, `items[]` of `{ batch_id, quantity }` | `201 Adjustment` · `200` on a repeat with the same id | `409 adjustment_empty`, `409 insufficient_stock`, `400 validation_failed`, `404 not_found`, `403 branch_not_allowed` |

**There is no `PATCH`, `PUT` or `DELETE` on an adjustment.** An adjustment *is* the correction
path for the ledger `[AC-U02]`, so correcting one is another adjustment with reason `correction`
and a note. This is the single most important shape in the module to not relax later.

**A negative adjustment is checked against stock** like any other out-movement: short stock is
`409 insufficient_stock` `[AC-E04]`. A positive one is not checked against anything — found
stock is found.

### Stock counts — online only `[AC-S07]`

| Method | Path | Permission | Request | Response | Errors |
|---|---|---|---|---|---|
| `GET` | `/stock-counts` | `stock_count.view` | `branch_id?`, `status?`, `cursor`, `limit` | `ListPage<StockCountSummary>` | `403` |
| `GET` | `/stock-counts/{id}` | `stock_count.view` | `cursor`, `limit` for its items | `StockCount` with items, each carrying `system_quantity`, `counted_quantity` and the difference | `404`, `403` |
| `POST` | `/branches/{branch_id}/stock-counts` | `stock_count.create` (`require_at`) | `{ id? }` | `201 StockCount`, `status = open` | `400`, `403` |
| `PUT` | `/stock-counts/{id}/items` | `stock_count.create` | `items[]` of `{ batch_id, counted_quantity }` — **the whole document**, replacing the set | `200 StockCount` | `409 stock_count_not_open`, `400`, `404`, `403` |
| `POST` | `/stock-counts/{id}/complete` | `stock_count.complete` | — | `200 { count, adjustment? }` — **one** adjustment for the differences, or none when nothing differs `[AC-E09]` | `409 stock_count_not_open`, `404`, `403` |
| `POST` | `/stock-counts/{id}/cancel` | `stock_count.cancel` | — | `200 StockCount` | `409 stock_count_not_open`, `404`, `403` |

**A count's items are one whole document**, the shape 05 chose for a purchase draft and for the
same reason: a stock-take screen submits a sheet, and a line-at-a-time API is a round trip per
row on a counter connection. `system_quantity` is frozen **per item, when that item is first
written** `[AC-E10]`, so a sheet submitted twice does not re-freeze what has since been sold.

### Branch transfers `[D-36]`

| Method | Path | Permission | Request | Response | Errors |
|---|---|---|---|---|---|
| `GET` | `/branch-transfers` | `branch_transfer.view` | `branch_id?` (either end), `from_branch_id?`, `to_branch_id?`, `created_from/to?`, `cursor`, `limit` | `ListPage<TransferSummary>` — **visible if either branch is permitted** | `403` |
| `GET` | `/branch-transfers/{id}` | `branch_transfer.view` | — | `Transfer` with items and both movements per item | `404`, `403` |

**`POST /branch-transfers` is not claimed here, and the reason is a disagreement rather than a
boundary.** Every table a transfer writes is 06's, so by 09's own one-line test it is
`commands::transfer_stock` and not a flow — but `workflows::transfer` is in 09's settled list of
seven and 09 Q6 forbids a module to claim one of those paths. [`SPEC.md`](SPEC.md) §5 **Q4** asks
for a ruling; the command is specified in §3 and works unchanged either way. Whichever crate
holds the route needs the **two-branch** permission check `[AC-W09]`, which `api::require_at`
cannot express and 09 `CONTRACT.md` §3 already filed as a gap.

### Reorder levels

| Method | Path | Permission | Request | Response | Errors |
|---|---|---|---|---|---|
| `GET` | `/reorder-levels` | `reorder_level.view` | `branch_id?`, `product_id?`, `cursor`, `limit` | `ListPage<ReorderLevel>` | `403` |
| `PUT` | `/branches/{branch_id}/reorder-levels/{product_id}` | `reorder_level.set` (`require_at`) | `{ reorder_level }` | `200 ReorderLevel` — upsert `[AC-E15]` | `400 validation_failed` (negative), `404 not_found` (unknown product or branch), `403` |
| `DELETE` | `/branches/{branch_id}/reorder-levels/{product_id}` | `reorder_level.set` (`require_at`) | — | `204` — **no row means no alert** | `404`, `403` |

### Batch MRP change requests — tenant side `[D-38]`

| Method | Path | Permission | Request | Response | Errors |
|---|---|---|---|---|---|
| `GET` | `/stock/batch-mrp-requests` | `batch_mrp_request.view` | `status?`, `branch_id?`, `batch_id?`, `cursor`, `limit` | `ListPage<MrpChangeRequest>` — this pharmacy's own | `403` |
| `GET` | `/stock/batch-mrp-requests/{id}` | `batch_mrp_request.view` | — | `MrpChangeRequest` | `404`, `403` |
| `POST` | `/stock/batches/{id}/mrp-change-requests` | `batch_mrp_request.create` | `{ id?, requested_unit_mrp, reason }` | `201 MrpChangeRequest`, `status = pending`, `current_unit_mrp` copied `[AC-E12]` | `409 batch_mrp_request_pending`, `409 batch_untracked`, `400 validation_failed`, `404`, `403` |

**A pharmacy creates a request and can do nothing else to it** — no update route, no cancel
route, and `pos_app` holds `SELECT, INSERT` only. Whether a pending request may be **withdrawn**
is [`SPEC.md`](SPEC.md) §5 **Q5**: the status enum has three values and inventing a fourth is not
this contract's to do.

### Batch MRP change requests — platform side, `/api/v1/admin`, platform token only

| Method | Path | Auth | Request | Response | Errors |
|---|---|---|---|---|---|
| `GET` | `/admin/batch-mrp-requests` | platform token | `status?`, `tenant_id?`, `cursor`, `limit` | `ListPage<AdminMrpChangeRequest>` — **across every pharmacy**, with the pharmacy and branch named | — |
| `POST` | `/admin/batch-mrp-requests/{id}/approve` | platform token, **`super_admin`** `[D-38]` | `{ review_note? }` | `200 MrpChangeRequest` — the batch's `unit_mrp` is now the requested one `[AC-E13]` | `409 mrp_request_not_pending`, `404 not_found` |
| `POST` | `/admin/batch-mrp-requests/{id}/reject` | platform token, **`super_admin`** | `{ review_note }` — required | `200 MrpChangeRequest` — the batch is **unchanged** `[AC-E14]` | `409 mrp_request_not_pending`, `404`, `400` |

These three run under `Tx::begin_platform` and the `app.is_platform` policy, and they are the
only writes in the system that may change `batches.unit_mrp`. **The approval must also write one
`audit_logs` row** — *"approving it updates `batches.unit_mrp` and writes to `audit_logs`"* — and
that table belongs to no module yet: [`SPEC.md`](SPEC.md) §5 **Q3**. Nothing else about the
approval waits on it.

The platform role check is **`super_admin`**, not a permission slug: `[D-38]` says super admin,
and `platform_users.role` is `03-identity`'s column. No slug in this module is platform-only, and
no pharmacy slug grants a review.

### Permission slugs this module adds

Seeded by **this module's migration** into `permissions`, the table `03-identity` owns, with
`module = 'inventory'` — the pattern `m0003_identity` and `m0006_catalog_permissions` already
use. Every slug is `is_tenant_assignable = true` and **`is_branch_scoped = true`**: stock is
always at a branch.

| Slug | Owner | Manager (shipped template) | Cashier |
|---|---|---|---|
| `stock.view` | ✓ | ✓ | **✓** |
| `stock.cost.view` | ✓ | ✓ | — |
| `batch.quarantine` | ✓ | ✓ | — |
| `stock_adjustment.view` | ✓ | ✓ | — |
| `stock_adjustment.create` | ✓ | ✓ | — |
| `stock_count.view` · `stock_count.create` | ✓ | ✓ | — |
| `stock_count.complete` | ✓ | ✓ | — |
| `stock_count.cancel` | ✓ | — | — |
| `branch_transfer.view` | ✓ | ✓ | — |
| `branch_transfer.create` | ✓ | — | — |
| `reorder_level.view` | ✓ | ✓ | — |
| `reorder_level.set` | ✓ | ✓ | — |
| `batch_mrp_request.view` | ✓ | ✓ | — |
| `batch_mrp_request.create` | ✓ | — | — |

Fifteen slugs. Three things the table decides on purpose:

1. **The cashier gets `stock.view` and not `stock.cost.view`.** This is the only module where a
   cashier needs a read at all — the sale screen lists batches, expiry and quantity on hand
   `[D-4]` — and splitting the cost figures out is what lets the pharmacy give it to the counter
   without showing what it paid. `unit_cost` and `stock_value` are absent from the response
   rather than null, so no client renders an empty money column.
2. **`branch_transfer.create` and `batch_mrp_request.create` are owner-only in the shipped
   template.** Moving stock between shops and asking the platform to reprice a batch are both
   decisions with money behind them; a pharmacy may of course grant either `[D-65]`.
3. **There is no slug for a stock movement** `[AC-U17]`, which is this contract's answer to 05
   `CONTRACT.md` §2's *"`purchase.post` and 06's stock-in slug"* — the first slug is the whole
   authorisation ([`SPEC.md`](SPEC.md) §3).

### Error cases

| Condition | Status | Code | Message to the user |
|---|---|---|---|
| Not enough stock in that batch, and the caller is not permitted to oversell | 409 | `insufficient_stock` | "There are only {n} left in that batch. Pick another batch, or adjust the stock first." |
| Selling from a batch whose expiry date has passed | 409 | `batch_expired` | "That batch expired on {date} and cannot be sold. Pick another batch." |
| Selling from a batch someone has quarantined | 409 | `batch_quarantined` | "That batch is on hold: {reason}. Pick another batch, or release it first." |
| An MRP change request on an untracked (non-medicine) batch | 409 | `batch_untracked` | "This item is not tracked by batch, so it has no batch price to change." |
| A second pending MRP request for one batch | 409 | `batch_mrp_request_pending` | "A price change for this batch is already waiting for review." |
| Reviewing a request that is already approved or rejected | 409 | `mrp_request_not_pending` | "This request has already been reviewed." |
| Quarantining a quarantined batch, or releasing one that is not | 409 | `batch_already_quarantined` / `batch_not_quarantined` | "That batch is already on hold." / "That batch is not on hold." |
| An adjustment or a transfer with no items | 409 | `adjustment_empty` / `transfer_empty` | "Add at least one item before saving this." |
| Editing or completing a count that is not open | 409 | `stock_count_not_open` | "This stock count is already finished. Start a new one to count again." |
| A medicine with no batch number or no expiry date `[D-8]`, a zero quantity, a transfer to the same branch, a negative reorder level, a requested MRP equal to the current one | 400 | `validation_failed` | Plain language naming the field, in `details.fields` |
| Stock at a branch the caller has no permission at — **or either branch of a transfer** | 403 | `branch_not_allowed` | "You do not have access to that branch." `details.required` names the slug |
| Another **pharmacy's** row, or an unknown id | 404 | `not_found` | "Not found." — never 403, which would confirm the id exists |
| A write while the subscription is read-only | 403 | `subscription_read_only` | Raised by the route before any command runs `[AC-W14]`. **Every read still succeeds** |
| Anything unexpected | 500 | `internal_error` | The transaction rolls back whole; the detail goes to the log with the trace id |

`insufficient_stock` and `batch_expired` are **already reserved** in
`../00-foundation/CONTRACT.md` §2 and are this module's to raise — which is the answer to 05
`[AC-W12]`'s deferral. The other six 409s are new and declared here; each travels as a
`&'static str`, exactly as permission slugs do.

**`insufficient_stock` is raised by the command, not by the route**, under the lock
`[AC-U08]` — a route that checked the balance and then called the command would permit the
oversell between the two statements, which is precisely the race 09 `[AC-U10]` exists to
prevent.

---

## 3. Public Rust surface

`crates/inventory` depends on `core`, `db`, `api`, and on **`catalog`** for one read — the
batch-tracked fact `[D-8]` it must enforce rather than trust `[AC-W03]`, the same single
dependency and the same justification as `05-procurement`. It depends on **no other module
crate**, and nothing depends on it except `server`, `workflows` and `reporting` `[D-97]`.
`repo.rs` and `entity/` are private.

```rust
// queries.rs — every function generic over C: ConnectionTrait (09 Q3, amending [D-98]),
//              so a workflow reads inside its own transaction and sees its own writes.

pub async fn batches<C: ConnectionTrait>(c: &C, ctx: &TenantContext, f: BatchFilter, p: Cursor)
    -> Result<ListPage<BatchStock>, AppError>;
pub async fn batch<C: ConnectionTrait>(c: &C, ctx: &TenantContext, id: BatchId)
    -> Result<BatchStock, AppError>;
pub async fn batch_lookup<C: ConnectionTrait>(c: &C, ctx: &TenantContext,
        branch: BranchId, product: ProductId, batch_number: &str)
    -> Result<Option<BatchStock>, AppError>;          // [D-10]'s prompt, 05's deferral

pub async fn sellable_batches<C: ConnectionTrait>(c: &C, ctx: &TenantContext,
        branch: BranchId, product: ProductId, today: NaiveDate, p: Cursor)
    -> Result<ListPage<SellableBatch>, AppError>;     // earliest expiry first  [D-4] [AC-U16]

pub async fn on_hand<C: ConnectionTrait>(c: &C, ctx: &TenantContext, batches: &[BatchId])
    -> Result<Vec<OnHand>, AppError>;                 // reads, locks nothing

pub async fn movements<C: ConnectionTrait>(c: &C, ctx: &TenantContext, f: MovementFilter, p: Cursor)
    -> Result<ListPage<StockMovement>, AppError>;
pub async fn expiring<C: ConnectionTrait>(c: &C, ctx: &TenantContext,
        f: ExpiryFilter, window_days: u32, today: NaiveDate, p: Cursor)
    -> Result<ListPage<ExpiringBatch>, AppError>;      // [D-5]'s window is a PARAMETER
pub async fn low_stock<C: ConnectionTrait>(c: &C, ctx: &TenantContext, branch: Option<BranchId>, p: Cursor)
    -> Result<ListPage<LowStockProduct>, AppError>;
pub async fn reorder_levels<C: ConnectionTrait>(…) -> Result<ListPage<ReorderLevel>, AppError>;

pub async fn adjustments<C: ConnectionTrait>(…)  -> Result<ListPage<AdjustmentSummary>, AppError>;
pub async fn adjustment<C: ConnectionTrait>(…)   -> Result<Adjustment, AppError>;
pub async fn stock_counts<C: ConnectionTrait>(…) -> Result<ListPage<StockCountSummary>, AppError>;
pub async fn stock_count<C: ConnectionTrait>(…)  -> Result<StockCount, AppError>;
pub async fn transfers<C: ConnectionTrait>(…)    -> Result<ListPage<TransferSummary>, AppError>;
pub async fn transfer<C: ConnectionTrait>(…)     -> Result<Transfer, AppError>;
pub async fn mrp_requests<C: ConnectionTrait>(…) -> Result<ListPage<MrpChangeRequest>, AppError>;
pub async fn mrp_request<C: ConnectionTrait>(…)  -> Result<MrpChangeRequest, AppError>;

// --- the locking read. FOR UPDATE, therefore generic, by 09 [AC-U10] / [AC-U08] ---

/// Locks the **`batches`** rows — not `stock_balances`, which `pos_app` cannot row-lock
/// [AC-W12] — in ascending (tenant_id, batch_id) order, then returns each batch's balance,
/// expiry and status. This is the read every stock-out decision is made on, and the one
/// lock order all seven workflows share (09 [AC-U08]).
pub async fn on_hand_for_update<C: ConnectionTrait>(c: &C, ctx: &TenantContext, batches: &[BatchId])
    -> Result<Vec<OnHandLocked>, AppError>;
```

```rust
// commands.rs — take &Tx. Called directly from a handler for a single-module write,
//               and by `workflows` for the four flows that move stock [D-97].

// ---- batches -------------------------------------------------------------------

pub struct BatchKey {
    pub branch_id: BranchId,
    pub product_id: ProductId,
    pub batch_number: Option<String>,     // None = the untracked batch  [D-8]
    pub expiry_date: Option<NaiveDate>,   // all three None together     [AC-U13]
    pub unit_mrp: Option<Decimal>,
}

/// Returns the existing batch UNCHANGED when one matches — MRP included  [D-10] [D-38]
/// [AC-E01] — and otherwise creates it with its zero balance row [AC-E02]. Refuses a
/// null batch number for a product 01 says is not `non_medicine` [AC-W03].
pub async fn find_or_create_batch(tx: &Tx, ctx: &TenantContext, key: BatchKey)
    -> Result<Batch, AppError>;

pub async fn quarantine_batch(tx: &Tx, ctx: &TenantContext, id: BatchId, reason: String)
    -> Result<Batch, AppError>;                       // writes no movement  [AC-E16]
pub async fn release_batch(tx: &Tx, ctx: &TenantContext, id: BatchId)
    -> Result<Batch, AppError>;

// ---- movements -----------------------------------------------------------------

/// The cost basis, and the ONLY shape money enters a batch in. There is deliberately no
/// per-unit variant: 05 [AC-U08] never emits one and [AC-W11] means nothing could accept it.
pub enum CostBasis {
    /// The exact money owed for the line and the exact units it bought — 05's
    /// `line_cost_total` + `units`. 10 units at 8.50 with 1 free is `(85.00, 11)`.
    Exact { line_cost_total: Decimal, units: i32 },
    /// Value these units at the batch's current average, rounded once, and freeze it on
    /// the movement. Used by transfers, adjustments, sales and returns.
    AtBatchAverage,
}

/// The seven ledger types, split by direction. `AdjustmentIn` and `AdjustmentOut` are both
/// the one `adjustment` type in the database — the direction is the quantity's sign, which
/// is the one type the check constraint lets go either way. There are seven types, not
/// eight.
pub enum InKind  { PurchaseIn, SaleReturnIn, TransferIn, AdjustmentIn }
pub enum OutKind { SaleOut, PurchaseReturnOut, TransferOut, AdjustmentOut }

/// Short-stock behaviour, supplied by the CALLER and never looked up here. Three different
/// grounds — the pharmacy's `allow_zero_stock_sale`, a manager's PIN approval, and an
/// offline replay [D-25] — all arrive as `Accept`, and this module must not care which
/// (09 [AC-U05]).
pub enum ShortStock { Reject, Accept }

pub struct StockIn {
    pub batch_id: BatchId,
    pub movement_id: Option<StockMovementId>,  // the phone's UUIDv7 when there is one (§5)
    pub kind: InKind,
    pub units: i32,                            // positive magnitude
    pub cost: CostBasis,
    pub occurred_at: DateTime<Utc>,            // the business moment  [AC-E17]
    pub device_id: Option<DeviceId>,
}

pub struct StockOut {
    pub batch_id: BatchId,
    pub movement_id: Option<StockMovementId>,
    pub kind: OutKind,
    pub units: i32,                            // positive magnitude; the sign is the kind's
    pub occurred_at: DateTime<Utc>,
    pub device_id: Option<DeviceId>,
    pub short: ShortStock,
    /// Reject a sale out of an expired or quarantined batch [AC-E06]. `false` is what admits
    /// a replayed offline sale and a return into an expired batch [D-17] [AC-E07].
    pub enforce_sellable: bool,
}

pub struct MovementResult {
    pub movement_id: StockMovementId,
    pub cost_total: Decimal,                   // what was frozen on the row
    pub resulting_balance: i32,                // may be negative  [AC-U04]
    pub shortfall: Option<i32>,                // Some(n) when it went negative  [D-25]
    pub batch_expired_at_occurrence: bool,     // [D-17]
}

pub async fn record_stock_in(tx: &Tx, ctx: &TenantContext, input: StockIn)
    -> Result<MovementResult, AppError>;
pub async fn record_stock_out(tx: &Tx, ctx: &TenantContext, input: StockOut)
    -> Result<MovementResult, AppError>;

/// The plural write, and what 09 §3 called "the allocation command" [D-9]. It allocates
/// NOTHING: it takes the lines the caller already chose [D-4], locks every batch in one
/// order [AC-U08], then writes them. Any caller moving more than one batch MUST use this,
/// or the shared lock order does not hold (09 [AC-U08]).
pub async fn record_stock_out_many(tx: &Tx, ctx: &TenantContext, inputs: Vec<StockOut>)
    -> Result<Vec<MovementResult>, AppError>;

// ---- adjustments, counts, transfers, reorder levels -----------------------------

pub async fn create_adjustment(tx: &Tx, ctx: &TenantContext, input: NewAdjustment)
    -> Result<Adjustment, AppError>;                   // header + items + one movement each  [AC-E08]

pub async fn open_stock_count(tx: &Tx, ctx: &TenantContext, branch: BranchId)
    -> Result<StockCount, AppError>;
pub async fn set_count_items(tx: &Tx, ctx: &TenantContext, id: StockCountId, items: Vec<CountedBatch>)
    -> Result<StockCount, AppError>;                   // whole document; freezes system_quantity  [AC-E10]
pub async fn complete_stock_count(tx: &Tx, ctx: &TenantContext, id: StockCountId)
    -> Result<(StockCount, Option<Adjustment>), AppError>;  // ONE adjustment, or none  [AC-E09]
pub async fn cancel_stock_count(tx: &Tx, ctx: &TenantContext, id: StockCountId)
    -> Result<StockCount, AppError>;

/// One transaction: transfer_out, find-or-create the target batch, transfer_in, with equal
/// and opposite cost totals [D-36] [AC-E11]. Every table it writes is this module's, which
/// is why SPEC.md §5 Q4 asks whether `workflows::transfer` should exist.
pub async fn transfer_stock(tx: &Tx, ctx: &TenantContext, input: NewTransfer)
    -> Result<Transfer, AppError>;

pub async fn set_reorder_level(tx: &Tx, ctx: &TenantContext, branch: BranchId, product: ProductId, level: i32)
    -> Result<ReorderLevel, AppError>;
pub async fn clear_reorder_level(tx: &Tx, ctx: &TenantContext, branch: BranchId, product: ProductId)
    -> Result<(), AppError>;

// ---- MRP change requests  [D-38] ------------------------------------------------

pub async fn request_mrp_change(tx: &Tx, ctx: &TenantContext, batch: BatchId,
        requested_unit_mrp: Decimal, reason: String)
    -> Result<MrpChangeRequest, AppError>;             // always `pending`  [AC-E12]
```

```rust
// platform.rs — platform-token writes, each asserting app.is_platform is set,
//               the pattern 01-catalog and 02-tenancy already use.

pub async fn approve_mrp_change(tx: &Tx, id: MrpRequestId, by: PlatformUserId, note: Option<String>)
    -> Result<MrpChangeRequest, AppError>;   // sets batches.unit_mrp  [AC-E13]
pub async fn reject_mrp_change(tx: &Tx, id: MrpRequestId, by: PlatformUserId, note: String)
    -> Result<MrpChangeRequest, AppError>;   // batch unchanged  [AC-E14]

// perms.rs — every slug as a &'static str const, so `crates/server`'s posting route can
//            require one without re-typing the string (the shape `catalog::perms` has).
pub mod perms { pub const STOCK_VIEW: &str = "stock.view"; /* … 15 */ }
```

### What the four stock-writing flows call — the handoff module 09 §3 asked for

09 lists six flows that write 06's tables: `sell`, `refund`, `post_purchase`, `transfer`,
`void_sale` and `post_supplier_return`. **No new command is needed for any of them** beyond the
ones above, and that is the design rather than a shortcut — the ledger has seven movement types
and every flow is some of them.

| Flow | What it calls here | Notes |
|---|---|---|
| `post_purchase` | `find_or_create_batch` then `record_stock_in` per line — or `receive_purchase_lines`, below | Exactly 05's first deferral row. `units` is 05's `quantity + free_quantity`, `cost` is `CostBasis::Exact { line_cost_total, units }` (05 `[AC-U08]`) |
| `sell` | `record_stock_out_many` with `kind: SaleOut`, `short` from the route's policy, `enforce_sellable: true` online | One `StockOut` per sale line, because `[D-9]` makes one line one batch. The lock order is the command's `[AC-U08]` |
| `refund` · `void_sale` | `record_stock_in` with `kind: SaleReturnIn`, `cost: AtBatchAverage`, into **the exact batch the units came from** | `enforce_sellable: false` — a return into an expired batch is accepted `[D-17]` `[AC-E07]`, and `batch_expired_at_occurrence` comes back for the flag |
| `post_supplier_return` | `record_stock_out` with `kind: PurchaseReturnOut` | **This is 05's "reversing movement"**: a reversal in this ledger is a new movement of the opposite direction, not an undo. `short: Reject`, so a return of more than is on hand is `409 insufficient_stock` (05 `[AC-W12]`) |
| `transfer` | `transfer_stock` — one call, the whole transfer | See §2 and [`SPEC.md`](SPEC.md) §5 Q4 |

```rust
/// One call per posted purchase: find-or-create each batch and write its purchase_in.
/// Results come back in INPUT ORDER, one per line, so the workflow can zip them with 05's
/// `PostingLine`s — this module cannot name `PurchaseItemId` without depending on 05, which
/// [D-97] forbids.
pub struct PurchaseStockIn {
    pub branch_id: BranchId,
    pub product_id: ProductId,
    pub batch_number: Option<String>,
    pub expiry_date: Option<NaiveDate>,
    pub unit_mrp: Option<Decimal>,     // the bill's printed price; NEVER overwrites an
    pub units: i32,                    //   existing batch's MRP  [D-10] [AC-E01]
    pub line_cost_total: Decimal,      // 05's exact figure — not a per-unit cost
    pub occurred_at: DateTime<Utc>,
}
pub struct ReceivedStock { pub batch_id: BatchId, pub stock_movement_id: StockMovementId }

pub async fn receive_purchase_lines(tx: &Tx, ctx: &TenantContext, lines: Vec<PurchaseStockIn>)
    -> Result<Vec<ReceivedStock>, AppError>;
```

`ReceivedStock` is precisely what 05's `complete_posting` stamps onto `purchase_items.batch_id`
and `.stock_movement_id`. **05's seven deferral rows, answered:** the create-or-find + stock-in
command (`receive_purchase_lines`); the cost basis (`CostBasis::Exact`, and
[`SPEC.md`](SPEC.md) §3); batch uniqueness (`(tenant_id, branch_id, product_id, batch_number)`,
§1 — so a second bill for the same batch number **finds** the batch); the existing-MRP read
(`batch_lookup`); the on-hand `FOR UPDATE` read and the short-batch code
(`on_hand_for_update`, `insufficient_stock`); who owns the batch-tracked fact (**01**, enforced
here `[AC-W03]`); and the reversing movement (`record_stock_out` with `PurchaseReturnOut`).

**Called by:** `crates/server` (mounts the `OpenApiRouter`; owns the six cross-module routes —
09 Q6) · `workflows` (the six flows above) · `11-reporting` (stock valuation, movement, expiry
and margin reports, reading these tables with `query_as!` `[D-99]`) · `12-jobs` (the two sweep
reads, §4). **By no other module crate** `[D-97]`.

### What this module needs from another

| Need | From | Status |
|---|---|---|
| **"Does this product require a batch?"** `[D-8]` — `products.product_type <> 'non_medicine'` — as a read **generic over `C: ConnectionTrait`**, callable inside a flow's transaction | **01 Catalogue** | **Gap.** `catalog`'s reads all take `&Db` today, including `is_sellable`; 09 Q3 already records that `catalog` has no generic reads and needs a backlog item. Called inside `find_or_create_batch`, so a non-generic read would run on a bare connection under forced RLS — the exact defect module 03 failed Gate 5 for. **It is an integrity rule, so 06 reads it rather than trusting a parameter**; the two tenant *settings* below are policy and arrive as parameters instead |
| `near_expiry_days` and `allow_zero_stock_sale`, read **inside the request's transaction** | **02 Tenancy**, through a caller | **Gap, and 06 does not want the dependency.** `tenancy::settings` takes `&Db`, so on a bare connection under forced RLS it returns nothing. Two ways out, and this contract recommends the first: a **`core::ports::TenantSettingsSource`** that `server` implements over `tenancy` — the `AccessModeSource` precedent exactly — or `tenancy`'s reads made generic (the backlog item 09 Q3 identified). Either way the values reach 06 as **parameters** `[AC-U07]`, and 06 imports no `tenancy` |
| A **two-branch** permission check for a transfer `[AC-W09]` | **00 Foundation** (`api`) | **Gap**, already recorded in 09 `CONTRACT.md` §3. `api::require_at` resolves one branch from the path, so a user holding the slug at the source only would pass. A `require_at_both`, or a resolver returning a set, is needed before the transfer route exists — in whichever crate it lands (§5 Q4) |
| `AuthUser::branches_with(slug) -> Vec<BranchId>` | **00 Foundation** (`api`) | **Convenience, not a gap**, and already requested by 05. Eight read handlers here filter to the same set |
| The owner of `sync_flags`, and a `type` value for `[D-17]` | **10 Sync** (presumed) | [`SPEC.md`](SPEC.md) §5 **Q1**. 06 supplies the facts regardless (`MovementResult`) |
| The owner of `audit_logs` | **unassigned** | [`SPEC.md`](SPEC.md) §5 **Q3**. Blocks one row inside `approve_mrp_change` |
| Nothing else | — | `Tx::begin`, `Tx::begin_platform`, `TenantTx`, `api::require`, `api::require_at`, the access-mode gate, `ListPage`, `Cursor`, `AppError` all exist |

---

## 4. Events and jobs

| Trigger | Job | Schedule | Idempotent? |
|---|---|---|---|
| — | **none owned here** | — | — |

**This module schedules nothing and is woken by nothing**, and two sweeps that a reader will
expect to find here are deliberately elsewhere:

| Sweep | Whose | What 06 provides |
|---|---|---|
| **Near-expiry alerts** `[D-5]` | `12-jobs`, writing cross-cutting `notifications` with a `dedupe_key` so a daily run does not send the same alert twice | `queries::expiring`, with the window as a parameter. **No state is flipped** — expiry is derived `[AC-U07]`, so there is nothing for a job to set and nothing that can be stale |
| **Low-stock alerts** | `12-jobs`, likewise | `queries::low_stock`, the sum over a product's sellable batches at a branch against its `reorder_level` |

`pos_jobs` therefore holds **`SELECT` only** on all eleven tables (§1, *Grants*). A job that
could insert a movement under `BYPASSRLS` is a risk with no feature behind it, and the ledger's
append-only grant would be the only thing between a buggy sweep and a pharmacy's stock figures.

**Nothing in this module may enqueue a job from inside a transaction** (09 `[AC-U07]`): a worker
can pick the job up before the commit, and it would survive a rollback. An alert that follows a
write is enqueued by the caller after the commit returns.

---

## 5. Offline and sync

| Question | Answer |
|---|---|
| Can the mobile app create this offline? | **Only one row type, and only inside a replayed sale.** `[D-24]` makes the offline scope sales only, so a `sale_out` movement is the whole of it. Stock counts, adjustments and transfers are **online** `[AC-S07]`; a sales return is online `[D-16]`; a batch is created offline by nobody — a phone selling offline sells from batches it already pulled |
| What the phone **pulls** | Its own branch's `batches` and `stock_balances`, so it can show expiry and quantity on hand on the sale screen `[D-4]`. Delivery, cursors and refresh are **10 Sync**'s contract, not this one |
| Client-generated id? | **`stock_movements.id` may be the phone's UUIDv7**, which is what the page means by *"created on the phone for offline sales"*, and `record_stock_in`/`record_stock_out` accept it. Every other table's id is **server-generated**: nothing else here is creatable offline ([`../rules/tenancy.md`](../rules/tenancy.md) rule 10 ties client generation to offline creation) |
| What makes an upload idempotent | **The sale's primary key, not the movement's** (09 `[AC-E02]`). The replay is refused by the root row inside the flow's own transaction, before any movement is attempted; child rows may take server ids. The movement's own PK is a **backstop** `[AC-W13]` and must not be relied on as the mechanism — a client that regenerated movement ids on retry would slip past it while the sale's key still correctly refuses. Not a Redis key either: a replay arrives hours `[D-26]` or days `[D-48]` later |
| Can it be flagged rather than rejected? | **Yes, for two of 09 `[AC-S02]`'s four cases, and 06 declares no flag column for either.** Short stock `[D-25]` and a batch that expired between sale and upload `[D-17]` are accepted when the caller says `ShortStock::Accept` / `enforce_sellable: false`, and the facts come back on `MovementResult` — `resulting_balance`, `shortfall`, `batch_expired_at_occurrence`. The **row** that records the flag is cross-cutting `sync_flags` ([`SPEC.md`](SPEC.md) §5 **Q1**), and its `stock_adjustment_id` is how a manager's correcting adjustment closes it. Everything else rejects (09 `[AC-S03]`) |
| Clock | `occurred_at` is the business moment the caller supplies — the device's clock, Asia/Dhaka; a sale made at 18:40 and uploaded at 22:10 happened at 18:40. `recorded_at` is set by the **database** `[AC-E17]`. A running total is order-independent, so a late upload needs no reordering and the balance is correct whatever order the day's movements arrive in; `occurred_at` orders every ledger view, with `id` as the tiebreaker |
| What a negative balance means afterwards | Exactly what it says: stock was sold that the system did not know about. Nothing repairs it automatically `[AC-S03]`. The manager clears the flag on 10-sync's reconciliation screen, usually with a `POST /branches/{id}/stock-adjustments` of reason `correction` — which is why the adjustment route is the one write in this module a reconciliation screen depends on |
