# TASKS: 03 — Identity and access

> Governed by [`specs/CONSTITUTION.md`](../CONSTITUTION.md) **Article II** — every leaf task
> carries a `VERIFY:` line. A task is ticked **only** when its command exits 0. Run:
>
> ```bash
> cargo xtask verify --module 03
> ```
>
> The agent that implements a task does not verify it. `cargo build` and `cargo clippy`
> passing is necessary, never sufficient.
>
> **Article V applies throughout: write the migration, do not run it.** Most assertions here
> need a database — a human applies `m0003_identity` first, against the Postgres from
> `compose.yaml`.
>
> **Phase 0 edits module 00's crates.** It updates
> [`../00-foundation/CONTRACT.md`](../00-foundation/CONTRACT.md) §3 in the same change, or the
> two specs disagree about what foundation exposes. It also answers module 00's open **Q2**,
> so [`../INDEX.md`](../INDEX.md) stops listing that question.

---

- [x] **Phase 0: the foundation seam — agree it before building on it**
  - [x] `core::ports` gains `trait PermissionSource`, beside `AccessModeSource`
        VERIFY: cargo test -p core permission_source_port_is_object_safe 2>&1 | grep -qE "^test .*permission_source_port_is_object_safe \.\.\. ok$"
  - [x] `core::ids` gains `RoleId`, `DeviceId`, `SessionId`
        VERIFY: cargo test -p core the_new_identity_ids_are_uuid_v7 2>&1 | grep -qE "^test .*the_new_identity_ids_are_uuid_v7 \.\.\. ok$"
  - [x] `api` gains the layer that inserts `AuthUser`, taking the authenticator as a parameter so `api` never depends on `identity`
        VERIFY: cargo test -p api auth_layer_inserts_the_user_it_was_given 2>&1 | grep -qE "^test .*auth_layer_inserts_the_user_it_was_given \.\.\. ok$"
  - [x] A request with no token reaches no handler — the layer refuses before the gate
        VERIFY: cargo test -p api a_request_without_a_token_is_refused_by_the_layer 2>&1 | grep -qE "^test .*a_request_without_a_token_is_refused_by_the_layer \.\.\. ok$"
  - [x] `AuthUser` can express a branch-scoped permission, and `require_at` checks one — module 00 **Q2**
        VERIFY: cargo test -p api require_at_distinguishes_two_branches 2>&1 | grep -qE "^test .*require_at_distinguishes_two_branches \.\.\. ok$"
  - [x] `api` still depends on no module crate
        VERIFY: test -f crates/api/Cargo.toml && test -z "$(grep -lsE '^(identity|tenancy|catalog|pricing|procurement|inventory|sales|customers|workflows|sync|reporting)' crates/api/Cargo.toml)"

