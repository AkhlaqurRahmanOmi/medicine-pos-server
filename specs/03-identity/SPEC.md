# SPEC: 03 — Identity and access

**Status:** Draft
**Crate:** `crates/identity`
**Bounded context:** Context 3 — Identity and access
**Owner:** Claude

---

## 1. Purpose

Who a person is, what they may do, where they may do it, and from which device. This is the
module that makes `tenant_id` arrive **from a token** rather than from a request body — the
single rule the whole isolation design rests on.

Today nothing populates `AuthUser` or `PlatformToken` outside tests, so every route in
`02-tenancy` is gated on an extension that no middleware inserts. This module is what makes
the existing gates real, and until it lands the API has authorisation shaped like
authentication and no authentication at all.

Three things it owns that are easy to mistake for one thing:

| | What it answers | Where it lives |
|---|---|---|
| **Authentication** | is this person who they say they are | `users`, `sessions`, `auth_tokens` |
| **Authorisation** | may they do this | `permissions`, `roles`, `role_permissions` |
| **Reach** | at which branches, on which device | `user_branch_roles`, `devices` |

It also owns the platform's own staff — a separate table with required two-factor, so a bug in
pharmacy user management cannot grant platform access — and the impersonation sessions through
which platform staff read a pharmacy's data.

**Not this module's job:**

| Thing | Belongs to |
|---|---|
| Pharmacies, branches, subscriptions, the access mode | `02-tenancy` |
| `TenantContext`, `Tx`, `set_config`, `AuthUser`, the `require(slug)` layer | `00-foundation` — this module fills them, it does not define them |
| Discount ceilings per role | `04-pricing` `[D-12]` |
| The running invoice counter | `07-sales` — the *prefix* is on the device, the number is not |
| Deciding a sale is valid on upload | `10-sync` — this module only says whether the session and device were live at the time |
| Route registration and middleware wiring | `crates/server`, Gate-0 exempt |
| The audit log's rows | the module performing the action; this module writes only its own |

---

## 2. Decisions this module implements

Cite, never restate. The reasoning lives in Notion.

| Ref | Decision | Consequence here |
|---|---|---|
| `[D-75]` | Staff login is email and password | `users.email` is the login and is unique **across every pharmacy**, so login never asks which pharmacy. `users.phone` is contact data and is not unique |
| `[D-76]` | Password reset by email only; an owner can set a temporary password | `auth_tokens` carries `purpose = password_reset`; `users.must_change_password` is what the temporary path sets |
| `[D-77]` | Shipped roles are locked; a pharmacy creates its own | `roles.is_system` is the lock. A pharmacy adds a role rather than editing a shipped one |
| `[D-78]` | Platform-only permissions are marked on the permission and never shown | `permissions.is_tenant_assignable = false`. Hidden from the role editor, and refused if submitted directly |
| `[D-79]` | The owner role is locked, holds every permission, cannot be deleted | `roles.is_owner`, one per pharmacy by partial unique index, always granted with `branch_id` null |
| `[D-80]` | Platform staff roles are a fixed list in code | `platform_users.role` is an enum — `super_admin`, `reviewer`, `support`, `billing` — not a row a pharmacy or a migration adds to |
| `[D-15]` | Role templates are shipped and editable | `role_templates` seed a pharmacy's roles at signup; `roles.template_code` records only where each came from |
| `[D-14]` | Role scope is per branch | `user_branch_roles.branch_id`, where **null means every active branch, including future ones** |
| `[D-30]` | Several devices per branch | `devices` is a real row, with its own `invoice_prefix` unique per pharmacy |
| `[D-63]` | Shared counter phone: each person their own account, PIN to switch user | `users.pin_hash`. A PIN unlocks a session that already exists on that device and is never a login |
| `[D-100]` | A manager approves on the cashier's phone with their PIN, and the approver is recorded | This module verifies the PIN and names the approver; the module performing the action records it |
| `[D-86]` | Rate limiting in v1 covers login and password reset only | Nothing else in this module, or any other, is rate limited |
| `[D-95]` | 5 failures per email per 15 minutes, then a 15-minute lockout. **The per-IP cap is dropped in v1** (Q4, 2026-10-24) — carrier NAT makes it harmful; the Notion decision needs amending | Counted in Redis, not in Postgres — a failed login must not write to the database |

---

## 3. Behaviour

EARS acceptance criteria, each one a future test name.

### Signing in

- `[AC-E01]` WHEN a correct email and password are presented, the system SHALL issue an access
  token and a refresh token, and SHALL create one `sessions` row.
- `[AC-U01]` The system SHALL answer login identically whether or not the account exists — same
  status, same code, same shape — so that login cannot be used to discover which emails exist.
