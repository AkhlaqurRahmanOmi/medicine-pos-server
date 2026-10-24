# CONTRACT: 00 — Foundation

**Status:** Draft
**Crates:** `crates/core` · `crates/db` · `crates/api` · `crates/cache` · `crates/infra`

> Gate 1 artifact. Foundation is unusual: it owns **no tables**. Its contract is the Rust
> surface every other module compiles against, plus three unauthenticated routes.

---

## 1. Tables owned

**None.** Every table in the system belongs to a domain module.

What this module owns instead are the *conventions* those tables must follow, and the
session settings their policies read:

| Setting | Type | Set by | Read by |
|---|---|---|---|
| `app.tenant_id` | text (uuid) | `db::Tx::begin` | every RLS policy |
| `app.user_id` | text (uuid) | `db::Tx::begin` | audit triggers |
| `app.branch_ids` | text | `db::Tx::begin` | branch-scoped policies |

All three are set with `set_config(name, $1, true)` — bound parameter, transaction-local.
**No other crate may call `set_config`.** This is assertable with a grep, and it is.

### Roles

| Role | Used by | RLS |
|---|---|---|
| `pos_owner` | `crates/migration` only | owner — bypasses RLS, which is why the server never uses it |
| `pos_app` | the running server | **subject to RLS**, no `BYPASSRLS`, not the table owner |
| `pos_jobs` | `crates/jobs` | `BYPASSRLS`, for cross-tenant sweeps |

Two separate connection URLs in the config, so using the wrong one is a configuration error
rather than a silent data leak.

---

## 2. HTTP surface

| Method | Path | Auth | Response | Notes |
|---|---|---|---|---|
| `GET` | `/health` | none | `200` | Liveness. Touches nothing. Must answer while the database is down |
| `GET` | `/health/ready` | none | `200` / `503` | Readiness. Checks the database and Redis |
| `GET` | `/openapi.json` | none | `200` | utoipa document, assembled from every module |
| `GET` | `/docs` | none | `200` | Scalar, rendering that document. The spec is inlined in the page; **the renderer itself loads from `cdn.jsdelivr.net`, so `/docs` needs the internet even though the API does not** |

**How the document is assembled.** `api::ApiDoc` is the root — title, tags, the `bearer`
scheme — and carries only the two `/health` paths it owns. Every module crate exports an
`OpenApiRouter` and registers handlers with `utoipa_axum::routes!`, which ties a route to its
`#[utoipa::path]`: **a route cannot be registered without its documentation.** `crates/server`
nests those routers onto the root and calls `split_for_parts()`, so the router that is mounted
and the document that is served are the same object — a path cannot be in one and not the
other. Each handler declares `security(("bearer" = []))`, which is what gives Scalar's "try
it" panel a token field.

### The envelope

**There is no success wrapper.** A successful response is the resource itself. This is the
API Design Guide's shape (Notion, §§2–5), which replaced the reference server's
`{success, message, data, meta}` when Q1 was answered.

```jsonc
// 201 create · 200 update, transition, or a repeat create with the same id · 204 nothing to return
{ "id": "019a…", "invoice_number": "C1-000412", "total": "340.00" }

// list — cursor, never page/offset: ids are UUIDv7 and already sort by creation time
{ "data": [ { "id": "019a…" } ], "next_cursor": "019a…", "has_more": true }

// error — one shape, every status
{ "error": { "code": "over_credit_limit",
             "message": "This sale would take the customer past their credit limit.",
             "details": { "credit_limit": "5000.00" },
             "trace_id": "019a3f…" } }
```

| Rule | Why it is this way |
|---|---|
| `limit` defaults to 50, capped at 200 | — |
| **No total count on any list** | Counting every matching row is the query that gets slow first, and no screen needs it. Reports give totals; lists give rows |
| `next_cursor` is `null` at the end, and `has_more` says it too | A client that forgets one will check the other |
| Money is a **string** — `"12.50"` | JavaScript rounds floats, and the column is `numeric(12,2)` |
| Field names are `snake_case` in bodies, hyphens in paths | serde's output with no renaming, so database, API and OpenAPI use the same word |
| Unknown request fields are **rejected**, not ignored | A typo fails loudly instead of silently doing nothing |
| Validation errors list fields in `details.fields` | Keyed by the same `snake_case` name the request used |

**`message` is English, for logs and support.** Each client maps `code` to its own Bengali or
English text, because the app must show the right sentence while offline, where there is no
API to ask. Nothing may match on `message`.

### Headers this crate owns

| Header | Direction | Rule |
|---|---|---|
| `X-Trace-Id` | out | **Always**, on every response, and the same value as the body's `trace_id` |
| `X-Request-Id` | in, optional | Echoed and logged if the client sends one; otherwise the server generates it |
| `Cache-Control: private, no-store` | out | On every tenant-scoped response. A cache that defaulted to `public` could serve one pharmacy's data to another |

`RequestContext.request_id` is the internal name for this value; `trace_id` and `X-Trace-Id`
are its names on the wire.

### Error codes

Stable, machine-readable, and the only thing a client may branch on. The `message` is for a
person and may change without notice.

Codes are **`lower_snake`**, not `SCREAMING_CASE` — the guide's form, and the one both
clients branch on.

