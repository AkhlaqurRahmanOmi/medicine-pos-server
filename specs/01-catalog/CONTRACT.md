# CONTRACT: 01 — Catalogue

**Status:** Draft
**Crate:** `crates/catalog`
**Spec:** [`SPEC.md`](SPEC.md)
**Bounded context:** Context 1 — Catalogue

> Gate 1 artifact. The tables this module owns and the HTTP surface it exposes, copied from
> Data Model Context 1 and the API Design Guide — this is the module's slice of them, not a
> second version. Where they disagree, they win and this is corrected.
>
> The envelope, the error shape and the headers are
> [`../00-foundation/CONTRACT.md`](../00-foundation/CONTRACT.md) §2 and are not restated.

---

## 1. Tables owned

Twelve shared tables with **no `tenant_id`**, and one tenant-owned table. Every one of the
thirteen gets `FORCE ROW LEVEL SECURITY` — a shared table is not an unprotected table.

### Submittable records — approved to all, pending to the submitter

| Table | Keys | Uniques | Notes |
|---|---|---|---|
| `companies` | `id` PK | `name` among pending + approved · `medex_id` | The manufacturer. 354 in the import |
| `generics` | `id` PK | `name` among pending + approved | The active ingredient, and what carries the medical text. 2,192 in the import |
| `products` | `id` PK | `(company_id, brand_name, strength, dosage_form_id)` `NULLS NOT DISTINCT` among pending + approved · `medex_id` | Brand + strength + form + company. A brand name alone never identifies a product — 742 of 17,979 are used by more than one company |

Each of the three carries the same six-column review tail: `status`
(`pending | approved | rejected | merged`), `merged_into_id`, `submitted_by_tenant_id`,
`submitted_by_user_id`, `reviewed_by_user_id`, `reviewed_at`, `review_note`.

**There is no `submissions` table**, and that is deliberate: the status, `merged_into_id` and
the reviewer columns carry the whole lifecycle. A separate table would only be warranted to
group several items into one submission, keep a resubmission history, or hold a conversation
with the submitter — v1 does none of the three.

### Satellites of a generic

| Table | Keys | Notes |
|---|---|---|
| `generic_monographs` | `generic_id` PK, 1:1 | Ten text columns: indications, description, pharmacology, dosage, interactions, contraindications, side effects, pregnancy and lactation, precautions, storage. Kept apart because it is large — this is what turns 127 MB of scraped text into a few MB `[D-22]` |
| `generic_ingredients` | `(generic_id, ingredient_id)` PK | `position` orders the molecules within the generic's name. 5,452 products are combinations |
| `generic_therapeutic_classes` | `(generic_id, therapeutic_class_id)` PK | The class hangs off the generic, never the product — `[AC-U08]` |

### Platform lookups — readable by all, writable by platform only

| Table | Keys | Uniques | Notes |
|---|---|---|---|
| `ingredients` | `id` PK | `name` | One molecule, e.g. Metformin |
| `therapeutic_classes` | `id` PK | `name` | 758 in the import |
| `pack_units` | `id` PK | `name` | piece, strip, box, bottle, vial |
| `dosage_forms` | `id` PK | `name` | tablet, syrup, injection. `base_unit_id` → `pack_units` says what one piece means `[D-6]` |
| `product_packs` | `(product_id, pack_unit_id)` PK | — | `base_units` — e.g. box = 30. **A central default only**, and no price column `[D-34]`. Written by platform staff, plus one narrow `INSERT`: a submitter may add a pack to its own still-`pending` product `[AC-E09]` |

### Tenant-owned — the one exception

| Table | Keys | Notes |
|---|---|---|
| `catalog_issue_reports` | `id` PK, `(tenant_id, id)` UK | `tenant_id`, `FORCE ROW LEVEL SECURITY`, tenant policy. Exactly one of `company_id`, `generic_id`, `product_id` is non-null. `status` is `open \| resolved \| dismissed` |

### Keys and constraints specific to this module