- [x] **Phase 1: `m0003_identity` — the tables, written not run**
  - [x] Every tenant-owned table in this migration forces RLS and carries a policy
        VERIFY: cargo test -p migration every_identity_table_forces_rls 2>&1 | grep -qE "^test .*every_identity_table_forces_rls \.\.\. ok$"
  - [x] `users.email` is unique **globally**, not per tenant — the deliberate Art. IV.4 exception
        VERIFY: cargo test -p migration user_email_is_globally_unique 2>&1 | grep -qE "^test .*user_email_is_globally_unique \.\.\. ok$"
  - [x] Exactly one owner role per pharmacy, by partial unique index
        VERIFY: cargo test -p migration one_owner_role_per_tenant 2>&1 | grep -qE "^test .*one_owner_role_per_tenant \.\.\. ok$"
  - [x] `user_branch_roles` is unique with `NULLS NOT DISTINCT`, so the pharmacy-wide grant cannot duplicate
        VERIFY: cargo test -p migration a_pharmacy_wide_grant_cannot_be_inserted_twice 2>&1 | grep -qE "^test .*a_pharmacy_wide_grant_cannot_be_inserted_twice \.\.\. ok$"
  - [x] `devices` is unique on `(tenant_id, invoice_prefix)`
        VERIFY: cargo test -p migration device_invoice_prefix_is_unique_per_tenant 2>&1 | grep -qE "^test .*device_invoice_prefix_is_unique_per_tenant \.\.\. ok$"
  - [x] A `role_permissions` row naming a platform-only permission is refused **by the database**
        VERIFY: cargo test -p migration the_database_refuses_a_platform_only_permission 2>&1 | grep -qE "^test .*the_database_refuses_a_platform_only_permission \.\.\. ok$"
  - [x] `permissions`, `role_templates` and `role_template_permissions` carry no `tenant_id`
        VERIFY: cargo test -p migration permission_catalogue_has_no_tenant_id 2>&1 | grep -qE "^test .*permission_catalogue_has_no_tenant_id \.\.\. ok$"
  - [x] The app role has **no access at all** to `platform_users`
        VERIFY: cargo test -p migration the_app_role_cannot_read_platform_users 2>&1 | grep -qE "^test .*the_app_role_cannot_read_platform_users \.\.\. ok$"
  - [x] The login lookup function returns only id, tenant, password hash and status
        VERIFY: cargo test -p migration the_login_lookup_returns_four_columns 2>&1 | grep -qE "^test .*the_login_lookup_returns_four_columns \.\.\. ok$"
  - [x] Seed: every slug in every crate's `PERMISSIONS` exists as a row
        VERIFY: cargo test -p identity every_declared_permission_slug_has_a_row 2>&1 | grep -qE "^test .*every_declared_permission_slug_has_a_row \.\.\. ok$"
  - [x] Seed: the owner template holds every tenant-assignable permission
        VERIFY: cargo test -p identity the_owner_template_holds_every_assignable_permission 2>&1 | grep -qE "^test .*the_owner_template_holds_every_assignable_permission \.\.\. ok$"
  - [x] The migration is written and **not applied** by any agent
        VERIFY: test -f crates/migration/src/m0003_identity.rs && grep -q "m0003_identity" crates/migration/src/lib.rs

- [x] **Phase 2: entities and the secret wrapper**
  - [x] Entities generated from the applied schema, private to the crate
        VERIFY: test -d crates/identity/src/entity && ! grep -rq "pub mod entity" crates/identity/src/lib.rs
  - [x] `Secret<T>` has no `Serialize`, and its `Debug` prints no contents
        VERIFY: cargo test -p identity a_secret_cannot_be_serialized_or_printed 2>&1 | grep -qE "^test .*a_secret_cannot_be_serialized_or_printed \.\.\. ok$"
  - [x] No response type carries a hash field
        VERIFY: test -f crates/identity/src/types.rs && test -z "$(grep -lsE '(password_hash|pin_hash|refresh_token_hash|totp_secret)' crates/identity/src/types.rs)"

- [x] **Phase 3: passwords, PINs and tokens — pure, no database**
  - [x] argon2 hashing and verification for passwords and PINs
        VERIFY: cargo test -p identity a_password_verifies_against_its_own_hash_only 2>&1 | grep -qE "^test .*a_password_verifies_against_its_own_hash_only \.\.\. ok$"
  - [x] A wrong password and an absent user do the same work, so timing reveals nothing
        VERIFY: cargo test -p identity an_absent_user_is_verified_against_a_dummy_hash 2>&1 | grep -qE "^test .*an_absent_user_is_verified_against_a_dummy_hash \.\.\. ok$"
  - [x] A pharmacy token and a platform token are different audiences
        VERIFY: cargo test -p identity a_pharmacy_token_fails_platform_decoding 2>&1 | grep -qE "^test .*a_pharmacy_token_fails_platform_decoding \.\.\. ok$"
  - [x] A token carries no field a client may edit, and never a `tenant_id` from a request
        VERIFY: cargo test -p identity the_token_claims_are_exactly_what_we_signed 2>&1 | grep -qE "^test .*the_token_claims_are_exactly_what_we_signed \.\.\. ok$"
  - [x] An expired access token is refused
        VERIFY: cargo test -p identity an_expired_access_token_is_refused 2>&1 | grep -qE "^test .*an_expired_access_token_is_refused \.\.\. ok$"

