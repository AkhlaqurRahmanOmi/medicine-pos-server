# CONTRACT: 03 — Identity and access

**Crate:** `crates/identity`
**Migration:** `m0003_identity`

> What this module exposes to the rest of the system. Shapes here are binding: another
> module reads this file rather than the code.

---

## 1. Tables owned

### Tenant-owned — `tenant_id`, `FORCE ROW LEVEL SECURITY`, tenant policy

| Table | Key | Tenant-scoped uniques | Notes |
|---|---|---|---|
| `users` | `id` | **none — `email` is unique globally** | The one deliberate exception to Art. IV.4 `[D-75]`. `email` lower-case, required; `phone` optional and not unique. Holds `password_hash`, `pin_hash`, `must_change_password`, `status`, `last_login_at` |
| `roles` | `(tenant_id, id)` | `(tenant_id, name)` | `is_system` locks a shipped role `[D-77]`; `is_owner` with a partial unique index gives exactly one owner role `[D-79]`; `template_code` records origin only |
| `role_permissions` | `(tenant_id, role_id, permission_code)` | the key | May only reference `permissions.is_tenant_assignable = true` `[D-78]` |
| `user_branch_roles` | `(tenant_id, id)` | `(user_id, role_id, branch_id)` **`NULLS NOT DISTINCT`** | `branch_id` null = every active branch `[D-14]`. Without `NULLS NOT DISTINCT` the same pharmacy-wide grant could be inserted twice |
| `devices` | `(tenant_id, id)` | `(tenant_id, invoice_prefix)` | Belongs to one branch `[D-30]`. `status`, `app_version`, `push_token`, `last_seen_at`, `revoked_at` |
| `sessions` | `(tenant_id, id)` | `refresh_token_hash` | Rotated on every refresh. `device_id` null for web. `expires_at`, `last_used_at`, `revoked_at`, `ip_address`, `user_agent` |
| `auth_tokens` | `(tenant_id, id)` | `token_hash` | `purpose` ∈ `password_reset` \| `phone_verify`. Hashed, single-use via `used_at`, short-lived `[D-76]` |
| `impersonation_sessions` | `id` | — | Carries `tenant_id` so an **owner reads their own**, but only platform staff write it. `reason` required, `writes_enabled_at` null = read-only throughout |

### Platform-owned — no `tenant_id`, no tenant policy

| Table | Key | Notes |
|---|---|---|
| `permissions` | `code` | `module` groups the role editor; `is_branch_scoped`; `is_tenant_assignable` is a **column, not a list in the editor** `[D-78]`. Every authenticated caller reads; only platform staff write |
| `role_templates` | `code` | `owner`, `manager`, `cashier` `[D-15]` |
| `role_template_permissions` | `(template_code, permission_code)` | — |
| `platform_users` | `id` | `email` unique, `role` enum `super_admin`\|`reviewer`\|`support`\|`billing` `[D-80]`, `totp_secret_encrypted` **required**, `status`, `last_login_at`. **Never readable in a tenant context** |

### Constraints the database enforces, not the code

| # | Constraint | Why it is not application logic |
|---|---|---|
| 1 | `UNIQUE (email)` on `users`, globally | Two pharmacies cannot claim one login, and login therefore needs no pharmacy hint |
| 2 | Partial unique index: one `is_owner` role per `tenant_id` | A second owner role is a lockout risk and a permission-model contradiction |
| 3 | `UNIQUE (user_id, role_id, branch_id) NULLS NOT DISTINCT` | Postgres treats nulls as distinct by default, so the pharmacy-wide grant would duplicate |
| 4 | `UNIQUE (tenant_id, invoice_prefix)` on `devices` | Two devices sharing a prefix collide on invoice numbers, which `07-sales` cannot recover from |
| 5 | Composite FKs `(tenant_id, …)` on every child | A role cannot be granted to another pharmacy's user or branch, whatever a query forgets |
| 6 | `role_permissions.permission_code` references a permission, with a trigger or check refusing `is_tenant_assignable = false` | `[D-78]` must hold against a direct insert, not only against the editor |
| 7 | `UNIQUE (refresh_token_hash)`, `UNIQUE (token_hash)` | Rotation and single-use are meaningless if a hash can repeat |

### Grants

`users`, `roles`, `role_permissions`, `user_branch_roles`, `devices`, `sessions`,
`auth_tokens` — the app role reads, inserts and updates. Nothing here is append-only.