- `[AC-U02]` The system SHALL store passwords, PINs, refresh tokens and reset tokens **only as
  hashes**, with argon2 for passwords and PINs.
- `[AC-S01]` WHILE a user's status is `disabled`, the system SHALL refuse login, and SHALL do
  so with the same answer as a wrong password.
- `[AC-W01]` IF a sixth login attempt is made for one email inside 15 minutes, THEN the system
  SHALL refuse it for 15 minutes `[D-95]`, AND the counter SHALL live in Redis so that a
  failed login writes nothing to the database.
- `[AC-U03]` Login SHALL run before a tenant is known, so it SHALL NOT run inside a tenant
  transaction. It uses one narrow lookup **by email** `[D-75]` returning only id, tenant, password hash
  and status.
- `[AC-E02]` WHEN login succeeds, the system SHALL record `last_login_at`.

### Tokens and sessions

- `[AC-U04]` An access token SHALL carry the tenant, the user, and nothing a client may edit;
  `tenant_id` SHALL NOT be accepted from any request, on any endpoint.
- `[AC-U05]` An access token SHALL live about 15 minutes (**Q1**).
- `[AC-E03]` WHEN a refresh token is used, the system SHALL rotate it — the presented token
  stops working and a new one is issued.
- `[AC-W02]` IF a refresh token is presented a second time, THEN the system SHALL treat it as
  theft: the session is revoked and the device must sign in again with a password.
- `[AC-U06]` A refresh token SHALL be bound to its device, and SHALL NOT be usable from
  another.
- `[AC-U07]` Revocation SHALL NOT wait for expiry: a revocation list in Redis SHALL be checked
  on every request, so a revoked session stops at its next call rather than at its token's
  expiry.
- `[AC-E04]` WHEN a device is revoked, the system SHALL revoke its sessions in the same
  transaction.
- `[AC-E05]` WHEN a user is disabled, the system SHALL revoke their sessions.

### Switching users on a shared phone

- `[AC-U08]` A PIN SHALL only unlock a session that already exists on that device. It SHALL
  NOT be accepted as a login, and SHALL NOT be accepted on a device with no session.
- `[AC-U09]` A PIN SHALL never be sent to the server as a credential `[D-63]`.
- `[AC-W03]` IF a PIN is entered wrongly repeatedly, THEN the device SHALL fall back to a full
  password (**Q2**).
- `[AC-E06]` WHEN a manager approves a blocked action with their PIN on someone else's phone
  `[D-100]`, the system SHALL record the approver's user id on the action.

### Roles and permissions

- `[AC-E07]` WHEN a pharmacy is created, the system SHALL create its roles from
  `role_templates`, flagged `is_system`.
- `[AC-U10]` A pharmacy SHALL hold exactly one owner role at all times, and that role SHALL
  hold every tenant-assignable permission.
- `[AC-W04]` IF an attempt is made to rename, delete, or change the permissions of an
  `is_system` role, THEN the system SHALL refuse it with `409 role_is_system` `[D-77]`.
- `[AC-W05]` IF a role is granted a permission whose `is_tenant_assignable` is false, THEN the
  system SHALL refuse it with `403 permission_not_assignable` — refused on submission as well
  as hidden from the editor `[D-78]`.
- `[AC-U11]` A pharmacy's role editor SHALL be offered only permissions with
  `is_tenant_assignable` true; platform-only permissions SHALL be **absent, not present and
  refused**.
- `[AC-W06]` IF the last active owner would be disabled or stripped of the owner role, THEN
  the system SHALL refuse it, so a pharmacy cannot lock itself out.
- `[AC-U12]` A user's permissions at a branch SHALL be **derived** — the union of the
  permissions of the roles granted at that branch or pharmacy-wide. No column stores them.

### Branch access

- `[AC-U13]` A user's branch access SHALL be derived from `user_branch_roles`, where a null
  `branch_id` means every **active** branch, including branches created later `[D-14]`.
- `[AC-U14]` Branch access SHALL be looked up per request and cached in Redis.
- `[AC-E08]` WHEN a user's roles or branch grants change, the system SHALL clear that user's
  cached branch access in the same operation.
- `[AC-U15]` The cache SHALL NOT fall back to "all branches" on a miss or an error; a lookup
  that cannot be answered denies.

### Devices

- `[AC-E09]` WHEN a device is registered, the system SHALL assign it an `invoice_prefix` unique
  within the pharmacy, because invoice numbers are generated per device.
- `[AC-U16]` A device SHALL belong to exactly one branch `[D-30]`.
- `[AC-E10]` WHEN a device syncs, the system SHALL record `last_seen_at`.
- `[AC-U17]` A revoked device SHALL be stopped at its next call to the API, **not instantly** —
  an accepted consequence of offline selling.

### Platform staff