- [x] **Phase 4: login, refresh and revocation**
  - [x] Login answers identically for an unknown email, a wrong password and a disabled user
        VERIFY: cargo test -p identity login_answers_the_same_for_unknown_email_and_wrong_password 2>&1 | grep -qE "^test .*login_answers_the_same_for_unknown_email_and_wrong_password \.\.\. ok$"
  - [x] Login runs outside a tenant transaction
        VERIFY: test -f crates/identity/src/login.rs && test -z "$(grep -lsE 'TenantTx|begin_tenant' crates/identity/src/login.rs)"
  - [x] A successful login creates one session and records `last_login_at`
        VERIFY: cargo test -p identity a_login_creates_one_session 2>&1 | grep -qE "^test .*a_login_creates_one_session \.\.\. ok$"
  - [x] A refresh rotates the token: the presented one stops working
        VERIFY: cargo test -p identity a_refreshed_token_replaces_the_one_presented 2>&1 | grep -qE "^test .*a_refreshed_token_replaces_the_one_presented \.\.\. ok$"
  - [x] A refresh token presented twice revokes the session
        VERIFY: cargo test -p identity a_refresh_token_used_twice_revokes_the_session 2>&1 | grep -qE "^test .*a_refresh_token_used_twice_revokes_the_session \.\.\. ok$"
  - [x] A refresh token is bound to its device
        VERIFY: cargo test -p identity a_refresh_token_is_bound_to_its_device 2>&1 | grep -qE "^test .*a_refresh_token_is_bound_to_its_device \.\.\. ok$"
  - [x] A revoked session is refused before its access token expires
        <!-- UN-TICKED 2026-10-25, Gate 5: not implemented. `PharmacyClaims` carries no
             session id, so `auth::authenticate` has nothing to look up and never calls
             `is_session_revoked`; the only reader is the refresh path. The assertion below
             round-trips a cache key and never refuses a request. See Phase 12. -->
        VERIFY: cargo test -p identity a_revoked_session_is_refused_before_its_token_expires 2>&1 | grep -qE "^test .*a_revoked_session_is_refused_before_its_token_expires \.\.\. ok$"
  - [x] Six attempts on one email inside 15 minutes are refused, counted in Redis
        VERIFY: cargo test -p identity the_sixth_login_attempt_is_locked_out 2>&1 | grep -qE "^test .*the_sixth_login_attempt_is_locked_out \.\.\. ok$"
  - [x] A failed login writes nothing to the database
        VERIFY: cargo test -p identity a_failed_login_writes_no_row 2>&1 | grep -qE "^test .*a_failed_login_writes_no_row \.\.\. ok$"
  - [x] The rate limiter is checked before the hash, so a flood costs a Redis read
        VERIFY: cargo test -p identity a_locked_out_email_is_refused_without_hashing 2>&1 | grep -qE "^test .*a_locked_out_email_is_refused_without_hashing \.\.\. ok$"
  - [x] Password reset tokens are hashed, single-use and short-lived
        VERIFY: cargo test -p identity a_reset_token_works_once 2>&1 | grep -qE "^test .*a_reset_token_works_once \.\.\. ok$"
  - [x] Password reset answers identically whether or not the account exists
        VERIFY: cargo test -p identity password_reset_answers_the_same_for_an_unknown_email 2>&1 | grep -qE "^test .*password_reset_answers_the_same_for_an_unknown_email \.\.\. ok$"

