# BACKLOG: 01 — Catalogue

> Governed by [`../rules/lifecycle.md`](../rules/lifecycle.md) §Optional tasks. **Nothing is
> failed by this file** — the module's count in [`../INDEX.md`](../INDEX.md) is what
> `tasks.md` asserts, and the runner never reads this file.
>
> Source: found on 2026-10-25 while module 05's Gate 1 pass read this module's contract for
> house style. **Not a review of module 01** — one defect, found incidentally, verified against
> the live database. Completing it means cutting the item into `tasks.md` under a new phase,
> ticking it there once its command exits 0, and raising the `INDEX.md` count to match.

---

## A1 · This module's five permission slugs are seeded by no migration

- [x] Every slug in `catalog::perms` exists as a `permissions` row and can be granted
      VERIFY: cargo test -p catalog every_catalog_slug_is_a_grantable_permission 2>&1 | grep -qE "^test .*every_catalog_slug_is_a_grantable_permission \.\.\. ok$"

**What is wrong.** `crates/catalog/src/perms.rs` declares five slugs — `catalog.view`,
`catalog.submit`, `catalog.issue.report`, `catalog.review`, `catalog.lookup.manage` — and
`crates/catalog/src/rest.rs` enforces four of them with `api::permission::require`. None of the
five exists in the `permissions` table: `m0005_catalog.rs` contains no `INSERT INTO permissions`,
and `m0003_identity.rs` is the only migration that seeds any.

**Why it is not cosmetic.** `role_permissions` carries an FK to `permissions(code)` and a
`check_role_permission_assignable()` trigger, so the slug cannot be granted to a role at all.
Verified against the dev database on 2026-10-25:

```
INSERT INTO role_permissions (tenant_id, role_id, permission_code, created_at)
VALUES (…, …, 'catalog.view', now());
→ ERROR: Permission catalog.view is not tenant assignable
   CONTEXT: PL/pgSQL function check_role_permission_assignable() line 6 at RAISE
```

The owner role is seeded from `permissions WHERE is_tenant_assignable = true`, so it does not
hold them either. **Every permission-gated catalogue endpoint is therefore unreachable for every
user in every pharmacy, the owner included** — `GET /catalog/products`, `POST /catalog/products`,
`POST /catalog/issues` and the admin review surface.

**Why no assertion caught it.** `identity`'s `every_declared_permission_slug_has_a_row` reads
`m0003_identity.rs` as *text* and checks only `identity`'s own `PERMISSIONS` const plus a
hand-copied list of ten tenancy slugs. `identity` is the only crate with a `PERMISSIONS` const,
so a module that declares slugs in its own `perms.rs` is invisible to it. The fix is worth more
than this one module: an assertion that walks every crate's declared slugs against the seeded
rows, run against the database rather than the migration's source.

**Written 2026-10-25, not applied.** `crates/migration/src/m0006_catalog_permissions.rs` is a
**new** migration rather than an edit to `m0005_catalog`, because Article V.4 makes an applied
migration immutable. It seeds the five rows — `catalog.review` and `catalog.lookup.manage` with
`is_tenant_assignable = false`, since `[D-78]` makes them platform-only — adds the three
assignable slugs to the owner template `[D-79]`, and **backfills the owner roles of pharmacies
that already exist**, which the template alone would not reach. `catalog.view` also goes to the
manager and cashier templates and `catalog.submit` to manager, which is not a judgement call:
[`SPEC.md`](SPEC.md) §4 says "Everything readable here is readable by a cashier. Submission is
not: Product Spec §3 limits it to owner and manager."

The assertion above is `crates/catalog/tests/permissions_seeded.rs`. It currently **fails** with
*"permission 'catalog.view' is declared in catalog::perms but has no row in `permissions`"* —
which is the defect, stated as a test. Its later stages, which prove the grant is actually
permitted and that the owner template carries the three, have not executed yet because the
first stage fails before them.

**An agent does not run migrations (Article V.1).** To apply it:

```bash
cargo run -p migration -- up
```

**Applied by a human 2026-10-25, and the assertion now passes** —
`every_catalog_slug_is_a_grantable_permission ... ok`, all three stages including the two that
had never executed. Verified in the database: five rows under module `catalog`, three
assignable and the platform-only pair at `is_tenant_assignable = false`, and 74 tenants'
owner/manager/cashier roles carrying catalogue grants from the backfill.