- `[AC-U18]` Platform staff SHALL live in `platform_users`, which SHALL NOT be readable with a
  tenant context at all.
- `[AC-U19]` Two-factor login SHALL be **required** for every platform user.
- `[AC-U20]` A platform token SHALL be a different audience from a pharmacy token, and
  pharmacy routes SHALL reject it rather than honour it.
- `[AC-E11]` WHEN platform staff open an impersonation session, the system SHALL require a
  stated reason, SHALL time-limit it (**Q3**), and SHALL make it readable by that pharmacy's
  owner.
- `[AC-U21]` An impersonation session SHALL be **read-only** unless writes were explicitly
  enabled, recorded as `writes_enabled_at`.
- `[AC-U22]` A change made during impersonation SHALL be written under **both** identities.

### Tenant isolation

- `[AC-U23]` A user SHALL read their own pharmacy's users, roles, devices and sessions, and
  nothing else. Another pharmacy's row SHALL be `404 not_found`, never `403`.
- `[AC-U24]` A role SHALL NOT be grantable to another pharmacy's user or branch, enforced by
  composite foreign keys and not by application logic.

**Offline behaviour.** The phone holds its session and its user's permissions, and switches
users offline by PIN. Nothing in this module is *created* offline — no user, role or device is
registered from a queue — so no row here needs a client-generated id for idempotency. What the
phone needs delivered is the permission set and branch list for the users on it, which is
`10-sync`'s pull.

---

## 4. Tenancy and permissions

| Question | Answer |
|---|---|
| Tenant-owned? | **Mixed.** `users`, `roles`, `role_permissions`, `user_branch_roles`, `devices`, `sessions`, `auth_tokens` are tenant-owned. `permissions`, `role_templates` and `role_template_permissions` are platform-owned with no `tenant_id`. `platform_users` has none and is never readable in a tenant context. `impersonation_sessions` carries `tenant_id` but is written only by platform staff |
| Branch-scoped? | Partly. `devices` belong to a branch, and `user_branch_roles` is the table that *defines* branch reach. The rest is pharmacy-wide |
| Visible to a cashier? | A cashier reads their own profile and their own permissions. The user list, role editor, device register and session list are permissioned. `password_hash`, `pin_hash`, `refresh_token_hash` and `totp_secret_encrypted` are **never** in any response |

Permissions this module declares, as `&'static str` on its own crate:

| Slug | Guards |
|---|---|
| `user.view` · `user.create` · `user.update` | staff accounts |
| `user.disable` | disabling an account, subject to `[AC-W06]` |
| `user.reset_password` | setting a temporary password `[D-76]` |
| `role.view` · `role.create` · `role.update` · `role.delete` | the role editor, subject to `[AC-W04]` |
| `role.assign` | granting a role to a user at a branch |
| `device.view` · `device.register` · `device.revoke` | the device register |
| `session.view` · `session.revoke` | signing another device out |

Article IV applies in full. Specific to this module:

- **`users.email` is unique globally, not per tenant** — the one deliberate exception to the
  tenant-scoped uniqueness rule, and the reason login never asks which pharmacy `[D-75]`.
- **Login cannot pass row-level security**, because there is no tenant yet. It is the only
  read in the system that runs outside a tenant transaction, and it returns four fields.
- `permissions` and `role_templates` join `02-tenancy`'s plan tables as the only tables with
  no `tenant_id`.

---

## 5. Open questions

None.

**Answered — Q1 and Q3, 2026-10-24.** Both lifetimes are **configuration, not constants.**
`sessions.expires_at` is `now() + auth.refresh_token_ttl_secs`, and an impersonation session's
`expires_at` is `now() + auth.impersonation_ttl_secs`, each read from `AuthConfig` and
settable per deployment by `REFRESH_TOKEN_TTL_SECS` and `IMPERSONATION_TTL_SECS`. The shipped
defaults are 30 days and 1 hour — the values the code had hardcoded — so no behaviour changes
by default, but neither number is a literal in a handler any more. The access-token TTL joins
them as `ACCESS_TOKEN_TTL_SECS`, default 900, for the same reason: it was a literal `900` in
two handlers.

**Answered — Q4, 2026-10-24.** There is **no per-IP cap in v1.** The per-email limiter stands
as `[D-95]` specifies it — 5 failures per email per 15 minutes, counted in Redis. The per-IP
half is dropped: Bangladesh pharmacies reach the API through mobile-carrier NAT, where one
address fronts thousands of unrelated shops, so a per-IP cap would lock out a carrier to slow
an attacker who can rotate addresses anyway. **`[D-95]` in Notion still describes a per-IP cap
and needs amending to match** — flagged, not edited.

**Ask; do not invent** still binds: each of these four was asked and answered on 2026-10-24,
and the two values the code had already invented were put to the requirement owner rather than
ratified silently.