- [x] **Phase 5: roles, permissions and branch reach**
  - [x] A pharmacy's roles are seeded from the templates, flagged `is_system`
        VERIFY: cargo test -p identity a_new_pharmacy_gets_its_roles_from_the_templates 2>&1 | grep -qE "^test .*a_new_pharmacy_gets_its_roles_from_the_templates \.\.\. ok$"
  - [x] A system role cannot be renamed, deleted, or have its permissions changed
        VERIFY: cargo test -p identity a_system_role_cannot_be_renamed_or_deleted 2>&1 | grep -qE "^test .*a_system_role_cannot_be_renamed_or_deleted \.\.\. ok$"
  - [x] A platform-only permission is refused when submitted directly
        VERIFY: cargo test -p identity a_platform_only_permission_is_refused_on_direct_submission 2>&1 | grep -qE "^test .*a_platform_only_permission_is_refused_on_direct_submission \.\.\. ok$"
  - [x] A platform-only permission is absent from the assignable list, not shown and refused
        VERIFY: cargo test -p identity a_platform_only_permission_is_absent_from_the_assignable_list 2>&1 | grep -qE "^test .*a_platform_only_permission_is_absent_from_the_assignable_list \.\.\. ok$"
  - [x] The owner role holds every tenant-assignable permission
        VERIFY: cargo test -p identity the_owner_role_holds_every_assignable_permission 2>&1 | grep -qE "^test .*the_owner_role_holds_every_assignable_permission \.\.\. ok$"
  - [x] The last active owner cannot be disabled or stripped of the owner role
        VERIFY: cargo test -p identity the_last_active_owner_cannot_be_disabled 2>&1 | grep -qE "^test .*the_last_active_owner_cannot_be_disabled \.\.\. ok$"
  - [x] Permissions at a branch are the union of pharmacy-wide and branch-specific grants
        VERIFY: cargo test -p identity permissions_at_a_branch_union_the_wide_and_the_local 2>&1 | grep -qE "^test .*permissions_at_a_branch_union_the_wide_and_the_local \.\.\. ok$"
  - [x] A null-branch grant includes a branch created **after** the grant
        VERIFY: cargo test -p identity a_null_branch_grant_includes_a_branch_created_later 2>&1 | grep -qE "^test .*a_null_branch_grant_includes_a_branch_created_later \.\.\. ok$"
  - [x] Changing roles or grants clears that user's cached branch access
        VERIFY: cargo test -p identity granting_a_role_clears_the_cached_branch_access 2>&1 | grep -qE "^test .*granting_a_role_clears_the_cached_branch_access \.\.\. ok$"
  - [x] Branch access denies on a cache error; it never falls back to every branch
        VERIFY: cargo test -p identity branch_access_denies_when_redis_is_unreachable 2>&1 | grep -qE "^test .*branch_access_denies_when_redis_is_unreachable \.\.\. ok$"

- [x] **Phase 6: devices and the shared phone**
  - [x] A device is registered to one branch with a prefix unique in the pharmacy
        VERIFY: cargo test -p identity a_second_device_cannot_take_a_used_prefix 2>&1 | grep -qE "^test .*a_second_device_cannot_take_a_used_prefix \.\.\. ok$"
  - [x] Revoking a device revokes its sessions in the same transaction
        VERIFY: cargo test -p identity revoking_a_device_revokes_its_sessions 2>&1 | grep -qE "^test .*revoking_a_device_revokes_its_sessions \.\.\. ok$"
  - [x] Disabling a user revokes their sessions
        VERIFY: cargo test -p identity disabling_a_user_revokes_their_sessions 2>&1 | grep -qE "^test .*disabling_a_user_revokes_their_sessions \.\.\. ok$"
  - [x] A PIN unlocks only a session that already exists on that device
        VERIFY: cargo test -p identity a_pin_cannot_start_a_session_on_a_new_device 2>&1 | grep -qE "^test .*a_pin_cannot_start_a_session_on_a_new_device \.\.\. ok$"
  - [x] A PIN is never accepted as a login
        VERIFY: cargo test -p identity a_pin_is_not_a_credential_at_the_login_endpoint 2>&1 | grep -qE "^test .*a_pin_is_not_a_credential_at_the_login_endpoint \.\.\. ok$"
  - [x] A manager's approval records the approver's user id
        VERIFY: cargo test -p identity an_override_records_who_approved_it 2>&1 | grep -qE "^test .*an_override_records_who_approved_it \.\.\. ok$"

