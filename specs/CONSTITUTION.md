# Medicine POS Backend — SDD Constitution (core)

> **This file plus `specs/rules/` is the single source of truth for every engineering rule.**
> The entry points — `CLAUDE.md`, `AGENTS.md`, `GEMINI.md`, `.cursor/rules/sdd.mdc` — restate
> nothing and point here. If a rule changes, it changes here only.
>
> Status lives in [`INDEX.md`](INDEX.md), never in this file.

**This core is deliberately short.** Read it once per session. Everything else loads only
when you are about to do the thing it governs — see the table at the bottom.

---

## Article 0: Rule scope

| Tag | Meaning |
|---|---|
| `[ALL]` | Binding on every agent. Written as a plain checklist so any tool can follow it. |
| `[CLAUDE]` | Depends on Claude Code skills, subagents or hooks. Every other agent follows the `[ALL]` equivalent stated alongside it. |

A rule that exists only as a Claude skill is not binding on Codex, Antigravity or Cursor.

---

## Article I: The 5-gate lifecycle `[ALL]`

Specs are the source of truth. Code is strictly downstream.

| Gate | Artifact | Exit condition |
|---|---|---|
| **0** | — | No implementation in `crates/<module>/` before that module's `SPEC.md` exists. |
| **1** | `SPEC.md` + `CONTRACT.md` | What, why, tables owned, HTTP surface. |
| **2** | `plan.md` | Crate layout, migrations, types, commands, queries, test plan. |
| **3** | `tasks.md` | Atomic tasks, each carrying a `VERIFY:` command. |
| **4** | Implementation | One task at a time. `fmt`, `clippy -D warnings`, `test` clean. |
| **5** | Verification | Every `VERIFY:` passes, run by an agent that did not implement it. |

`server` and `migration` are permanently exempt from Gate 0 — they are composition, not
domain. Temporary exemptions are registered in [`PLACEHOLDERS.md`](PLACEHOLDERS.md).

---

## Article II: Gate 5 means executable proof `[ALL]`

**A task may not be ticked on intent. It is ticked on a command that exits 0.**

```markdown
- [ ] Reject a sale line whose batch belongs to another tenant
      VERIFY: cargo test -p sales rls_blocks_foreign_batch -- --exact
```

1. A checkbox with no `VERIFY:` line cannot be ticked.
2. **The agent that implements a task may not be the agent that verifies it.** Gate 5 runs as
   a separate pass reading only `tasks.md` and the repo — no session context, no memory of
   intent.
3. `cargo build` and `cargo clippy` passing are necessary but **never sufficient**. Rust's
   type system proves the shape of the program, not its agreement with the specification.
4. If a `VERIFY:` command cannot be written for a task, the task is too vague — split it.
5. **Prefer `cargo test` to `grep`.** A `grep` that asserts a behaviour is a false assertion.

```bash
cargo xtask verify --lint            # structural, executes nothing
cargo xtask verify --module 07       # run one module's assertions
cargo xtask verify --strict          # also fail on a tick with no VERIFY: line
```

Enforced on everyone by `crates/server/build.rs` on every compile and by `.githooks/pre-commit`
on every commit. Agent hooks add the same check at tick time. Details in
[`rules/enforcement.md`](rules/enforcement.md).

---

## Article III: Requirements live in Notion `[ALL]`

This repository owns architecture, data model, API, security and module specs. Product
requirements and the decisions log are in Notion.

**Never open `PRODUCT_SPEC.md` or fetch a Notion page to check one decision** — that is
~33,000 tokens to read one line. Use the local index:

```bash
cargo xtask d 73           # one decision, ~40 tokens
cargo xtask d "staging"    # search
```

Cite as `[D-73]`. Never restate the reasoning. **If a requirement is missing or
contradictory, ask** — do not invent it. Details in [`rules/requirements.md`](rules/requirements.md).

---

## Article IV: The five that bite `[ALL]`

The full set of tenancy and security invariants is in [`rules/tenancy.md`](rules/tenancy.md),
and you must load it before writing any migration or tenant-scoped query. These five cause
silent, total failures, so they live in the core:

1. **The app connects as a non-owner role.** The owner bypasses RLS silently — no error, no
   log, just every pharmacy's data in one response.
2. **`set_config('app.tenant_id', $1, true)` uses a bound parameter**, never `format!`. The
   `true` makes it transaction-local; without it the value leaks to the next request on that
   pooled connection.
3. **Foreign keys across tenant-owned tables are composite — `(tenant_id, id)`.**
4. **Uniqueness is tenant-scoped** — `UNIQUE (tenant_id, invoice_no)`.
5. **Money is `rust_decimal::Decimal` end to end**, never a float, at any layer including
   serialization.

---

## Article V: Load on demand

Read a rule file when you are about to do the thing it governs — not before.

| Before you… | Load | ~tokens |
|---|---|---|
| write a migration or any tenant-scoped query | [`rules/tenancy.md`](rules/tenancy.md) | 700 |
| write code in a module crate | [`rules/rust.md`](rules/rust.md) | 700 |
| draft or change a `SPEC.md` / `CONTRACT.md` | [`rules/requirements.md`](rules/requirements.md) | 700 |
| change the verifier, a hook or the build gate | [`rules/enforcement.md`](rules/enforcement.md) | 800 |
| run a command you do not recognise | [`../.specify/workflow/sdd-process.md`](../.specify/workflow/sdd-process.md) | 900 |

**Do not load all five at the start of a session.** That is how a context window gets spent
on rules instead of work.