`permissions`, `role_templates`, `role_template_permissions`, `platform_users` — the app role
**reads** the first three and has **no access at all** to `platform_users`; writes are the
platform policy's.

`impersonation_sessions` — the app role reads rows for its own tenant; inserts and updates
come through the platform policy.

The login lookup is a **`SECURITY DEFINER` function** returning `(id, tenant_id,
password_hash, status)` for one email `[D-75]`, and nothing else. It is the only path in the
system that reads a tenant-owned table without a tenant context, which is why it returns four
columns rather than a row.

---

## 2. HTTP surface

### Public — no token

| Method | Path | Request | Response | Errors |
|---|---|---|---|---|
| `POST` | `/api/v1/auth/login` | `{email, password, device?}` | `{access_token, refresh_token, expires_in, user}` | `401 invalid_credentials` — the same answer for a wrong password, an unknown email and a disabled user `[AC-U01]`; `429 too_many_requests` `[D-95]` |
| `POST` | `/api/v1/auth/refresh` | `{refresh_token}` | `{access_token, refresh_token, expires_in}` | `401 invalid_token`; **`401 session_revoked` when the token is presented twice**, and the session is revoked by that call `[AC-W02]` |
| `POST` | `/api/v1/auth/logout` | `{refresh_token}` | `204` | — |
| `POST` | `/api/v1/auth/password-reset` | `{email}` | `202` | `429`. Answers identically whether or not the account exists |
| `POST` | `/api/v1/auth/password-reset/confirm` | `{token, new_password}` | `204` | `400 invalid_token` — also what an expired or already-used token returns |

### Pharmacy-facing — `/api/v1`, pharmacy token

| Method | Path | Permission | Response |
|---|---|---|---|
| `GET` | `/me` | authenticated | The user, their permissions, and their branch list |
| `POST` | `/me/password` | authenticated | `204`. Clears `must_change_password` |
| `POST` | `/me/pin` | authenticated | `204`. Sets `pin_hash` |
| `GET` | `/users` | `user.view` | `ListPage<User>` |
| `POST` | `/users` | `user.create` | `201 User` |
| `PATCH` | `/users/{id}` | `user.update` | `200 User` |
| `POST` | `/users/{id}/disable` | `user.disable` | `200 User`, or `409 last_active_owner` |
| `POST` | `/users/{id}/reset-password` | `user.reset_password` | `200 {temporary_password}` `[D-76]` |
| `GET` | `/permissions` | `role.view` | The assignable catalogue — `is_tenant_assignable = false` rows are **absent** `[D-78]` |
| `GET` · `POST` | `/roles` | `role.view` · `role.create` | `ListPage<Role>` · `201 Role` |
| `PATCH` · `DELETE` | `/roles/{id}` | `role.update` · `role.delete` | `200 Role` · `204`, or `409 role_is_system` |
| `PUT` | `/roles/{id}/permissions` | `role.update` | `200 Role`, or `403 permission_not_assignable` |
| `POST` | `/users/{id}/roles` | `role.assign` | `201`. Body `{role_id, branch_id?}` — null branch is pharmacy-wide |
| `DELETE` | `/users/{id}/roles/{grant_id}` | `role.assign` | `204` |
| `GET` · `POST` | `/devices` | `device.view` · `device.register` | `ListPage<Device>` · `201 Device` |
| `POST` | `/devices/{id}/revoke` | `device.revoke` | `200 Device`. Revokes its sessions in the same transaction |
| `GET` | `/sessions` | `session.view` | `ListPage<Session>` — hashes never included |
| `DELETE` | `/sessions/{id}` | `session.revoke` | `204` |
| `GET` | `/impersonation-sessions` | `user.view` | `ListPage<ImpersonationSession>` — the owner's window into platform access |

### Platform-facing — `/api/v1/admin`, platform token only

| Method | Path | Response |
|---|---|---|
| `POST` | `/auth/login` | Step one of two. `{mfa_token}` — never an access token |
| `POST` | `/auth/login/totp` | `{access_token, refresh_token}`. Required, not optional `[AC-U19]` |
| `GET` · `POST` | `/platform-users` | The staff register, `super_admin` only `[D-80]` |
| `POST` | `/impersonation-sessions` | `201`. Requires `reason`; read-only unless writes are enabled |
| `POST` | `/impersonation-sessions/{id}/enable-writes` | `200`. The only way `writes_enabled_at` is set |
| `POST` | `/impersonation-sessions/{id}/end` | `200` |