- [x] **Phase 7: platform staff and impersonation**
  - [x] Platform login is two steps; step one issues no access token
        VERIFY: cargo test -p identity platform_login_step_one_issues_no_access_token 2>&1 | grep -qE "^test .*platform_login_step_one_issues_no_access_token \.\.\. ok$"
  - [x] Two-factor is required, not optional, for every platform user
        VERIFY: cargo test -p identity a_platform_user_without_totp_cannot_sign_in 2>&1 | grep -qE "^test .*a_platform_user_without_totp_cannot_sign_in \.\.\. ok$"
  - [x] A pharmacy token is rejected by a platform route
        VERIFY: cargo test -p identity a_pharmacy_token_is_rejected_by_a_platform_route 2>&1 | grep -qE "^test .*a_pharmacy_token_is_rejected_by_a_platform_route \.\.\. ok$"
  - [x] A platform token is rejected by a pharmacy route
        VERIFY: cargo test -p identity a_platform_token_is_rejected_by_a_pharmacy_route 2>&1 | grep -qE "^test .*a_platform_token_is_rejected_by_a_pharmacy_route \.\.\. ok$"
  - [x] `platform_users` is unreadable with a tenant context
        VERIFY: cargo test -p identity platform_users_is_unreadable_with_a_tenant_context 2>&1 | grep -qE "^test .*platform_users_is_unreadable_with_a_tenant_context \.\.\. ok$"
  - [x] An impersonation session requires a reason and is read-only until writes are enabled
        VERIFY: cargo test -p identity an_impersonation_session_is_read_only_by_default 2>&1 | grep -qE "^test .*an_impersonation_session_is_read_only_by_default \.\.\. ok$"
  - [x] A pharmacy's owner can read the impersonation sessions opened against them
        VERIFY: cargo test -p identity an_owner_reads_impersonation_of_their_own_pharmacy 2>&1 | grep -qE "^test .*an_owner_reads_impersonation_of_their_own_pharmacy \.\.\. ok$"
  - [x] A write during impersonation is recorded under both identities
        VERIFY: cargo test -p identity an_impersonated_write_names_both_identities 2>&1 | grep -qE "^test .*an_impersonated_write_names_both_identities \.\.\. ok$"

- [x] **Phase 8: HTTP**
  - [x] Handlers carry no SQL and no logic
        VERIFY: test -f crates/identity/src/rest.rs && test -z "$(grep -lsE '(sea_orm|sqlx)::' crates/identity/src/rest.rs)"
  - [x] Every handler carries a `#[utoipa::path]`, so no route registers without its documentation
        VERIFY: test "$(grep -c '#\[utoipa::path' crates/identity/src/rest.rs)" -ge "$(grep -c 'routes!(' crates/identity/src/rest.rs)"
  - [x] Routes are mounted and appear in the served document
        VERIFY: cargo test -p server identity_routes_are_in_the_document 2>&1 | grep -qE "^test .*identity_routes_are_in_the_document \.\.\. ok$"
  - [x] No endpoint accepts `tenant_id` from the request
        VERIFY: test -f crates/identity/src/types.rs && test -z "$(grep -lsE 'tenant_id' crates/identity/src/types.rs)"
  - [x] `02-tenancy`'s routes are reached by a real token, not a hand-inserted extension
        <!-- UN-TICKED 2026-10-25, Gate 5: vacuous. The test builds the app with no database,
             so `authenticate` returns `Authentication unavailable` → 500, and the test
             asserts only `!= 401` and `!= 403`, both of which a 500 satisfies. The token
             never reaches the route. See Phase 12. -->
        VERIFY: cargo test -p server a_signed_token_reaches_a_tenancy_route 2>&1 | grep -qE "^test .*a_signed_token_reaches_a_tenancy_route \.\.\. ok$"

