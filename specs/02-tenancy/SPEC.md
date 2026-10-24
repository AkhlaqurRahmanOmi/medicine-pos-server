# SPEC: 02 — Tenancy

**Status:** Draft
**Crate:** `crates/tenancy`
**Bounded context:** Context 2 — Tenancy and plans
**Owner:** Claude

---

## 1. Purpose

The pharmacy itself: who it is, which branches it trades from, what it has paid for, and
**whether it may act at all right now**. Every other module assumes a `tenant_id` that means
something; this module is what makes it mean something.

Its most consequential output is not a table but a derived value — the **access mode** of the
current request. A pharmacy awaiting approval cannot act, a suspended one cannot act, a
lapsed one may read but not write, and everything else may work normally. That decision is
made here and enforced on every request of every module.

It also owns the platform's side of the relationship: approving a signup, suspending a shop,
reviewing a bank slip, and the plan catalogue those subscriptions are sold from. The plan
tables are the one thing in this module with **no `tenant_id`** — every pharmacy reads them
and only platform staff edit them.

**Not this module's job:**

| Thing | Belongs to |
|---|---|
| Users, roles, permissions, devices, login | `03-identity` |
| The *mechanism* that carries `tenant_id` into Postgres (`Tx`, `set_config`) | `00-foundation` |
| Product prices, VAT per product, MRP | `04-pricing` |
| Producing the export file a tenant requested | `12-jobs` — this module owns the request row and its state |
| Deleting a pharmacy's domain rows at purge time | `12-jobs`, by the rule stated here |
| Route registration and middleware wiring | `crates/server`, Gate-0 exempt |

This module decides *whether* a request may write. It never decides *what* a request may
write — that is a permission, and permissions are `03-identity`.

---

## 2. Decisions this module implements

Cite, never restate. The reasoning lives in Notion.

| Ref | Decision | Consequence here |
|---|---|---|
| `[D-82]` | A new pharmacy needs platform approval before it can be used | `tenants.status` starts `pending_approval`, and that state is blocked, not merely flagged |
| `[D-20]` | A lapsed subscription is read-only after the grace period | Access mode has a **read-only** state; writes fail with `subscription_read_only` |
| `[D-83]` | Suspension is fully blocked, unlike a lapse | A separate state and a separate code — `tenant_suspended`. The app shows nothing else at all |
| `[D-81]` | Free trials are manual, case by case | No trial plan. A complimentary subscription carries `is_complimentary` and a required `grant_reason` |
| `[D-89]` | One plan in v1, every limit unlimited; the machinery stays | `plan_limits` ships with `max_value = NULL` rows. The limit check exists and always passes |
| `[D-19]` | Plan limits deferred; the model stays generic | Limits are key/value rows, so a new limit is a row and not a migration |
| `[D-18]` | Subscription payment is manual first, bKash later | `subscription_payments.method` carries `bank_transfer` now; the column, not the code, is what changes later |
| `[D-14]` | Role scope is per branch | `branches` must exist and be stable before `03-identity` can scope anything |
| `[D-30]` | Several devices per branch | Branch is a real row devices point at, not a label |

---

## 3. Behaviour

EARS acceptance criteria, each one a future test name.

### Access mode — the rule every other module depends on

- `[AC-U01]` The module SHALL resolve exactly one access mode for a request: **blocked**,
  **read_only**, **full**, or **full_with_warning**.
- `[AC-S01]` WHILE `tenants.status` is `pending_approval` or `suspended`, the system SHALL
  resolve **blocked**, and SHALL do so regardless of any subscription the pharmacy holds.
- `[AC-S02]` WHILE an active subscription covers the current instant, the system SHALL
  resolve **full**.
- `[AC-S03]` WHILE the current instant is after `ends_at` but within `ends_at + grace_days`,
  the system SHALL resolve **full_with_warning**.
- `[AC-S04]` WHILE no subscription covers the current instant and the grace period has
  passed, the system SHALL resolve **read_only**.
- `[AC-W01]` IF a request would write while **read_only**, THEN the system SHALL reject it
  with `403 subscription_read_only`, AND a read of the same data SHALL still succeed.
- `[AC-W02]` IF a request arrives while **blocked**, THEN the system SHALL reject it with
  `403 tenant_suspended` or `403 tenant_pending_approval` — the two states are never
  collapsed into one code.
- `[AC-U02]` The access mode SHALL be derived, never stored. No column records it.

### The pharmacy and its branches

- `[AC-E01]` WHEN a pharmacy is created, the system SHALL create exactly one branch marked
  `is_default` and one `tenant_settings` row, in the same transaction as the tenant.
- `[AC-U03]` The system SHALL hold exactly one default branch per pharmacy at all times.
- `[AC-W03]` IF a branch is created with a name another branch of the same pharmacy already
  has, THEN the system SHALL reject it with `409 duplicate_branch_name`.
- `[AC-E02]` WHEN a branch is retired, the system SHALL deactivate it rather than delete it,
  because inventory, sales and devices point at it forever.
- `[AC-W04]` IF the only active branch, or the default branch, would be deactivated, THEN the
  system SHALL reject it with `409 last_active_branch`.

### Approval, suspension and closure — platform only

- `[AC-E03]` WHEN platform staff approve a pharmacy, the system SHALL set `status = active`
  and record `approved_by_platform_user_id` and `approved_at`.