**Promoted and ticked 2026-10-25.** Both reasons this was left open have been met:

1. **Article II.2** — a separate agent, given only the module number and no session context,
   ran the `VERIFY:` command, checked the claim against the live database independently of the
   test, ran the dead-port counterfactual to prove the assertion does not pass vacuously, and
   inspected `m0006` against Article V. Verdict `PHASE 8: PASS`.
2. **Ownership** — cut into [`tasks.md`](tasks.md) as **Phase 8** and ticked, with the module's
   count in [`../INDEX.md`](../INDEX.md) raised 65 → 66 to match.

The verification also found three things this fix does *not* cover. They are **new items below,
uncounted** — not reasons to hold A1 open.

---

## A2 · The assertion's slug list is hand-copied, not derived

- [ ] A slug declared in `catalog::perms` cannot exist without appearing in the assertion
      VERIFY: *no command yet — see below*

`crates/catalog/tests/permissions_seeded.rs` carries `const SLUGS: &[(&str, bool)]` of five
string literals and **never links the `catalog` crate**: its imports are `sqlx`, `Duration` and
`uuid`, and `catalog::perms` appears only in a doc comment and a panic message. The copy is
accurate today — `perms.rs` declares exactly five `pub const`s and a grep for `catalog\.[a-z.]+`
across the crate returns those five and nothing else — so there is no live defect. But a sixth
slug added to `perms.rs` with no `permissions` row would leave the test green, which is A1's own
bug one level up.

**The fix is a single source both sides consume** — `pub const ALL: &[(&str, bool)]` in
`perms.rs`, read by the test and by `rest.rs` — so the list cannot drift. Writing that is what
gives this item a real `VERIFY:` line; until then it has none and is not a candidate for
`tasks.md`. Distinct from the cross-crate walker in [`tasks.md`](tasks.md) *Not in this list*,
which is 00's or 03's.

---

## A3 · Three of `m0006`'s six statements are asserted by nothing

- [ ] The backfill and the manager/cashier grants are proven, not just believed
      VERIFY: *no command yet — the test needs extending*

`permissions_seeded.rs` step 3 reads `role_template_permissions` for `template_code = 'owner'`
and stops there. So:

| Statement in `m0006` | Asserted? |
|---|---|
| The five `permissions` rows, with `[D-78]` assignability | yes, step 1 |
| The three on the owner **template** `[D-79]` | yes, step 3 |
| The owner-role **backfill** for existing pharmacies | **no** |
| `catalog.view` → manager and cashier templates | **no** |
| `catalog.submit` → manager template | **no** |
| The manager/cashier **role** backfill | **no** |

All four are true in the database — the verifier counted 74 tenants per role/slug pair — but the
command does not prove them, and the backfill is *precisely* the half A1 argued the template
alone would not reach. A regression there would be invisible.

Also in scope: step 2's loop `break`s at the first non-assignable slug, so only `catalog.review`
is ever proven refused by the trigger; `catalog.lookup.manage`'s refusal rides on step 1's
`is_tenant_assignable` assert instead. A savepoint per slug rather than the `break` closes it.

---

## A4 · `catalog.lookup.manage` is declared, seeded, and enforced nowhere

- [ ] Either a route requires it, or it is removed from `perms.rs` and `m0006`
      VERIFY: *needs a decision first, not a command*

A workspace-wide grep finds no `require(perms::LOOKUP_MANAGE)`. The admin lookup writes it was
presumably meant for are gated by platform token alone (Phase 5's
`a_pharmacy_cannot_edit_a_lookup_table`). `[D-78]` makes it platform-only, so a slug may be the
wrong mechanism for it entirely — a platform token is already the check. [`tasks.md`](tasks.md)
Phase 8 is honest about enforcing "four of them", so nothing here is overstated; the slug is
simply dead weight until someone decides which way it goes.

---

## A5 · Who may file an issue report is undeclared judgement

- [ ] `SPEC.md` §4 states the distribution of `catalog.issue.report`, or Notion is asked
      VERIFY: *a requirement question (Art. III), not a command*

`m0006` gives `catalog.issue.report` to the **owner template only**, and the migration's comment
cites [`SPEC.md`](SPEC.md) §4 as settling the distribution. §4 settles `catalog.view` and
`catalog.submit`; it says nothing about issue reports. So a cashier who spots a wrong MRP cannot
file one, which is a plausible answer but an undeclared one. Low confidence it is wrong — worth a
line in §4 either way, and a Notion check if Product Spec §3 covers it.