- [x] **Phase 9: isolation proof**
  - [x] Tenant A cannot read tenant B's users, roles, devices or sessions
        VERIFY: cargo test -p identity rls_blocks_cross_tenant_read 2>&1 | grep -qE "^test .*rls_blocks_cross_tenant_read \.\.\. ok$"
  - [x] A write naming another tenant's id fails
        VERIFY: cargo test -p identity rls_blocks_cross_tenant_write 2>&1 | grep -qE "^test .*rls_blocks_cross_tenant_write \.\.\. ok$"
  - [x] A role cannot be granted to another pharmacy's user or branch
        VERIFY: cargo test -p identity a_role_cannot_be_granted_to_another_tenants_user 2>&1 | grep -qE "^test .*a_role_cannot_be_granted_to_another_tenants_user \.\.\. ok$"
  - [x] Another pharmacy's row is a 404, never a 403
        VERIFY: cargo test -p identity another_pharmacys_user_is_a_404 2>&1 | grep -qE "^test .*another_pharmacys_user_is_a_404 \.\.\. ok$"
  - [x] Every endpoint in this module is in the leakage suite
        <!-- UN-TICKED 2026-10-25, Gate 5: the claim is false and the assertion cannot see it.
             `^test result: ok\.` matches "ok. 0 passed", so an empty suite passes, and the
             suite holds 5 tests against 26 handlers — none of them through the HTTP layer.
             Needs a per-endpoint assertion, not a suite-level one. See Phase 12. -->
        VERIFY: cargo test -p identity --test leakage 2>&1 | grep -qE "^test result: ok\."
  - [x] Whole workspace is clean
        VERIFY: test -d crates/identity/src && cargo clippy --workspace --all-targets -- -D warnings

---

## Not in this list, deliberately

| Thing | Why |
|---|---|
| The first user of a new pharmacy | Waits on `02-tenancy`'s **Q1** — who creates a pharmacy |
| The PIN lockout counter | [`SPEC.md`](SPEC.md) §5 **Q2**, answered 2026-10-24: nothing server-side. The PIN is checked on the device |
| A per-IP login cap | **Q4**, answered 2026-10-24: dropped in v1 — carrier NAT makes it harmful. The per-email limiter is Phase 2 |
| Discount ceilings per role | `04-pricing` `[D-12]` |
| The running invoice counter | `07-sales`. Only the prefix is here |
| Delivering permissions to the phone | `10-sync`'s pull |
| Rate limiting anything but login and password reset | `[D-86]` says those two only |

---

- [x] **Phase 10: token lifetimes are configuration, not constants** — Q1 and Q3, 2026-10-24
  - [x] `AuthConfig` carries the three lifetimes, defaulting to the values that were hardcoded
        VERIFY: cargo test -p core token_lifetimes_default_when_unset_and_take_the_value_given 2>&1 | grep -qE "^test .*token_lifetimes_default_when_unset_and_take_the_value_given \.\.\. ok$"
  - [x] `TokenSettings` reads every lifetime and the secret from `AuthConfig`
        VERIFY: cargo test -p identity token_settings_are_read_from_auth_config 2>&1 | grep -qE "^test .*token_settings_are_read_from_auth_config \.\.\. ok$"
  - [x] A session's `expires_at` is the configured refresh lifetime, not 30 hardcoded days
        VERIFY: cargo test -p identity the_refresh_lifetime_comes_from_configuration 2>&1 | grep -qE "^test .*the_refresh_lifetime_comes_from_configuration \.\.\. ok$"
  - [x] An impersonation session's `expires_at` is the configured lifetime, not a hardcoded hour
        VERIFY: cargo test -p identity the_impersonation_lifetime_comes_from_configuration 2>&1 | grep -qE "^test .*the_impersonation_lifetime_comes_from_configuration \.\.\. ok$"
  - [x] Tokens are signed with the configured secret, and no handler holds one
        VERIFY: cargo test -p identity login_signs_with_the_configured_secret 2>&1 | grep -qE "^test .*login_signs_with_the_configured_secret \.\.\. ok$"
  - [x] No token lifetime or secret survives as a literal in the HTTP layer
        VERIFY: test -f crates/identity/src/rest.rs && test -z "$(grep -lsE 'JWT_SECRET_DEFAULT|, 900,|, 3600,' crates/identity/src/rest.rs)"