- **Tenant-scoped uniques:** none, and that is the point. Uniqueness here is **global**,
  because the data is shared — two pharmacies must not be able to create two "Napa 500 mg" rows
  by the same company. `SPEC.md` §4.2 says why `Art. IV.4` does not apply.
- **Composite foreign keys:** only `catalog_issue_reports` has a `tenant_id` to compose with,
  and it references nothing tenant-owned, so there are none. Every other FK here is a plain
  `id` into a shared table, which is correct rather than an oversight.
- **Partial unique indexes**, all three `WHERE status IN ('pending','approved')`:
  `companies(name)`, `generics(name)`, and
  `products(company_id, brand_name, strength, dosage_form_id)` with `NULLS NOT DISTINCT`
  because `dosage_form_id` is nullable for a non-medicine. A rejected or merged row must never
  block a later legitimate submission.
- **Check constraints:**

| Constraint | Table | Why it is in the database |
|---|---|---|
| `CHECK ((status = 'merged') = (merged_into_id IS NOT NULL))` | `companies` `generics` `products` | `[AC-U03]`. Two columns that must agree, on three tables, written by two different code paths |
| `CHECK (product_type = 'non_medicine' OR (generic_id IS NOT NULL AND dosage_form_id IS NOT NULL))` | `products` | `[AC-U04]` `[D-8]` |
| `CHECK (num_nonnulls(company_id, generic_id, product_id) = 1)` | `catalog_issue_reports` | `[AC-U05]`. A polymorphic target is already the exception the data model documents; the count is what keeps it honest |
| `CHECK (status <> 'rejected' OR review_note IS NOT NULL)` | `companies` `generics` `products` | `[AC-E05]`. A rejection always records why |
| `CHECK (merged_into_id <> id)` | `companies` `generics` `products` | A row cannot merge into itself. `[AC-W03]`'s other half — a target that is itself merged — needs a query and stays in Rust |

- **Indexes the queries actually need:**
  `gin (brand_name gin_trgm_ops)` on `products` and `gin (name gin_trgm_ops)` on `generics` for
  search `[AC-E08]`; `(status)` on all three submittables for the review queue;
  `(submitted_by_tenant_id)` for a pharmacy's own submissions list;
  `(company_id)` and `(generic_id)` on `products`; `(status, created_at)` on
  `catalog_issue_reports`. `pg_trgm` is the one extension this module adds.
- **Triggers:** none. Nothing here maintains a running total.

### Historical copies

**None.** The catalogue is current-state reference data: there is no price, VAT or cost frozen
at transaction time anywhere in this context. `products.reference_unit_mrp` looks like a frozen
price and is not one — it is the imported medex suggestion, overwritten freely by platform
staff, and **nothing is ever sold at it**. The price that must not move is copied onto the
batch by **06 Inventory** and onto the sale line by **07 Sales**.

### Grants

| Role | Rights |
|---|---|
| `pos_app` | `SELECT` on all thirteen. `INSERT` on `companies`, `generics`, `products` — a pending submission — and `INSERT` on `catalog_issue_reports`. **No `UPDATE` on any shared table**, because a pharmacy never edits central data: it files an issue report `[AC-U07]` |
| `pos_app` + `app.is_platform` | the same, plus `INSERT`/`UPDATE` on every shared table and `UPDATE` on `catalog_issue_reports` to resolve or dismiss one |
| `pos_jobs` | `SELECT` only. No job in this module writes |

**No `DELETE` for anybody.** A catalogue row is rejected or merged, never removed — stock and
sales elsewhere point at it forever.

---

## 2. HTTP surface

All paths under `/api/v1`. Conventions, envelope and error shape:
[`../00-foundation/CONTRACT.md`](../00-foundation/CONTRACT.md) §2.

### Pharmacy-facing

