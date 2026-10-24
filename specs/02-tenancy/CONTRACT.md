# CONTRACT: 02 — Tenancy

**Crate:** `crates/tenancy`
**Spec:** [`SPEC.md`](SPEC.md)
**Bounded context:** Context 2 — Tenancy and plans

Tables, HTTP surface and public Rust surface. The envelope, the error shape and the headers
are [`../00-foundation/CONTRACT.md`](../00-foundation/CONTRACT.md) §2 and are not restated.

---

## 1. Tables owned

### Tenant-owned — `tenant_id`, `FORCE ROW LEVEL SECURITY`, tenant policy

| Table | Keys | Tenant-scoped uniques | Notes |
|---|---|---|---|
| `tenants` | `id` PK | — | The row *is* the tenant: `id` is the `tenant_id` every other table carries. `status` is `pending_approval \| active \| suspended \| closing` |
| `tenant_settings` | `tenant_id` PK, 1:1 | — | Typed columns: `vat_rate`, `vat_included`, `rounding_mode`, `near_expiry_days`, `default_payment_terms_days`, `allow_zero_stock_sale`, `receipt_header`, `receipt_footer`, `receipt_paper_width` |
| `branches` | `id` PK, `(tenant_id, id)` UK | `(tenant_id, name)` | `(tenant_id, id)` exists so other contexts can reference a branch with a composite key. `drug_license_no` lives here — each shop is licensed separately |
| `subscriptions` | `id` PK, `(tenant_id, id)` UK | — | One row per paid period. `price` is a copy taken at purchase |
| `subscription_payments` | `id` PK | `(method, reference)` among unrejected rows — **not** tenant-scoped | `tenant_id` is **nullable** and `ON DELETE SET NULL` |
| `tenant_exports` | `id` PK, `(tenant_id, id)` UK | — | The request and its state. `12-jobs` fills `file_key`, `ready_at`, `expires_at` |

`tenants.vat_bin` is on the tenant because the BIN belongs to the business;
`branches.drug_license_no` is on the branch because each shop is licensed separately.

### Platform-owned — no `tenant_id`, no tenant policy

| Table | Keys | Uniques | Notes |
|---|---|---|---|
| `plans` | `id` PK | `code` | `grace_days` is read by the access-mode rule |
| `plan_prices` | `(plan_id, billing_period)` PK | — | One plan sells monthly and yearly at different prices |
| `plan_limit_types` | `key` PK | — | `branches`, `users`, `devices` |
| `plan_limits` | `(plan_id, limit_key)` PK | — | `max_value` NULL means unlimited `[D-89]` |

**These four are the only tables in the system without `tenant_id`.** RLS is enabled with a
read-to-all, write-to-platform policy rather than left off, so "no policy" never reads as an
oversight. Everyone reads them; only `app.is_platform` writes.

### Constraints the database enforces, not the code

| Constraint | Table | Why it is in the database |
|---|---|---|
| Partial unique index on `(tenant_id) WHERE is_default` | `branches` | Exactly one default branch. Application logic cannot hold this under concurrency |
| `EXCLUDE USING gist (tenant_id WITH =, tstzrange(starts_at, ends_at) WITH &&)` | `subscriptions` | No two subscriptions of one pharmacy overlap. Needs `btree_gist` |
| `CHECK (ends_at > starts_at)` | `subscriptions` | — |
| `(plan_id, billing_period)` FK → `plan_prices` | `subscriptions` | A subscription cannot exist at a price that does not |
| Partial unique index on `(method, reference) WHERE status <> 'rejected'` | `subscription_payments` | One bank slip or bKash transaction cannot be used twice |
| `CHECK (NOT is_complimentary OR grant_reason IS NOT NULL)` | `subscriptions` | A free period always records why `[D-81]` |
| `CHECK (status <> 'suspended' OR suspension_reason IS NOT NULL)` | `tenants` | A suspension always records why `[D-83]` |

### Grants

| Role | Rights |
|---|---|
| `pos_app` | `SELECT` on all; `INSERT`/`UPDATE` on `tenants`, `tenant_settings`, `branches`, `tenant_exports`; `INSERT` only on `subscription_payments`; **no rights at all** on the four plan tables beyond `SELECT` |
| `pos_app` + `app.is_platform` | the same grants, with policies that no longer filter by `tenant_id`, plus `UPDATE` on `subscription_payments` and write access to the plan tables |
| `pos_jobs` | `SELECT` and `UPDATE` on `tenant_exports`; `UPDATE` on `tenants` for the purge; `UPDATE` on `subscription_payments` to null `tenant_id` at purge |

