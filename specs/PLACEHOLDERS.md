# Declared Placeholders

> Gate 0 (constitution Article I) forbids implementation in `crates/<module>/` before that
> module's `SPEC.md` exists. This file is the only exemption, and it is temporary by design.

A placeholder is an **empty crate that reserves a name** so the workspace compiles. It must:

- contain nothing but a module declaration or an empty `lib.rs`;
- declare no `pub fn` that does work;
- depend on neither `db` nor `sea-orm` nor `sqlx`;
- stay within its declared line cap.

The moment it touches the database it is an implementation, and Gate 0 applies in full —
`cargo xtask verify` will fail until its `SPEC.md` exists.

## Registered placeholders

| Crate | Line cap | Registered | Reason |
|---|---|---|---|
| — | — | — | None. No module crates exist yet. |

## Permanently exempt

These are composition, not domain, and need no spec of their own:

| Crate | Why |
|---|---|
| `server` | The binary. Wires modules together and owns no rules. |
| `migration` | The migrator. Each migration is governed by the module spec that introduced it. |
| `tools/*` | The SDD runner itself. |