| Method | Path | Permission | Response | Errors |
|---|---|---|---|---|
| `GET` | `/catalog/search` | `catalog.view` | `ListPage<ProductSummary>` — trigram over brand name and generic name, approved plus the caller's own pending | `400 validation_failed` on an empty `q` |
| `GET` | `/catalog/products` | `catalog.view` | `ListPage<ProductSummary>`, filterable by `company_id`, `generic_id`, `product_type` | — |
| `GET` | `/catalog/products/{id}` | `catalog.view` | the product with its company, generic name, dosage form and central packs | `404 not_found` |
| `GET` | `/catalog/generics/{id}` | `catalog.view` | the generic with its ingredients and therapeutic classes — **no monograph** | `404 not_found` |
| `GET` | `/catalog/generics/{id}/monograph` | `catalog.view` | the ten text fields, or `204` when the generic has none | `404 not_found` |
| `GET` | `/catalog/companies` | `catalog.view` | `ListPage<Company>` | — |
| `GET` | `/catalog/lookups` | `catalog.view` | dosage forms, pack units and therapeutic classes in one response — three small fixed lists the client needs before it can render a submission form | — |
| `POST` | `/catalog/companies` | `catalog.submit` | `201` the company, `pending` | `409 duplicate_company_name` |
| `POST` | `/catalog/generics` | `catalog.submit` | `201` the generic, `pending` | `409 duplicate_generic_name` |
| `POST` | `/catalog/products` | `catalog.submit` | `201` the product, `pending`, with its pack when one was given | `409 duplicate_product`, `400 validation_failed`, `404 not_found` for an unknown `pack_unit_id` |
| `GET` | `/catalog/submissions` | `catalog.submit` | `ListPage<Submission>` — the caller's own, any status, with `review_note` and `merged_into_id` when set | — |
| `POST` | `/catalog/issues` | `catalog.issue.report` | `201` the report, `open` | `400 validation_failed` |
| `GET` | `/catalog/issues` | `catalog.issue.report` | `ListPage<IssueReport>` — the caller's own | — |

A submission is always created `pending` and the request body carries no `status` — the same
shape as `POST /subscription-payments` in module 02, and for the same reason.

`POST /catalog/products` takes an **optional** `pack` — `{pack_unit_id, base_units}` — and
stores it as the product's central default in the submission's transaction `[AC-E09]`. It is
not in the required set `[AC-W02]`: Product Specification §4 makes a pharmacy's own pack sizes
its own, so the submitter is offering a fact about the box, not setting a price `[D-34]`. A
submission with no `pack` is complete.

`GET /catalog/submissions` exists even though a pending row is also returned by
`/catalog/products`: a pharmacy needs one place to see what it has asked for and what came
back, including the rejected and merged rows that search deliberately hides `[AC-S03]`.

### Platform-facing — `/api/v1/admin`, platform token only

| Method | Path | Response | Errors |
|---|---|---|---|
| `GET` | `/admin/catalog/queue` | `ListPage<QueueItem>`, filterable by `kind` (`company \| generic \| product`) and `status`, oldest first. Each item carries `suggestions` — up to five similar existing rows, most similar first, advisory only `[AC-E10]` | — |
| `POST` | `/admin/catalog/{kind}/{id}/approve` | the row, now `approved` | `409 already_reviewed` |
| `POST` | `/admin/catalog/{kind}/{id}/reject` | the row, now `rejected` | `400 validation_failed` with no note, `409 already_reviewed` |
| `POST` | `/admin/catalog/{kind}/{id}/merge` | the row, now `merged`, with `merged_into_id` | `409 invalid_merge_target`, `409 already_reviewed` |
| `PATCH` | `/admin/catalog/products/{id}` | the product — platform staff are the only editors of central data | `404 not_found`, `409 duplicate_product` |
| `PATCH` | `/admin/catalog/generics/{id}` | the generic, its ingredients and classes replaced as given | `404 not_found` |
| `PUT` | `/admin/catalog/generics/{id}/monograph` | the monograph, written with the generic in one transaction `[AC-E07]` | `404 not_found` |
| `PUT` | `/admin/catalog/products/{id}/packs/{pack_unit_id}` | the central pack default | `404 not_found` for an unknown pack unit |
| `GET` `POST` | `/admin/catalog/ingredients` · `PATCH` `/{id}` | `ListPage<Ingredient>` / the ingredient | `409 duplicate_name` |
| `GET` `POST` | `/admin/catalog/therapeutic-classes` · `PATCH` `/{id}` | likewise | `409 duplicate_name` |
| `GET` `POST` | `/admin/catalog/dosage-forms` · `PATCH` `/{id}` | likewise, with `base_unit_id` | `409 duplicate_name`, `404 not_found` for an unknown pack unit |
| `GET` `POST` | `/admin/catalog/pack-units` · `PATCH` `/{id}` | likewise | `409 duplicate_name` |
| `GET` | `/admin/catalog/issues` | `ListPage<IssueReport>` across all pharmacies, filterable by `status` | — |
| `POST` | `/admin/catalog/issues/{id}/resolve` · `/dismiss` | the report, with `admin_note` | `409 already_closed` |