| Code | Status |
|---|---|
| `bad_request`, `validation_failed`, `unknown_field` | 400 |
| `invalid_token`, `token_expired`, `device_revoked` | 401 |
| `permission_denied`, `branch_not_allowed`, `subscription_read_only`, `tenant_suspended`, `tenant_pending_approval` | 403 |
| `not_found` | 404 |
| *module-owned conflict codes* — `insufficient_stock`, `batch_expired`, `duplicate_supplier_invoice`, `already_returned`, `credit_note_spent` | 409 |
| `file_too_large` | 413 |
| `app_too_old` | 426 |
| `too_many_requests` | 429 |
| `internal_error` | 500 |

Four rules the guide is explicit about:

- **`validation_failed` is 400, not 422.**
- **A lapsed subscription is 403 `subscription_read_only`, not 402.** The app shows a renewal
  banner; `tenant_suspended` shows nothing else at all. Different codes, different screens.
- **Another pharmacy's row is 404, never 403.** A 403 confirms the id exists, which is the
  leak row-level security exists to prevent.
- **A denied permission names what it needed** in `details.required`, so the app can say
  "ask your manager" rather than "forbidden".

**409 codes belong to the module that raises them**, not to `core` — `core` cannot know what a
batch is. They travel as `&'static str`, exactly as permission slugs do.
| `APP_VERSION_UNSUPPORTED` | 426 |
| `INTERNAL` | 500 |

The last four are ours rather than the reference's, and each maps to a system-state screen in
the mobile app (S52).

---

## 3. Public Rust surface

### `core` — no dependencies on anything else in the workspace

```rust
pub struct Config { /* parsed once at boot, exits non-zero on error */ }

pub struct TenantContext { tenant_id: TenantId, user_id: UserId, branch_ids: Vec<BranchId> }
pub struct RequestContext { request_id: RequestId, now: DateTime<Utc> }

pub struct TenantId(Uuid);  pub struct BranchId(Uuid);  pub struct UserId(Uuid);
pub struct RoleId(Uuid);    pub struct DeviceId(Uuid);  pub struct SessionId(Uuid);
pub struct Money(Decimal);  // never f64, at any layer

pub enum AppError { … }     // user message + log detail + ErrorCode
pub enum ErrorCode { … }    // the table above

pub trait Clock { fn now(&self) -> DateTime<Utc>; }
pub trait MailSender { … }
pub trait FileStorage { … }
pub trait AccessModeSource: Send + Sync { … }
pub trait PermissionSource: Send + Sync { … }

pub struct Page<T> { items: Vec<T>, total: u64, page: u32, page_size: u32 }
```

### `db` — depends on `core`

```rust
pub struct Db(DatabaseConnection);   // reads — queries take this
pub struct Tx<'a>(DatabaseTransaction); // writes — commands take this

impl Db { pub fn pool(&self) -> &PgPool; }  // for sqlx query_as! macros [D-99]
impl Tx<'_> {
    pub async fn begin(db: &Db, ctx: &TenantContext) -> Result<Self, AppError>;
    pub async fn begin_platform(db: &Db, ctx: &TenantContext) -> Result<Self, AppError>;
}
```

`Tx::begin` and `Tx::begin_platform` in `crates/db/src/tx.rs` are the **only** places `set_config` appears.

### `api` — depends on `core`, `db`

```rust
pub struct ApiOk<T>(T);          // impl IntoResponse — the envelope IS the return type
pub struct AuthUser {
    pub user_id: UserId,
    pub tenant_id: TenantId,
    pub permissions: Vec<String>,
    pub branch_permissions: BTreeMap<BranchId, Vec<String>>,
}
pub struct TenantTx(pub Tx);     // extractor; commits on Ok, rolls back on Err
pub struct Pagination { page: u32, page_size: u32 }
pub fn require(slug: &'static str) -> impl Layer<…>;
pub fn require_at<B: BranchResolver>(slug: &'static str, resolver: B) -> impl Layer<…>;
pub fn auth_layer<A: Authenticator>(authenticator: A) -> impl Layer<…>;
```

### `cache` — depends on `core`

```rust
pub struct Cache(deadpool_redis::Pool);
impl Cache {
    pub async fn rate_limit(&self, key: &str, limit: u32, window: Duration) -> Result<Allowed, AppError>;
    pub async fn idempotency_guard(&self, key: &str) -> Result<Guard, AppError>;
}
```

### `infra` — depends on `core` only

Concrete adapters for the `core` traits: SMTP mail, S3-compatible storage, `SystemClock`.
Exists as its own crate so that `jobs` can send a subscription-lapse email without `sales`
transitively depending on an SMTP library.

**Called by:** `server` and `jobs` construct them; nothing else names a vendor SDK.

---

## 4. Events and jobs

| Trigger | Job | Schedule | Idempotent? |
|---|---|---|---|
| — | — | — | — |

None. Foundation starts no work of its own. `12-jobs` owns the scheduler and uses these
crates.

---

## 5. Offline and sync

| Question | Answer |
|---|---|
| Can the mobile app create this offline? | Not applicable — no entities |
| Client-generated id? | Foundation defines `Uuid` v7 as the id type; `10-sync` defines what is deduplicated |
| What makes an upload idempotent | `cache::idempotency_guard` provides the mechanism; the key is `10-sync`'s decision |
| Can it be flagged rather than rejected? | Not applicable |