**No `DELETE` for `pos_app` anywhere in this module.** Closure is a status change, branches
deactivate, and the purge belongs to `pos_jobs`.

---

## 2. HTTP surface

All paths are under `/api/v1`. Conventions, envelope and error shape:
[`../00-foundation/CONTRACT.md`](../00-foundation/CONTRACT.md) §2.

### Pharmacy-facing

| Method | Path | Permission | Response | Errors |
|---|---|---|---|---|
| `GET` | `/tenant` | `tenant.view` | the pharmacy, its settings, and its current access mode | — |
| `PATCH` | `/tenant` | `tenant.update` | the updated pharmacy | `400 validation_failed` |
| `GET` | `/tenant/settings` | `tenant.view` | `tenant_settings` | — |
| `PATCH` | `/tenant/settings` | `tenant.settings.update` | the updated settings | `400 validation_failed` |
| `GET` | `/branches` | `branch.view` | `ListPage<Branch>` | — |
| `POST` | `/branches` | `branch.create` | `201` the branch | `409 duplicate_branch_name`, `409 plan_limit_reached` |
| `GET` | `/branches/{id}` | `branch.view` | the branch | `404 not_found` |
| `PATCH` | `/branches/{id}` | `branch.update` | the branch | `409 duplicate_branch_name` |
| `POST` | `/branches/{id}/deactivate` | `branch.update` | the branch, now inactive | `409 last_active_branch` |
| `GET` | `/plans` | authenticated | `ListPage<Plan>` with prices and limits | — |
| `GET` | `/subscriptions` | `subscription.view` | `ListPage<Subscription>` — billing history | — |
| `GET` | `/subscriptions/current` | `subscription.view` | the covering subscription, or `204` | — |
| `POST` | `/subscription-payments` | `subscription.pay` | `201` the payment, always `submitted` | `409 duplicate_payment_reference` |
| `POST` | `/subscription-payments/{id}/slip` | `subscription.pay` | the payment with `proof_file_key` | `413 file_too_large` |
| `POST` | `/exports` | `tenant.export` | `202` with an id | `409 export_already_running` |
| `GET` | `/exports/{id}` | `tenant.export` | state, and a short-lived signed URL once `ready` | `404 not_found` |
| `POST` | `/tenant/closure` | `tenant.close` | the pharmacy, now `closing`, with its purge date | `409 already_closing` |

A deactivated branch is still returned by `GET /branches`, with `is_active: false`. A client
that filtered on absence would lose the history its sales point at.

### Platform-facing — `/api/v1/admin`, platform token only

| Method | Path | Response | Errors |
|---|---|---|---|
| `GET` | `/admin/tenants` | `ListPage<TenantSummary>`, filterable by `status` | — |
| `GET` | `/admin/tenants/{id}` | the pharmacy, its settings, branches and subscriptions | `404 not_found` |
| `POST` | `/admin/tenants/{id}/approve` | the pharmacy, now `active` | `409 not_pending_approval` |
| `POST` | `/admin/tenants/{id}/suspend` | the pharmacy, now `suspended` | `400 validation_failed` when no reason |
| `POST` | `/admin/tenants/{id}/reinstate` | the pharmacy, now `active` | `409 not_suspended` |
| `POST` | `/admin/tenants/{id}/subscriptions` | `201` the subscription | `409 subscription_overlap`, `400 validation_failed` when complimentary without a reason |
| `GET` | `/admin/subscription-payments` | `ListPage<Payment>`, filterable by `status` | — |
| `POST` | `/admin/subscription-payments/{id}/confirm` | the payment, and the subscription it activated | `409 already_reviewed` |
| `POST` | `/admin/subscription-payments/{id}/reject` | the payment, now `rejected` | `400 validation_failed` when no note |
| `GET` `POST` | `/admin/plans` · `/admin/plans/{id}` `PATCH` | the plan with its prices and limits | `409 duplicate_plan_code` |
| `PUT` | `/admin/plans/{id}/limits/{limit_key}` | the limit | `404 not_found` for an unknown limit type |

`PUT` on a plan limit is the one exception to "`PATCH`, never `PUT`": the row is a single
value keyed by `(plan_id, limit_key)`, so there is nothing partial about setting it.