Two shapes are deliberate exceptions to house convention, both for the reason module 02 gave
for `PUT` on a plan limit — the row is a single value at a known key, so there is nothing
partial about setting it: `PUT …/monograph` and `PUT …/packs/{pack_unit_id}`.

`{kind}` in the three review paths is a path enum, not a free string: one handler for three
tables would be a polymorphic write, so it is three handlers behind one path pattern.

**Near-duplicate suggestions** on the queue are trigram similarity over `brand_name`, read
from the `pg_trgm` index the search surface already needs `[AC-E10]`. `QueueItem.suggestions`
is `Vec<SuggestedMatch>` — `{id, name, similarity}`, at most five, strongest first, and empty
when nothing clears the threshold. A reviewer still decides: the queue never pre-selects or
performs a merge.

### Error cases

| Condition | Status | Code | Message to the user |
|---|---|---|---|
| Brand already exists for that company, strength and form | 409 | `duplicate_product` | "This product is already in the catalogue. Search for it instead of adding it." |
| Company name already submitted or approved | 409 | `duplicate_company_name` | "A manufacturer with this name already exists." |
| Generic name already submitted or approved | 409 | `duplicate_generic_name` | "A generic with this name already exists." |
| Medicine submitted without company, generic, strength or form | 400 | `validation_failed` | "Fill in the manufacturer, generic, strength and dosage form." — with `details` naming each missing field |
| Review action on a row already reviewed | 409 | `already_reviewed` | "This submission has already been reviewed." |
| Merge target is merged, or is the row itself | 409 | `invalid_merge_target` | "Pick a product that is still active to merge into." |
| Issue report with no target, or more than one | 400 | `validation_failed` | "Choose the one product, generic or manufacturer this report is about." |
| Pending row of another pharmacy | 404 | `not_found` | "Not found." `[AC-W06]` |
| Lookup write without the platform token | 403 | `permission_denied` | "Only platform staff can change this." |

---

## 3. Public Rust surface

`crates/catalog` — depends on `core`, `db`, `api`. **No `cache`**: nothing here is hot enough
to cache behind Redis, and the catalogue is already the cheapest read in the system. Depends on
**no module crate** `[D-97]`.