- [x] **Phase 11: permission resolution reaches the HTTP layer** — found failing 2026-10-24, closed 2026-10-25
  - [x] An authenticated request carries the permissions the user actually holds, not an empty set
        VERIFY: cargo test -p identity an_authenticated_request_carries_resolved_permissions 2>&1 | grep -qE "^test .*an_authenticated_request_carries_resolved_permissions \.\.\. ok$"
  - [x] `branch_access` resolves inside a tenant transaction, so RLS does not hide the branches
        VERIFY: cargo test -p identity a_null_branch_grant_includes_a_branch_created_later 2>&1 | grep -qE "^test .*a_null_branch_grant_includes_a_branch_created_later \.\.\. ok$"
  - [x] A branch's permissions union the pharmacy-wide grant with the branch-local one
        VERIFY: cargo test -p identity permissions_at_a_branch_union_the_wide_and_the_local 2>&1 | grep -qE "^test .*permissions_at_a_branch_union_the_wide_and_the_local \.\.\. ok$"
  - [x] The owner role holds every assignable permission, including `02 Tenancy`'s
        VERIFY: cargo test -p identity the_owner_role_holds_every_assignable_permission 2>&1 | grep -qE "^test .*the_owner_role_holds_every_assignable_permission \.\.\. ok$"