Platform permissions are **absent from a pharmacy's permission list, not present and
refused** — they are platform-token-only, and this module is where that audience is decided.

---

## 3. Public Rust surface

```rust
// identity::types — API shapes. No hash field appears in any of them.
pub struct User { pub id: UserId, pub name: String, pub email: String,
                  pub phone: Option<String>, pub status: UserStatus,
                  pub must_change_password: bool, pub last_login_at: Option<DateTime<Utc>> }
pub struct Me { pub user: User, pub permissions: Vec<String>, pub branch_ids: Vec<BranchId> }
pub struct Role { pub id: RoleId, pub name: String, pub is_system: bool, pub is_owner: bool,
                  pub template_code: Option<String>, pub permissions: Vec<String> }
pub struct Permission { pub code: String, pub module: String, pub description: String,
                        pub is_branch_scoped: bool }
pub struct Device { pub id: DeviceId, pub branch_id: BranchId, pub name: String,
                    pub invoice_prefix: String, pub status: DeviceStatus,
                    pub app_version: Option<String>, pub last_seen_at: Option<DateTime<Utc>> }
pub struct Session { pub id: SessionId, pub user_id: UserId, pub device_id: Option<DeviceId>,
                     pub last_used_at: Option<DateTime<Utc>>, pub expires_at: DateTime<Utc> }

// identity::auth — what `crates/server` mounts as middleware.
pub async fn authenticate(/* … */) -> Result<AuthUser, AppError>;
pub async fn authenticate_platform(/* … */) -> Result<PlatformToken, AppError>;

// identity — the trait `00-foundation` holds the port for.
pub trait PermissionSource: Send + Sync {
    async fn permissions_at(&self, user: UserId, branch: Option<BranchId>)
        -> Result<Vec<String>, AppError>;
    async fn branch_access(&self, user: UserId) -> Result<Vec<BranchId>, AppError>;
}

pub const PERMISSIONS: &[&str];   // every slug this module declares
```

### What this module needs from `00-foundation`

Two additions, each of which updates
[`../00-foundation/CONTRACT.md`](../00-foundation/CONTRACT.md) §3 in the same change:

1. **An auth middleware seam.** `AuthUser` and `PlatformToken` are read from request
   extensions by `api::permission::require` and
   `api::middleware::require_platform_middleware`, and **nothing inserts them.** `api` gains
   the layer that calls `identity::auth::authenticate` and inserts the result — so that the
   crate holding the gate does not also hold the JWT, and `api` keeps no dependency on
   `identity` (the function is passed in, as `02-tenancy` passes `AccessModeSource`).
2. **`trait PermissionSource` in `core::ports`**, beside `AccessModeSource`, so `api` can
   resolve permissions without depending on this crate.

**One thing foundation must change rather than add.** `AuthUser.permissions` is a flat
`Vec<String>`, but `permissions.is_branch_scoped` means a permission can be held at one branch
and not another. A flat list cannot express that, so either `AuthUser` carries the branch
dimension or `require(slug)` learns which branch the request is about. This is module 00's
open **Q2** (`branch_ids`) arriving for real; it is an engineering question, so §7 of
[`plan.md`](plan.md) proposes an answer rather than recording it as a requirement gap.

---

## 4. Events and jobs

| Event | Who reacts |
|---|---|
| Roles or branch grants changed | this module clears the user's cached branch access, synchronously, in the same operation `[AC-E08]` |
| Device revoked · user disabled | sessions revoked in the same transaction, and the session ids added to the Redis revocation list |
| Pharmacy created (`02-tenancy`) | this module seeds that pharmacy's roles from `role_templates` `[AC-E07]` |
| Expired `auth_tokens` and `sessions` | `12-jobs` sweeps them. Nothing here deletes on read |

This module **emits no queue jobs.** Everything it does is synchronous, because an
authorisation decision that arrives later is not an authorisation decision.

---

## 5. Offline and sync

| Concern | Answer |
|---|---|
| Created offline? | **No.** No user, role, device or session is created from the phone's queue, so nothing here needs a client-generated id for idempotency |
| Read offline? | Yes — the phone holds each signed-in user's permission set and branch list, and switches between them by PIN `[D-63]` |
| Delivery | `10-sync`'s pull, not an endpoint here |
| A revoked device still selling | It continues until its next call to the API, and its queued sales are rejected if the session was revoked **before the sale's time** `[AC-U17]`. `10-sync` enforces that; this module only answers when the session died |