- `[AC-E04]` WHEN platform staff suspend a pharmacy, the system SHALL require a
  `suspension_reason` and SHALL store it.
- `[AC-E05]` WHEN a pharmacy requests closure, the system SHALL set `status = closing` with
  `deletion_requested_at`, AND the purge date SHALL be that instant plus 30 days.
- `[AC-U04]` A purge SHALL keep `subscription_payments` rows with `tenant_id` set to NULL —
  amounts, dates, methods and references survive for accounting; identifying fields do not.
  This is the only place a tenant's deletion does not take its children with it.

### Subscriptions and payments

- `[AC-U05]` Two subscriptions of one pharmacy SHALL NOT overlap in time, enforced by a
  database exclusion constraint and never by application logic.
- `[AC-U06]` `subscriptions.price` SHALL be a copy taken at purchase. A later change to
  `plan_prices` SHALL NOT alter what a pharmacy already paid.
- `[AC-E06]` WHEN a pharmacy submits a payment, the system SHALL accept it only as
  `submitted`; the pharmacy SHALL NOT be able to create or move it to `confirmed`.
- `[AC-E07]` WHEN platform staff confirm a payment, the system SHALL activate the
  subscription it belongs to, in the same transaction.
- `[AC-W05]` IF a payment reuses a `(method, reference)` pair that an unrejected payment
  already holds, THEN the system SHALL reject it with `409 duplicate_payment_reference` — one
  bank slip cannot be used twice.
- `[AC-E08]` WHEN platform staff reject a payment, the system SHALL require a `review_note`.
- `[AC-W06]` IF a complimentary subscription is created without a `grant_reason`, THEN the
  system SHALL reject it — otherwise nothing records *why* it was free.

### Plan catalogue

- `[AC-U07]` `plans`, `plan_prices`, `plan_limit_types` and `plan_limits` SHALL carry no
  `tenant_id`: readable by every authenticated caller, writable only by platform staff.
- `[AC-U08]` A subscription SHALL only exist at a `(plan_id, billing_period)` that exists in
  `plan_prices`.
- `[AC-U09]` Every plan SHALL hold a row for every limit type.
- `[AC-S05]` WHILE a pharmacy is over a limit after a downgrade, the system SHALL let it keep
  what it has and SHALL refuse only further additions.

### Tenant isolation

- `[AC-U10]` A pharmacy SHALL read its own `tenants` row and rows carrying its `tenant_id`,
  and nothing else. Another pharmacy's row SHALL be `404 not_found`, never `403`.
- `[AC-U11]` A platform request SHALL read across tenants only where the policy honours
  `app.is_platform`, which is **these tables only**.

**Offline behaviour.** Server-only, with one exception: `tenant_settings` is read on every
sale, so the phone holds a copy and reads it offline. This module never writes from a phone —
nothing in Context 2 is created offline, so no row here needs a client-generated id for
idempotency's sake. Delivery of that copy is `10-sync`'s pull, not an endpoint here (**Q2**).

---

## 4. Tenancy and permissions

| Question | Answer |
|---|---|
| Tenant-owned? | **Mixed.** `tenants`, `tenant_settings`, `branches`, `subscriptions`, `subscription_payments`, `tenant_exports` are tenant-owned. The four plan tables are platform-owned with no `tenant_id` |
| Branch-scoped? | No. This module *defines* branches; it is not scoped by them |
| Visible to a cashier? | `subscriptions`, `subscription_payments` and `plan_prices` carry money and are hidden from cashier-facing responses. Branch name and `tenant_settings` are readable by anyone signed in |

Permissions this module declares, as `&'static str` on its own crate:

| Slug | Guards |
|---|---|
| `tenant.view` | the pharmacy's own profile and settings |
| `tenant.update` | name, legal name, VAT BIN, contact details |
| `tenant.settings.update` | VAT rate, rounding, receipt text, near-expiry days |
| `branch.view` · `branch.create` · `branch.update` | branches |
| `subscription.view` | the pharmacy's own billing history |
| `subscription.pay` | submitting a payment and its slip |
| `tenant.export` | requesting a data export |
| `tenant.close` | requesting closure |

**Platform permissions are absent from a pharmacy's permission list, not present and
refused** — approval, suspension, payment review and plan editing are platform-token-only,
and `03-identity` holds that token's audience. A pharmacy role editor never shows them.

Article IV applies in full. Specific to this module: the plan tables are the **only** tables
in the system with no `tenant_id` and no tenant policy, and `app.is_platform` is honoured by
**no other module's policies**. Both are deliberate and both are asserted.

---

## 5. Open questions

| # | Question | Blocks | Raised |
|---|---|---|---|
| 1 | Is signup self-serve — a public endpoint creating a `pending_approval` pharmacy — or does platform staff create the row? `[D-82]` settles that approval is required, not who creates the record. | The signup/create-tenant endpoint only. Everything else proceeds | 2026-10-21 |
| 2 | Does the phone receive `tenant_settings` through `10-sync`'s pull, or through a read endpoint here? Settings are read on every sale, so the phone must hold them. | The app-facing settings read only | 2026-10-21 |

**A spec with questions blocking Gate 2 does not advance** — so both are scoped to a single
endpoint each, and [`plan.md`](plan.md) §8 lists what is therefore unplanned. Neither blocks
the tables, the access mode, or the platform surface. **Ask; do not invent.**
