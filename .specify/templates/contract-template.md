# CONTRACT: [NN] — [Module Title]

**Status:** Draft | In Review | Approved
**Crate:** `crates/[name]`

> Gate 1 artifact. The database tables this module owns and the HTTP surface it exposes.
> Copied from the data model and the API design guide — this file is the module's slice of
> them, not a second version of them. Where they disagree, they win and this is corrected.

---

## 1. Tables owned

A table is owned by exactly one module. Another module reads it only through this module's
public queries.

| Table | Tenant-owned | Append-only | Notes |
|---|---|---|---|
| `table_name` | yes / no | yes / no | [composite FK targets, tenant-scoped uniques] |

### Keys and constraints specific to this module

- **Tenant-scoped uniques:** `UNIQUE (tenant_id, …)` — list each.
- **Composite foreign keys:** `(tenant_id, id)` references — list each.
- **Check constraints:** [any invariant enforced in the database rather than in Rust.]
- **Triggers:** [what maintains a running total, and what rebuilds it.]

### Historical copies

[Which columns are deliberate denormalised copies frozen at transaction time — price, VAT,
cost — and why re-deriving them later would be wrong. If there are none, say none.]

---

## 2. HTTP surface

| Method | Path | Permission | Request | Response | Errors |
|---|---|---|---|---|---|
| `GET` | `/v1/…` | `slug` | — | `Shape` | `404` |

### Error cases

| Condition | Status | Code | Message to the user |
|---|---|---|---|

Plain language, with a next action. The user is a pharmacy owner, not an engineer.

---

## 3. Public Rust surface

What other crates may call. Everything else is private — `repo` and `entity` always are.

```rust
// commands — take &Tx
pub async fn …(tx: &Tx, ctx: &TenantContext, …) -> Result<…, Error>;

// queries — take &Db
pub async fn …(db: &Db, ctx: &TenantContext, …) -> Result<…, Error>;
```

**Called by:** [which crates, and for what. If the answer is `workflows` only, say so.]

---

## 4. Events and jobs

| Trigger | Job | Schedule | Idempotent? |
|---|---|---|---|

---

## 5. Offline and sync

| Question | Answer |
|---|---|
| Can the mobile app create this offline? | yes / no |
| Client-generated id? | UUIDv7 / server-assigned |
| What makes an upload idempotent | [the key the server deduplicates on] |
| Can it be flagged rather than rejected? | [oversell, over-limit credit — or "no, rejected"] |