```rust
// queries.rs — reads, take &Db
pub async fn search(db: &Db, tenant: TenantId, q: &str, page: Pagination) -> Result<ListPage<ProductSummary>, AppError>;
pub async fn product(db: &Db, tenant: TenantId, id: ProductId) -> Result<Product, AppError>;
pub async fn generic(db: &Db, tenant: TenantId, id: GenericId) -> Result<Generic, AppError>;
pub async fn monograph(db: &Db, id: GenericId) -> Result<Option<Monograph>, AppError>;
pub async fn companies(db: &Db, tenant: TenantId, page: Pagination) -> Result<ListPage<Company>, AppError>;
pub async fn lookups(db: &Db) -> Result<Lookups, AppError>;
pub async fn submissions(db: &Db, tenant: TenantId, page: Pagination) -> Result<ListPage<Submission>, AppError>;

// commands.rs — writes, take &Tx
pub async fn submit_company(tx: &Tx, input: NewCompany) -> Result<Company, AppError>;
pub async fn submit_generic(tx: &Tx, input: NewGeneric) -> Result<Generic, AppError>;
pub async fn submit_product(tx: &Tx, input: NewProduct) -> Result<Product, AppError>;
pub async fn report_issue(tx: &Tx, input: NewIssueReport) -> Result<IssueReport, AppError>;

// platform.rs — platform-token writes, each asserting app.is_platform is set
pub async fn approve(tx: &Tx, kind: RecordKind, id: Uuid, by: UserId) -> Result<Reviewed, AppError>;
pub async fn reject(tx: &Tx, kind: RecordKind, id: Uuid, by: UserId, note: String) -> Result<Reviewed, AppError>;
pub async fn merge(tx: &Tx, kind: RecordKind, id: Uuid, into: Uuid, by: UserId) -> Result<Reviewed, AppError>;
pub async fn upsert_monograph(tx: &Tx, id: GenericId, input: MonographInput) -> Result<Monograph, AppError>;

// sellable.rs — the one question every other module asks of this one
pub async fn is_sellable(db: &Db, tenant: TenantId, id: ProductId) -> Result<bool, AppError>;

// perms.rs
pub mod perms { pub const VIEW: &str = "catalog.view"; /* … */ }
```

`repo.rs` and `entity/` are private. No other crate reaches the entities or the SQL.

**Called by:** `server` mounts the routers. **06 Inventory** and **07 Sales** call
`is_sellable` before they accept a batch or a sale line against a product — the one piece of
this module's surface another context genuinely needs, and the reason it is a named function
rather than a join across a module boundary. **09 Workflows** calls `merge` alongside the
stock and sales moves that `[D-2]` requires, in one `Tx`.

### What this module needs from another

**Nothing.** Every mechanism it uses — `Tx::begin`, `Tx::begin_platform`, `AuthUser`,
`PlatformToken`, `api::require`, `ListPage`, `Pagination` — exists today, shipped by modules 00
and 03. This is the first module in the build order that adds no row to another module's
contract, which is what being downstream of a finished foundation is supposed to feel like.

---

## 4. Events and jobs

| Trigger | Job | Owner |
|---|---|---|
| — | **none** | — |

This module schedules nothing and is woken by nothing. There is no expiry to sweep, no total
to recompute, no notification to send: a submission waits in the queue until a human looks at
it, and a queue depth is a report, not a job. The one-time import is **Q1**, and a one-time
import is not a job either.

---

## 5. Offline and sync

| Question | Answer |
|---|---|
| Can the mobile app create this offline? | **No.** Offline scope is sales only `[D-24]`. Every write here needs a connection |
| Client-generated id? | Server-assigned UUIDv7. With no offline create, there is no upload to make idempotent |
| What makes an upload idempotent | Not applicable. The nearest thing is the partial unique index, which makes a double-submitted product a `409` rather than a second queue entry |
| Can it be flagged rather than rejected? | No. Nothing here is written under time pressure at a counter |

| Resource | Direction | Notes |
|---|---|---|
| products, companies, generics, dosage forms, pack units, central packs | pull | The lean copy the phone searches offline. Delivery and refresh are **10 Sync**'s contract |
| `generic_monographs` | **neither** | Online-only `[D-22]`, cached per generic once opened `[D-27]`. 127 MB cannot ship to a phone, and this is why the text is its own table and its own endpoint |
| `catalog_issue_reports` | neither | Filed from the app while online, read in the admin panel |

A pending row the pharmacy submitted is part of its lean copy — it is sellable to them
`[D-1]`, so a device that cannot see it cannot sell it. A row that is later **rejected or
merged** must therefore disappear from, or be rewritten in, a device's copy; **10 Sync** owns
that, and it is the one real coupling between this module and the phone.
