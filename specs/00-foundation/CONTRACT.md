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

### The envelope

```jsonc
// success
{ "success": true,  "message": "…", "data": { }, "meta": { "requestId": "…" } }

// error
{ "success": false, "message": "…", "errors": [ { "field": "…", "code": "…", "detail": "…" } ],
  "meta": { "requestId": "…", "code": "VALIDATION_FAILED" } }
```

Pagination adds `meta.pagination = { total, page, pageSize }`.

> **Open question 1** in [`SPEC.md`](SPEC.md): confirm this against the API Design Guide
> before `api` reaches Gate 2. The shape above is taken from the reference server, not from
> our own guide, and the guide wins.

### Error codes

Stable, machine-readable, and the only thing a client may branch on. The `message` is for a
person and may change without notice.

| Code | Status |
|---|---|
| `BAD_REQUEST` | 400 |
| `VALIDATION_FAILED` | 422 |
| `UNAUTHORIZED` | 401 |
| `FORBIDDEN` | 403 |
| `NOT_FOUND` | 404 |
| `CONFLICT` | 409 |
| `RATE_LIMITED` | 429 |
| `TENANT_SUSPENDED` | 403 |
| `SUBSCRIPTION_LAPSED` | 402 |
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
pub struct Money(Decimal);  // never f64, at any layer

pub enum AppError { … }     // user message + log detail + ErrorCode
pub enum ErrorCode { … }    // the table above

pub trait Clock { fn now(&self) -> DateTime<Utc>; }
pub trait MailSender { … }
pub trait FileStorage { … }

pub struct Page<T> { items: Vec<T>, total: u64, page: u32, page_size: u32 }
```

### `db` — depends on `core`

```rust
pub struct Db(DatabaseConnection);   // reads — queries take this
pub struct Tx<'a>(DatabaseTransaction); // writes — commands take this

impl Db { pub fn pool(&self) -> &PgPool; }  // for sqlx query_as! macros [D-99]
impl Tx<'_> { pub async fn begin(db: &Db, ctx: &TenantContext) -> Result<Self, AppError>; }
```

`Tx::begin` is the **only** place `set_config` appears.

### `api` — depends on `core`, `db`

```rust
pub struct ApiOk<T>(T);          // impl IntoResponse — the envelope IS the return type
pub struct AuthUser(…);          // extractor; a handler without it cannot see a user
pub struct TenantTx(pub Tx);     // extractor; commits on Ok, rolls back on Err
pub struct Pagination { page: u32, page_size: u32 }
pub fn require(p: Permission) -> impl Layer<…>;   // per-route permission
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