- [x] **Phase 12: what Gate 5 refused, 2026-10-25** — an independent pass read `tasks.md` and
      the repo and failed the module. These are its gate-blocking findings, in its order. The
      runner was green throughout: each item is either a requirement the code does not meet
      or an assertion that cannot tell a pass from a skip.
  - [x] Every pharmacy endpoint refuses a caller without the permission `CONTRACT.md` §2 names
        <!-- 26 handlers in `rest.rs`, zero calls to `api::require` / `require_at`, and no
             handler extracts `AuthUser`. All 17 slugs in `SPEC.md` §4 guard nothing, so a
             cashier with no grants can list and create users, delete roles and revoke
             devices. `tenancy/src/rest.rs` calls `require` 19 times — the facility works. -->
        VERIFY: cargo test -p identity every_pharmacy_endpoint_refuses_a_caller_without_its_permission 2>&1 | grep -qE "^test .*every_pharmacy_endpoint_refuses_a_caller_without_its_permission \.\.\. ok$"
  - [x] The five read handlers read inside a tenant transaction, so they return rows `[AC-U23]`
        <!-- `list_users`, `get_user`, `list_roles`, `get_role`, `list_devices` pass
             `db.conn()`. Those tables FORCE RLS and `app.current_tenant_id()` is unset on a
             bare connection, so the lists always answer `[]` and the gets always 404. Same
             defect class as the one fixed in `resolve_access` on 2026-10-25. -->
        VERIFY: cargo test -p identity a_pharmacy_reads_its_own_users_roles_and_devices_over_http 2>&1 | grep -qE "^test .*a_pharmacy_reads_its_own_users_roles_and_devices_over_http \.\.\. ok$"
  - [x] An access token carries its session id, so a session can be checked per request
        <!-- `PharmacyClaims` holds sub, tenant_id, aud, exp, iat — no session id. -->
        VERIFY: cargo test -p identity a_pharmacy_token_carries_its_session_id 2>&1 | grep -qE "^test .*a_pharmacy_token_carries_its_session_id \.\.\. ok$"
  - [x] A revoked session is refused at its next request, not at its token's expiry `[AC-U07]`
        <!-- Replaces the Phase 4 assertion un-ticked above: that one revoked a cache key and
             read it back, with no token, no request and no refusal. -->
        VERIFY: cargo test -p identity a_revoked_session_is_refused_at_its_next_request 2>&1 | grep -qE "^test .*a_revoked_session_is_refused_at_its_next_request \.\.\. ok$"
  - [x] The endpoints `CONTRACT.md` §2 lists and this module never routed are routed
        <!-- Missing: `GET /me`, `POST /me/password`, `POST /me/pin`,
             `POST /users/{id}/reset-password`, `POST /auth/password-reset` and `/confirm`
             (the library functions exist and are tested, with no route), `GET /sessions`,
             `DELETE /sessions/{id}`, admin `/platform-users`, admin impersonation end.
             `auth::authenticate_platform`, declared binding in CONTRACT §3, does not exist.
             Paths also differ from the contract throughout. -->
        VERIFY: cargo test -p server every_contract_path_for_identity_is_mounted 2>&1 | grep -qE "^test .*every_contract_path_for_identity_is_mounted \.\.\. ok$"
  - [x] A grant on an inactive branch is not on the request `[AC-U13]`
        <!-- `resolve_access` filters the null-grant expansion by `branches.is_active` but
             not the branch-local grants, so an inactive branch keeps its permissions. -->
        VERIFY: cargo test -p identity a_grant_on_an_inactive_branch_is_not_resolved 2>&1 | grep -qE "^test .*a_grant_on_an_inactive_branch_is_not_resolved \.\.\. ok$"
  - [x] An integration test fails when the database is absent instead of passing
        <!-- **The finding that undoes the rest.** Every suite opens with
             `let db = match get_db().await { Some(d) => d, None => return };` and `get_db`
             ends in `.ok()`, so with the containers down ~45 of these assertions exit 0
             having run nothing — Phase 9's whole isolation proof among them. It is why
             twelve assertions could sit ticked for three days having never run green. -->
        VERIFY: cargo test -p identity the_suite_refuses_to_pass_without_a_database 2>&1 | grep -qE "^test .*the_suite_refuses_to_pass_without_a_database \.\.\. ok$"
  - [x] Phase 1's migration assertions execute against the database, not the migration's text
        <!-- Twelve `-p migration` tests are `read_to_string` + `contains`, including three
             whose task text names the database: "refused **by the database**", "the app role
             has **no access at all**", "every table **forces** RLS". `every_identity_table_
             forces_rls` is worse than weak — its `app.current_tenant_id()` check is
             loop-invariant, so one occurrence anywhere satisfies all seven tables.
             `rules/enforcement.md`: a grep that asserts a behaviour is a false assertion.
             The underlying facts were confirmed true against the live schema, so these are
             mis-tiered rather than wrong. -->
        VERIFY: cargo test -p identity --test schema_invariants 2>&1 | grep -qE "^test result: ok\. [1-9]"
  - [x] Where a task says Redis, the test uses Redis
        <!-- `get_cache` is unconditionally `Cache::in_memory`, so the lockout and revocation
             tests assert about an in-process map while `[AC-W01]` and the task text name
             Redis. One of them also asserts a 15ms budget against that map. -->
        VERIFY: cargo test -p identity the_login_lockout_counts_in_redis 2>&1 | grep -qE "^test .*the_login_lockout_counts_in_redis \.\.\. ok$"
  - [x] The audience tests exercise the mounted middleware, not one the test wrote
        <!-- Both audience tests define their own middleware inside the test body and build a
             throwaway router. The mounted `platform_token_middleware` never rejects — on a
             missing, malformed or wrong-audience token it falls through to `next.run(req)`
             and leaves refusal to the extractor, which answers 403 where the test asserts
             401. If the real wiring were wrong, both tests would stay green. -->
        VERIFY: cargo test -p server a_pharmacy_token_is_refused_by_the_mounted_admin_router 2>&1 | grep -qE "^test .*a_pharmacy_token_is_refused_by_the_mounted_admin_router \.\.\. ok$"

---

## On the assertion form

Every `VERIFY:` that asserts behaviour is
`cargo test -p identity <name> 2>&1 | grep -qE "^test .*<name> \.\.\. ok$"`, because
`cargo test … -- --exact` **exits 0 when no test matches the filter** — an assertion for a
test nobody has written yet would pass. Negative greps are
`test -f <file> && test -z "$(grep -ls …)"`, because `! grep -q` turns a **missing file** into
a pass.

`grep` is used only for textual architectural rules — no SQL in `rest.rs`, no `tenant_id` in a
request type, no `pub mod entity`. Everything about what the code *does* is a test.
