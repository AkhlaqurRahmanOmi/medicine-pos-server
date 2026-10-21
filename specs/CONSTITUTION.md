# Medicine POS Backend — SDD Constitution (core)

> This file plus `specs/rules/` is the single source of truth for every engineering rule.
> A rule changes here only. Status lives in [`INDEX.md`](INDEX.md), never in this file.

**The core holds only what must be true before you act** — everything else is a command or a
rule file (Article VI). Read it once per session and load nothing else until you are about to
do the thing it governs.

`[ALL]` binds every agent; `[CLAUDE]` needs Claude Code skills, subagents or hooks, and every
other agent follows the `[ALL]` equivalent stated alongside it.

---

## Article I: Specs come first `[ALL]`

Five gates: `SPEC.md` + `CONTRACT.md` → `plan.md` → `tasks.md` → implementation →
verification. **Gate 0 is the one that binds before your first edit: no implementation in
`crates/<module>/` before that module's `SPEC.md` exists.** `server` and `migration` are
exempt; temporary exemptions live in [`PLACEHOLDERS.md`](PLACEHOLDERS.md).

The gate table, the exemption mechanics and how a module advances are in
[`rules/lifecycle.md`](rules/lifecycle.md).

---

## Article II: Gate 5 means executable proof `[ALL]`

**A task may not be ticked on intent. It is ticked on a command that exits 0**, written on
the task as `VERIFY: cargo test -p sales rls_blocks_foreign_batch -- --exact`.

1. A checkbox with no `VERIFY:` line cannot be ticked.
2. **The agent that implements a task may not be the agent that verifies it.** Gate 5 is a
   separate pass reading only `tasks.md` and the repo — no session context, no memory of
   intent.
3. `cargo build` and `cargo clippy` passing are necessary, **never sufficient**: the type
   system proves the shape of the program, not its agreement with the spec.

Enforced by `crates/server/build.rs` on every compile and `.githooks/pre-commit` on every
commit. Assertion strength, the runner's flags and the tiers:
[`rules/enforcement.md`](rules/enforcement.md).

---

## Article III: Requirements live in Notion `[ALL]`

**Notion is the source of truth.** Fetch one named page by its id — `cargo xtask n <topic>`
gives page and id, and flags the nine documents not written yet. Never search the workspace.

**One decision is never worth a page fetch:** `cargo xtask d 73`, ~40 tokens against ~33,000.
Fetch the page when the reasoning matters, when `d` has no row, or when more than one line is
in question. **`docs/` is a stale export** — read it only when Notion lacks the thing or
cannot be reached, and **Notion wins where they disagree**.

Cite as `[D-73]`, never restating the reasoning. **If a requirement is missing or
contradictory, ask** — do not invent it. [`rules/requirements.md`](rules/requirements.md).

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

## Article V: Never run it for them `[ALL]`

Neither the database nor the history is yours. An agent proposes the command; a human runs
it.

1. **Never run a migration** — `migrate up`, `down`, any schema-altering SQL — on any
   database, local included, unless asked in this session. Print the command instead.
2. **`migrate fresh`, `refresh`, `reset` drop data: never run them at all.** A dev database
   holds the seed someone spent a day on.
3. **Never regenerate entities unless asked**; it overwrites the whole target directory.
4. **A migration applied anywhere but your machine is immutable.** Write a new one.
5. One-way: **SQL migration → database → generated entities.** Never the reverse, and never
   hand-edit a generated entity.
6. **Never `git commit`, never `git push`.** Leave the work in the tree, say what changed, and
   let a human read the diff and own it — for a one-line change too.
7. **Never move history** — `rebase`, `merge`, `reset`, `cherry-pick`, `revert`, `stash`,
   `tag`, `switch`/`checkout` — and **never `--no-verify`**. Reading is free.

Full detail in [`rules/rust.md`](rules/rust.md) §4.

---

## Article VI: Commands, and loading on demand

```bash
cargo xtask status                       # module phases, one line each
cargo xtask d <number|text>              # one decision, ~40 tokens
cargo xtask n [topic]                    # the Notion page that holds it: name + id
cargo xtask verify --module 07           # run a module's assertions
cargo xtask verify --lint                # structural, executes nothing
cargo xtask new 07-sales                 # scaffold a spec directory
cargo run -p server                      # the API on :8080
cargo clippy --workspace --all-targets -- -D warnings
```

Load a rule file when you are about to do the thing it governs — never at the start of a
session, which is how a context window gets spent on rules instead of work.

All under `specs/rules/`, except the last:

| Before you… | Load (~tokens) |
|---|---|
| start a module or advance a gate | [lifecycle](rules/lifecycle.md) (600) |
| write a migration or tenant-scoped query | [tenancy](rules/tenancy.md) (700) |
| write code in a module crate | [rust](rules/rust.md) (2,400) |
| draft a `SPEC.md` / `CONTRACT.md` | [requirements](rules/requirements.md) (1,100) |
| change the verifier, a hook or the gate | [enforcement](rules/enforcement.md) (900) |
| run a command you do not recognise | [sdd-process](../.specify/workflow/sdd-process.md) (900) |