---

## A6 · The platform cannot insert into companies, generics or products

- [ ] A super admin can add a catalogue record directly, or the contract says it cannot
      VERIFY: *needs a decision first — a new migration either way*

Found 2026-10-25 while writing the import. All three submittable tables have exactly one INSERT
policy and it reads:

```sql
CREATE POLICY products_insert ON products FOR INSERT WITH CHECK (
    status = 'pending' AND submitted_by_tenant_id = app.current_tenant_id()
);
```

There is **no `app.is_platform` branch**, unlike every shape-B table. So the only way a row enters
the central catalogue is: a tenant inserts it `pending`, then the platform *updates* it to
`approved` — which the `*_update` policies do allow. Two consequences:

1. **The import cannot run as `pos_app` at all.** An imported row is `approved` with no submitting
   tenant, so it runs as `pos_owner`, bypassing RLS the way the migrator does. That is defensible
   for a human-run admin tool and is documented in `seed.rs`, but it was not a choice — the policy
   set left no alternative.
2. **A super admin cannot add a product.** They can approve, reject and merge what pharmacies
   submit, and nothing more. Whether that is the intended contract is a **requirement question**:
   [`CONTRACT.md`](CONTRACT.md) §2 has no admin-create route either, so the code and the contract
   agree — but nothing records that it is deliberate, and `[D-78]`'s platform-only
   `catalog.lookup.manage` slug hints the opposite was expected.

Either answer needs a new migration (Article V.4), so this is not a quick fix.

---

## A7 · `product_packs` is seeded by nothing, and the page overstates what the data supports

- [ ] Central pack defaults are imported, or §4's `[Proposed]` is withdrawn
      VERIFY: *needs the decision in A7 below, then a parser*

Product Spec §4 says central defaults exist *"where the source data knows the box size (about 80%
of rows say things like `30's pack`)"* — marked `[Proposed]`. Measured against the real file,
`(N's pack: …)` matches **15,162 of 30,670 rows, 49%**, and `pack_info` carries 40-plus formats
(`100 ml bottle: ৳ 80`, `1 gm vial: ৳ 190`, `10 x 10: ৳ 300`, `Price Unavailable`, several packs in
one cell). Meanwhile *"Tenants define pack sizes"* is `[Decided]`, and `[D-34]` gives
`product_packs` no price.

So the import leaves the table empty. Writing a half-parsed guess into a table every pharmacy
reads is worse than an empty one, and `10 x 10: ৳ 300` has no unambiguous reading as
`(pack_unit_id, base_units)` — is that a box of 100, or ten strips of ten? **The 49% figure should
reach Notion either way**, since 80% is what the proposal was justified on.

---

## A8 · `ingredients` and `generic_ingredients` have no source

- [ ] Both tables are populated from a real source, or dropped from the model
      VERIFY: *needs a data source that does not exist yet*

`m0005_catalog` creates `ingredients` and `generic_ingredients` with a `position` column, so the
model expects a generic to decompose into ordered active ingredients. **The scrape has no
ingredient column.** The only candidate is `generic_name`, and splitting it on `+` — which is how
the 5,452 combination drugs are written — yields 2,246 "ingredients" including
*"23 herb formula to increase breast milk"*, *"20% Fat Emulsion"*, *"15 vitamins and minerals"* and
*"10 Vitamin & 6 Mineral [Pregnancy and Breast Feeding Formula]"*.

That is not a parse failure to tidy up; multivitamins and herbal formulations genuinely do not
decompose this way. Both tables stay empty after the import, which means nothing reads them and
nothing breaks — but two tables in a `[DONE]` module hold no rows and have no path to any.

---

## Not this module's, recorded so it is not lost

**`m0006`'s `down` reports success having reversed nothing** — `Ok(())` and no SQL. For a
seed-only migration an honest `down` is a `DELETE FROM permissions WHERE code IN (…)`, since both
dependent FKs cascade. But `m0003_identity` and `m0004_tenancy_backlog` have the identical no-op
while `m0001`, `m0002` and `m0005` have real ones, and **nothing in [`../rules/`](../rules/)
requires a `down` at all**. That makes it a rules gap across the migration crate rather than a
Phase 8 defect, so it is not filed as an item here.
