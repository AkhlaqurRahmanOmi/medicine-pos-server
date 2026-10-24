# SPEC: 00 — Foundation

**Status:** Draft
**Crates:** `crates/core` · `crates/db` · `crates/api` · `crates/cache` · `crates/infra`
**Bounded context:** none — this module is below the contexts, not one of them
**Owner:** Claude

---

## 1. Purpose

Everything every other module stands on: typed configuration, the tenant context, the error
type and its HTTP shape, the database pool and the transaction that carries tenant identity
into PostgreSQL, the Redis pool, and the vendor adapters.

The module exists so that no domain crate ever writes an HTTP status code, parses an
environment variable, opens a connection, or names a vendor SDK. It is **five crates and one
spec**, because they share one rule set and are meaningless apart.

**Nothing in this module knows what a pharmacy is.** It has no tables, no permissions of its
own, and no business rules. The moment something here mentions a sale, a batch or a supplier,
it belongs in a module crate instead.

**Not this module's job:**

| Thing | Belongs to |
|---|---|
| Who a user is, what roles they hold | `03-identity` |
| Which pharmacy a request belongs to, and whether it is suspended | `02-tenancy` |
| The permission list and its meanings | `03-identity` |
| Any table with `tenant_id` on it | the module that owns the table |
| Route registration and the composition root | `crates/server`, Gate-0 exempt |

Foundation defines the *shape* of a permission check and the *mechanism* that makes RLS work.
It does not define a single permission or a single policy.

---

## 2. Decisions this module implements

| Ref | Decision | Consequence here |
|---|---|---|
| `[D-97]` | Cross-module flows go through `workflows`; modules never call each other | `api` sits below every module so no module depends on `server` — otherwise the graph cycles and Cargo refuses to build |
| `[D-98]` | Commands take a transaction, queries take a connection | `db` exposes both `Tx` and `Db` as distinct types, so the signature carries the distinction |
| `[D-99]` | sqlx `query_as!` macros over SeaORM's pool | `db` exposes the underlying `PgPool`, and `.sqlx/` is committed |
| `[D-85]` | Error text is plain language with a next action | `AppError` carries two strings: one for the user, one for the log |
| `[D-86]` | Rate limiting on login | `cache` provides the counter; `03-identity` decides the limit |
| `[D-87]` | Health checks | `/health` is owned here |
| `[D-92]` | Sentry on backend and app | the error mapping is the single place a 5xx is reported |
| `[D-94]` | Login rate limit | as `[D-86]` |

---

## 3. Behaviour

### Configuration

- `[AC-U01]` The server SHALL parse every environment variable into a typed struct **once at
  boot**, and SHALL refuse to start if any required variable is missing or malformed.
- `[AC-W01]` IF a variable is invalid, THEN the process SHALL exit non-zero naming the
  variable and what was expected, and SHALL NOT start a listener.
- `[AC-U02]` Configuration SHALL NOT be readable as untyped key lookups anywhere after boot.
  A missing value is a compile error, never a runtime `None`.

### Tenant context and isolation

- `[AC-U03]` Every request that touches tenant data SHALL carry a `TenantContext` holding the
  tenant id, the acting user id, and the branch ids that user may see.
- `[AC-E01]` WHEN a transaction opens, the system SHALL set `app.tenant_id`, `app.user_id`
  and `app.branch_ids` **as bound parameters** with `set_config(..., true)`, before any
  statement runs.
- `[AC-U04]` A handler that does not take the tenant transaction extractor SHALL have no way
  to reach the database. This is enforced by types, not by review.
- `[AC-W02]` IF the tenant setting cannot be applied, THEN the transaction SHALL abort and the
  request SHALL fail — never proceed with the setting absent.
- `[AC-E02]` WHEN a handler returns `Ok`, the transaction SHALL commit. WHEN it returns `Err`,
  the transaction SHALL roll back. Neither SHALL require the handler to say so.

### Errors and the response envelope

- `[AC-U05]` Every successful response SHALL carry the same envelope, with the request id in
  its metadata.
- `[AC-U06]` Every error response SHALL carry the same error envelope, with a stable machine
  code and the request id.
- `[AC-U07]` A handler SHALL NOT be able to return a bare body. The envelope is the return
  type, not a layer that wraps one.
- `[AC-U08]` Mapping a domain error to a status code SHALL happen in exactly one place, and
  SHALL be exhaustive — a new error variant without a mapping is a compile error.
- `[AC-S01]` WHILE the environment is production, an internal error SHALL return a generic
  message and a request id; the detail goes to the log and to Sentry, never to the response.

### Request identity

- `[AC-E03]` WHEN a request arrives without `x-request-id`, the system SHALL generate one.
  WHEN one is present and well-formed, it SHALL be used as given.
- `[AC-U09]` The request id SHALL appear on every log line for that request, and in every
  response envelope.

### Health

- `[AC-E04]` WHEN `/health` is called, the system SHALL return 200 without touching the
  database.
- `[AC-E05]` WHEN `/health/ready` is called, the system SHALL check the database and Redis and
  return 503 if either is unreachable.

### Offline behaviour

**Server-only.** No part of this module runs on a phone. The mobile app's offline behaviour is
`10-sync`'s concern; foundation only guarantees that an upload carrying a client-generated id
reaches a transaction with the right tenant set.

---

## 4. Tenancy and permissions

| Question | Answer |
|---|---|
| Tenant-owned? | **No.** This module owns no tables. |
| Branch-scoped? | No. It carries branch ids; it does not interpret them. |
| Permissions required | None. `/health` is unauthenticated; everything else is a library. |
| Visible to a cashier? | Not applicable — no user-facing surface beyond errors. |

[`rules/tenancy.md`](../rules/tenancy.md) applies in full, and this module is where three of
its ten rules are actually *implemented* rather than merely obeyed:

- **Rule 3** — the pool connects as the non-owner application role. The owner role is used
  only by `crates/migration`, and the two are separate URLs in the config.
- **Rule 4** — `set_config` with a bound parameter lives here and nowhere else. No other crate
  may call it.
- **Rule 9** — the money type is defined here, so no module can accidentally use `f64`.

---

## 5. Open questions

None.

**Answered — Q1, 2026-10-21.** The envelope does *not* match the API Design Guide, and the
guide wins: no success wrapper, cursor pagination with no total count, and a single
`{error:{code, message, details, trace_id}}` shape with `lower_snake` codes. Recorded in
[`CONTRACT.md`](CONTRACT.md) §2 — the guide is the source, and this spec does not restate it.

**Answered — Q2, 2026-10-24.** The question is void: there is no `app.branch_ids` setting.
`Tx` sets `app.tenant_id`, `app.user_id` and `app.is_platform` only, and no policy reads a
branch list — branch scoping is an authorisation check in `api`, not a row-level one, carried
by `branch_permissions` on `AuthUser` and enforced by `require_at`. Neither form was chosen
because neither is needed; a future policy that does need branch ids raises a new question.