**Not specified** — the endpoint that creates a pharmacy. See `SPEC.md` §5 **Q1**.

---

## 3. Public Rust surface

`crates/tenancy` — depends on `core`, `db`, `api`, `cache`. Depends on **no module crate**.

```rust
// access.rs — the value every other module's requests are gated on
pub enum AccessMode { Blocked(BlockReason), ReadOnly, Full, FullWithWarning { until: DateTime<Utc> } }
pub enum BlockReason { PendingApproval, Suspended }

impl AccessMode {
    pub fn permits_write(&self) -> bool;
    pub fn error(&self) -> Option<AppError>;   // the 403 to return, already coded
}

// queries.rs — reads, take &Db
pub async fn access_mode(db: &Db, tenant: TenantId, now: DateTime<Utc>) -> Result<AccessMode, AppError>;
pub async fn tenant(db: &Db, tenant: TenantId) -> Result<Tenant, AppError>;
pub async fn settings(db: &Db, tenant: TenantId) -> Result<TenantSettings, AppError>;
pub async fn branches(db: &Db, tenant: TenantId, page: Pagination) -> Result<ListPage<Branch>, AppError>;
pub async fn limit_remaining(db: &Db, tenant: TenantId, key: &str) -> Result<Option<i32>, AppError>;

// commands.rs — writes, take &Tx
pub async fn create_branch(tx: &Tx, input: NewBranch) -> Result<Branch, AppError>;
pub async fn deactivate_branch(tx: &Tx, id: BranchId) -> Result<Branch, AppError>;
pub async fn update_settings(tx: &Tx, input: SettingsPatch) -> Result<TenantSettings, AppError>;
pub async fn submit_payment(tx: &Tx, input: NewPayment) -> Result<Payment, AppError>;
pub async fn request_closure(tx: &Tx, by: UserId) -> Result<Tenant, AppError>;

// platform.rs — platform-token writes, each one asserting app.is_platform is set
pub async fn approve(tx: &Tx, tenant: TenantId, by: UserId) -> Result<Tenant, AppError>;
pub async fn suspend(tx: &Tx, tenant: TenantId, reason: String) -> Result<Tenant, AppError>;
pub async fn confirm_payment(tx: &Tx, id: PaymentId) -> Result<(Payment, Subscription), AppError>;

// perms.rs
pub mod perms { pub const VIEW: &str = "tenant.view"; /* … */ }
```

`repo.rs` is **private**. No other crate reaches the entities or the SQL.

### What this module needs from `00-foundation`

Two additions, because the mechanism is foundation's and the meaning is this module's:

| Addition | Where | Why it cannot live here |
|---|---|---|
| `Tx::begin_platform` — sets `app.is_platform` alongside the tenant settings | `crates/db` | `set_config` appears in exactly one file in the workspace, by Article IV.2 and the assertion that proves it |
| `trait AccessModeSource` | `crates/core/ports.rs` | `api`'s write-gate layer must not depend on a module crate. `api` takes the trait, `tenancy` implements it, `server` wires it — the same shape as `MailSender` |

Both change a Gate-5-complete module, so both carry a task here **and** an update to
[`../00-foundation/CONTRACT.md`](../00-foundation/CONTRACT.md) §3 in the same change.

---

## 4. Events and jobs

| Trigger | Job | Owner |
|---|---|---|
| `POST /exports` | build the archive, set `file_key`, `ready_at`, `expires_at` | `12-jobs` |
| nightly | lapse sweep: nothing to write — access mode is derived, so a lapse needs no job | — |
| nightly | purge pharmacies whose `deletion_requested_at` is older than 30 days, nulling `subscription_payments.tenant_id` | `12-jobs` |
| nightly | expire `tenant_exports` past `expires_at` | `12-jobs` |

**No job changes a tenant's status on its own**, and no job computes access mode. A lapse is
the absence of a covering subscription, not an event — which is why there is nothing to
schedule and nothing to go stale.

---

## 5. Offline and sync

| Resource | Direction | Notes |
|---|---|---|
| `tenant_settings` | pull | Read on every sale, so the phone holds a copy. Delivery mechanism is **Q2** |
| `branches` | pull | The phone needs branch names and the device's branch |
| everything else | neither | Nothing in Context 2 is created on a phone, so nothing here needs a client-generated id |

Access mode is **not** synced. A phone that has been offline for a week cannot know its
pharmacy was suspended yesterday; the answer arrives when it next reaches the API, and the
upload is rejected then.
