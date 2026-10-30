# PLAN: 03 — Identity and access

**Gate 2.** How [`SPEC.md`](SPEC.md) and [`CONTRACT.md`](CONTRACT.md) get built. Shapes that
leave the crate are the contract's; everything here is internal and may change without
touching another module.

---

## 1. Crate layout

```
crates/identity/src/
  lib.rs            # mod repo;  pub mod commands/queries/rest/types/auth/password/roles
  types.rs          # API types + newtypes. No hash field in any of them
  password.rs       # argon2 hashing and verification, for passwords and PINs
  tokens.rs         # JWT encode/decode, the two audiences, hashing of refresh tokens
  auth.rs           # authenticate() / authenticate_platform() — the middleware's body
  revocation.rs     # the Redis revocation list, and the branch-access cache
  rate_limit.rs     # login and reset counters in Redis [D-86] [D-95]
  roles.rs          # the derivation: permissions at a branch, branch access
  commands.rs       # WRITES — take &Tx and &TenantContext
  queries.rs        # READS  — take &Db
  platform.rs       # platform_users, TOTP, impersonation
  login.rs          # the one path that runs without a tenant
  entity/           # generated, private
  repo.rs           # SQL, private
  rest.rs           # handlers + utoipa attributes + OpenApiRouter
```

**Dependencies:** `core`, `db`, `api`, `cache`, `sea-orm`, `sqlx`, `uuid`, `chrono`,
`serde`, `thiserror`, `utoipa`, `utoipa-axum`, `tracing`, plus **`argon2`**, **`jsonwebtoken`**
and **`totp-rs`** — the three new ones, all named in the stack as `[Proposed]` or `[Decided]`.

`password.rs` and `tokens.rs` are separate from `auth.rs` on purpose: hashing and signing are
pure and testable without a database, and the middleware body is not.

---

## 2. Migration `m0003_identity`

One migration, written and **not run** (Art. V.1). Raw SQL for everything the schema builder
cannot say, which here is most of it.

| Order | What | Why raw SQL |
|---|---|---|
| 1 | `permissions`, `role_templates`, `role_template_permissions` | No `tenant_id`; RLS enabled with a platform-write policy, as `02-tenancy` does for plans |
| 2 | `platform_users` | No `tenant_id`, and **no grant at all to the app role** |
| 3 | `users`, with `UNIQUE (email)` | The global unique is the deliberate Art. IV.4 exception |
| 4 | `roles` + partial unique `WHERE is_owner` | Schema builder cannot express a partial index |
| 5 | `role_permissions` + the `is_tenant_assignable` check | A check against another table needs a trigger or a composite FK onto a filtered view |
| 6 | `user_branch_roles` + `UNIQUE … NULLS NOT DISTINCT` | Postgres 15+ syntax, no builder support |
| 7 | `devices`, `sessions`, `auth_tokens` | Composite FKs `(tenant_id, …)` |
| 8 | `impersonation_sessions` | Mixed ownership: a tenant read policy, a platform write policy |
| 9 | `FORCE ROW LEVEL SECURITY` + policy on every tenant-owned table | Art. IV.1 |
| 10 | The login lookup `SECURITY DEFINER` function | Cannot be expressed as a query at all |
| 11 | Grants, including the **absence** of one on `platform_users` | — |

### Seed, in the same migration

- Every permission slug every module declares, with `module`, `is_branch_scoped` and
  `is_tenant_assignable`. **Including `02-tenancy`'s**, which currently declares slugs no table
  holds — this migration is where they become rows.
- `role_templates` for `owner`, `manager`, `cashier` `[D-15]`, and their
  `role_template_permissions`.
- The owner template holds every `is_tenant_assignable` permission `[D-79]`.

**No seed user.** A pharmacy's first user comes from signup, which is `02-tenancy`'s **Q1** —
still unanswered. So this migration seeds catalogue data only, and the first account is
created by whatever answers that question.

---

## 3. Types

Newtypes in `core::ids` (add `RoleId`, `DeviceId`, `SessionId` beside the existing ones).
Enums are plain unit enums, `lower_snake` on the wire:

```rust
pub enum UserStatus { Active, Disabled }
pub enum DeviceStatus { Active, Revoked }
pub enum DevicePlatform { Android, Ios, Web }
pub enum PlatformRole { SuperAdmin, Reviewer, Support, Billing }   // [D-80], fixed in code
pub enum AuthTokenPurpose { PasswordReset, PhoneVerify }
pub enum TokenAudience { Pharmacy, Platform }                      // [AC-U20]
```

**`Secret<T>` for anything hashed.** `password_hash`, `pin_hash`, `refresh_token_hash` and
`totp_secret_encrypted` are wrapped in a newtype whose `Debug` prints `[redacted]` and which
has **no `Serialize`**. A hash cannot then reach a response, a log or a Sentry event by
accident — `[AC-U02]` and the "what the error tracker must never see" rule enforced by the
type system rather than by review.

---

## 4. Commands

Each takes `&Tx` and `&TenantContext`, returns plain data, and contains no HTTP.

| Function | Rule it carries |
|---|---|
| `create_user` | email lower-cased before insert; the global unique does the rest |
| `update_user` · `disable_user` | `disable_user` refuses the last active owner `[AC-W06]` |
| `set_password` · `set_pin` | argon2, via `password.rs`; clears `must_change_password` |
| `reset_password_for_user` | generates a temporary password, sets `must_change_password` `[D-76]` |
| `create_role` · `update_role` · `delete_role` | each refuses `is_system` first `[AC-W04]` |
| `set_role_permissions` | refuses any `is_tenant_assignable = false` slug `[AC-W05]`, then clears the cache for every user holding that role |
| `grant_role` · `revoke_grant` | composite FKs stop a cross-tenant grant; both clear that user's cached branch access `[AC-E08]` |
| `register_device` · `revoke_device` | `revoke_device` revokes the device's sessions in the same transaction `[AC-E04]` |
| `seed_roles_for_tenant` | called by `02-tenancy`'s pharmacy creation `[AC-E07]` |

Login, refresh and logout live in `login.rs` rather than `commands.rs`, because **login runs
outside a tenant transaction** `[AC-U03]` and so cannot take `&Tx` and `&TenantContext`. That
difference in signature is the honest signal that it is a different kind of operation.

---

## 5. Queries

| Function | Shape |
|---|---|
| `me` | user + derived permissions + derived branch list |
| `users` · `roles` · `devices` · `sessions` | cursor pages, `ListPage<T>` |
| `assignable_permissions` | `WHERE is_tenant_assignable` — the filter is in the query, not the handler `[AC-U11]` |
| `permissions_at(user, branch)` | union over `user_branch_roles` where `branch_id = $2 OR branch_id IS NULL` |
| `branch_access(user)` | null grant expands to every **active** branch `[AC-U13]`, so it joins `branches` and is re-derived rather than cached forever |

`permissions_at` and `branch_access` are the two hot reads — they run on every request — and
both are `sqlx::query_as!` so the compiler checks them against the live schema.

### Caching

Redis, through `cache`:

| Key | Holds | Cleared by |
|---|---|---|
| `t:{tenant}:u:{user}:branches` | branch access | `grant_role`, `revoke_grant`, `set_role_permissions`, `disable_user` |
| `t:{tenant}:u:{user}:perms:{branch}` | permission set | the same |
| `revoked:session:{id}` | a revoked session, TTL = the access token's remaining life | set by logout, device revoke, user disable |

**A miss or a Redis error denies** `[AC-U15]` — it never falls back to "all branches", which is
the same rule `02-tenancy` applies to the access mode and for the same reason.

---

## 6. Test plan

Against a real Postgres, per the testing rules. The ones that matter:

| Test | Proves |
|---|---|
| `login_answers_the_same_for_unknown_email_and_wrong_password` | `[AC-U01]` — compares status, code and body, not just status |
| `a_refresh_token_used_twice_revokes_the_session` | `[AC-W02]`, the theft rule |
| `a_refresh_token_is_bound_to_its_device` | `[AC-U06]` |
| `revoking_a_device_revokes_its_sessions` | `[AC-E04]`, one transaction |
| `a_revoked_session_is_refused_before_its_token_expires` | `[AC-U07]`, the Redis list |
| `the_last_active_owner_cannot_be_disabled` | `[AC-W06]` |
| `a_system_role_cannot_be_renamed_or_deleted` | `[AC-W04]` |
| `a_platform_only_permission_is_refused_on_direct_submission` | `[AC-W05]` — the editor hiding it is not the test |
| `a_platform_only_permission_is_absent_from_the_assignable_list` | `[AC-U11]` |
| `a_null_branch_grant_includes_a_branch_created_later` | `[AC-U13]`, the part a snapshot would get wrong |
| `branch_access_denies_when_redis_is_unreachable` | `[AC-U15]` |
| `a_role_cannot_be_granted_to_another_tenants_user` | `[AC-U24]`, against the database |
| `a_pharmacy_token_is_rejected_by_a_platform_route` | `[AC-U20]`, the audience split |
| `platform_users_is_unreadable_with_a_tenant_context` | `[AC-U18]` |
| `no_response_type_serializes_a_hash` | `Secret<T>` has no `Serialize`; a compile-fail test or a reflection over the types |
| `rls_blocks_cross_tenant_read` · `rls_blocks_cross_tenant_write` | Art. IV, per the template |

**The leakage suite gains every endpoint in this module**, run as pharmacy A against pharmacy
B's ids.

---

## 7. Risks

| Risk | Handling |
|---|---|
| **The permission check has no branch dimension.** `AuthUser.permissions` is flat; `is_branch_scoped` says it should not be | **Agreed and built** (Phase 11, 2026-10-25). `AuthUser` carries `permissions: Vec<String>` for pharmacy-wide slugs plus `branch_permissions: BTreeMap<BranchId, Vec<String>>`, and `require_at(slug, branch)` ORs the two. `identity::auth::authenticate` fills both from `queries::resolve_access`, **inside a tenant transaction** — on a bare connection forced RLS returns an empty set rather than an error, which is exactly how the authenticator shipped empty and unnoticed. The map is keyed by every reachable branch, so the tenant context's branch list is its keys |
| Login is the one read outside RLS | It is a `SECURITY DEFINER` function returning four columns, and it is the only such function. A test asserts it returns no other column |
| argon2 is deliberately slow, and login has a rate limit in front of it | Parameters tuned once and recorded; the rate limiter is checked **before** hashing, so a flood costs a Redis read, not a CPU-bound hash |
| A timing difference reveals whether an email exists | Verify against a dummy hash when the user is absent, so both paths do the same work `[AC-U01]` |
| Seeding every module's permission slugs now, before those modules exist | Deliberate: the slug catalogue is one table, and a module that cannot find its slug fails loudly at startup rather than silently authorising nothing |
| `02-tenancy` already declares permission slugs with no rows behind them | This migration's seed is where they become real. A test asserts every slug in every crate's `PERMISSIONS` exists as a row |
| TOTP secrets at rest | Encrypted with a key from config, never the database's own. Losing the key locks out platform staff, which is a documented operational risk, not a silent one |

---

## 8. Blocked — planned, not specified

These wait on [`SPEC.md`](SPEC.md) §5. Nothing else in the module waits on them.

| Question | What is not planned |
|---|---|
| **Q1** — refresh-token lifetime | `sessions.expires_at`'s default. The rotation and theft rules are planned and testable without it |
| **Q2** — PIN lockout threshold | The counter only, and possibly nothing server-side at all. PIN verification is planned |
| **Q3** — impersonation session duration | `expires_at`'s default. The read-only rule and the owner's visibility are planned |
| **Q4** — the per-IP cap | The per-IP half of `rate_limit.rs`. The per-email half is fully specified by `[D-95]` |

Also not planned here, by scope: the first user of a new pharmacy, which waits on
`02-tenancy`'s **Q1** (who creates a pharmacy).
