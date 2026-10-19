# Medicine POS Backend — Spec-Driven Development Constitution

> **This file is the single source of truth for every engineering rule in this repository.**
>
> `CLAUDE.md`, `AGENTS.md`, `GEMINI.md` and `.cursor/rules/sdd.mdc` at the repo root are thin
> entry points that point here. They MUST NOT restate rules. If a rule changes, it changes
> **here only**.
>
> Status and progress live in [`specs/INDEX.md`](INDEX.md), never in this file.

---

## Article 0: Rule Scope & Agent Capability

Not every agent has the same tooling. Each rule below is tagged:

| Tag | Meaning |
|---|---|
| `[ALL]` | Binding on every agent — Claude, Codex, Antigravity/Gemini, Cursor, and subagents. Written as a plain checklist so any tool can follow it. |
| `[CLAUDE]` | Depends on Claude Code skills, subagents or hooks. Every other agent MUST follow the `[ALL]` equivalent stated alongside it. |

**A rule that exists only as a Claude skill is not binding on Codex, Antigravity or Cursor.**
Where a `[CLAUDE]` rule exists, its manual equivalent is stated in the same place.

---

## Article I: The 5-Gate Lifecycle `[ALL]`

Specs in `specs/` are the source of truth. Code is strictly downstream. Every module moves
through five gates in order:

| Gate | Artifact | Exit condition |
|---|---|---|
| **0** | — | No implementation may exist in `crates/<module>/` for a module whose `SPEC.md` does not exist. |
| **1** | `SPEC.md` + `CONTRACT.md` | What, why, the tables it owns, and the HTTP surface it exposes. |
| **2** | `plan.md` | How: types, commands, queries, migrations, error cases. |
| **3** | `tasks.md` | Atomic tasks, each carrying a `VERIFY:` command (Article II). |
| **4** | Implementation | One task at a time. `cargo fmt --check`, `cargo clippy -- -D warnings` and `cargo test` clean. |
| **5** | Verification & sign-off | Every `VERIFY:` passes. `cargo xtask verify --module NN` exits 0. |

### Gate 0 is binding

Writing `crates/sales/src/commands.rs` before `specs/07-sales/SPEC.md` exists is a
constitution violation. `cargo xtask verify` checks this on every run, before any task
assertion, and the `build.rs` gate checks it on every compile.

**Two permanent exemptions**, because they are composition, not domain:

- `crates/server` — the binary. Wires modules together; owns no rules of its own.
- `crates/migration` — the migrator. Its content is governed by the module spec that
  introduced each migration.

**One temporary exemption — the declared placeholder.** An empty crate that reserves a name
may exist before its spec, provided it is registered in [`PLACEHOLDERS.md`](PLACEHOLDERS.md)
and stays inert: no `pub fn` beyond a module declaration, no dependency on `db` or `sea-orm`,
and within its declared line cap. The moment it touches the database it is an implementation
and Gate 0 applies in full.

---

## Article II: Gate 5 Means Executable Proof `[ALL]`

**A task may not be ticked on intent. It is ticked on a command that exits 0.**

Every task in `tasks.md` carries a machine-checkable assertion:

```markdown
- [ ] Reject a sale line whose batch belongs to another tenant
      VERIFY: cargo test -p sales rls_blocks_foreign_batch -- --exact
```

### Rules

1. A checkbox with no `VERIFY:` line cannot be ticked.
2. The agent that implements a task **may not** be the agent that verifies it. Gate 5 runs
   as a separate pass reading only `tasks.md` and the repo — no session context, no memory
   of intent.
3. `cargo build` and `cargo clippy` passing are necessary but **never sufficient**. Code that
   compiles cleanly can still contradict its own spec. Rust's type system proves the shape of
   the program, not its agreement with the specification.
4. If a `VERIFY:` command cannot be written for a task, the task is too vague — split it.

### Prefer a test to a grep

This repository is Rust, so most assertions can be real. Use the strongest form available:

| Strength | Form | Use for |
|---|---|---|
| Strongest | `cargo test -p <crate> <test_name> -- --exact` | Behaviour. Anything about what the code *does*. |
| Strong | `cargo sqlx prepare --check` · `cargo clippy -p <crate> -- -D warnings` | Query correctness, lint-expressible rules. |
| Structural | `! grep -rq "<pattern>" crates/<x>/src` | Architectural invariants — "no module imports another's repo". |
| Weakest | `test -f <path>` | File existence only. Acceptable for a scaffolding task, never for behaviour. |

**A `grep` that asserts a behaviour is a false assertion.** `grep -q "tenant_id" sales.rs`
proves the string is present, not that isolation works. If the task is about behaviour and
you reach for `grep`, write the test instead.

### The runner

`cargo xtask verify` executes every `VERIFY:` line in bulk:

```bash
cargo xtask verify                  # all modules, ticked tasks only
cargo xtask verify --module 07      # one module
cargo xtask verify --all            # also probe unticked tasks: which are ready to tick?
cargo xtask verify --strict         # also fail on ticked tasks with no VERIFY: line
cargo xtask verify --lint           # structural only, executes nothing (~ms)
```

Exit 0 means every ticked assertion holds. Exit 1 means a completion claim is not true.

Only *leaf* tasks are asserted — a group header ("Phase 1: …") with nested children is
skipped. Assertions run from the repo root through `bash`, with a 180s timeout each.

> **Windows note.** Assertions run via Git Bash (`bash -c`). Write POSIX commands — `test -f`,
> `grep`, `&&` — never PowerShell or `cmd` syntax. Git Bash must be on `PATH`.

### Three tiers of enforcement

| Tier | Trigger | Cost | Binds |
|---|---|---|---|
| Hook | tick time, automatic | ~ms | Claude Code, Cursor, Codex (where hooks are supported) |
| `build.rs` | every `cargo build` / `cargo check` | ~ms | **every agent and every human** |
| `cargo xtask verify --strict` | manual / CI | minutes | everyone; the only tier that proves assertions *hold* |
| `pre-commit` | every `git commit` | ~ms | **every agent and every human** |

### Two constraints on enforcement

1. **Never run `cargo xtask verify` (without `--lint`) from `build.rs` or from a `VERIFY:`
   line.** Many assertions invoke `cargo test`, which re-runs `build.rs`, which would invoke
   the runner again, without termination. Only `--lint` is safe inside the build graph: it
   parses files and executes nothing.
2. **The `pre-commit` hook is bypassable** with `git commit --no-verify`. That is acceptable.
   This system exists to stop an agent ticking a box it cannot defend, not to stop a
   determined human. Evading it is a decision, and a decision leaves a trace.

---

## Article III: Requirement Extraction `[ALL]`

Product requirements and the decisions log live in **Notion**, not here. This repository owns
architecture, data model, API design, security and the module specs derived from them.

Before drafting or updating any `SPEC.md`:

1. Read the module's row in [`INDEX.md`](INDEX.md) for its bounded context.
2. Record the tables the module owns in `CONTRACT.md`, copied from the data model — table
   names, keys, tenant-scoped uniques, and which ledgers are append-only.
3. Record the HTTP surface in `CONTRACT.md` — path, method, permission, request and response
   shape, and error cases.
4. Cite the decision number for anything a reader might otherwise think was arbitrary, in the
   form `[D-73]`. Never restate the decision's reasoning; cite it.

**If a requirement is missing or contradictory, stop and ask.** Do not invent one and do not
infer it from the code. A requirement invented inside a spec is indistinguishable from a real
one three weeks later, and it will be implemented.

---

## Article IV: Multi-Tenancy & Security Invariants `[ALL]`

These are the rules that a passing test suite will not catch and a code review usually
misses. They are binding on every line of code in this repository.

1. **Every tenant-owned table carries `tenant_id`, including child tables.** A child row is
   never scoped only through its parent.
2. **Row-level security is `FORCE`d on every tenant-owned table.** A table without
   `FORCE ROW LEVEL SECURITY` is a defect regardless of whether a policy exists.
3. **The application connects as a non-owner role.** The owner role bypasses RLS silently —
   no error, no log, just every pharmacy's data in one response. Migrations use the owner
   role; the running server never does.
4. **`set_config('app.tenant_id', $1, true)` uses a bound parameter.** Never `format!`, never
   string concatenation. The `true` makes it transaction-local; without it the setting leaks
   to the next request on that pooled connection.
5. **Foreign keys across tenant-owned tables are composite — `(tenant_id, id)`.** A plain `id`
   FK permits a row in one tenant to reference a row in another.
6. **Uniqueness is tenant-scoped.** `UNIQUE (tenant_id, invoice_no)`, never `UNIQUE (invoice_no)`.
7. **Append-only ledgers have insert-only grants** — `stock_movements`, `supplier_ledger_entries`,
   `customer_ledger_entries`, `audit_logs`. No `UPDATE`, no `DELETE`, enforced by grant and not
   by convention.
8. **No polymorphic foreign keys**, with exactly the two exceptions the data model documents.
   A new one requires a decision recorded in Notion first.
9. **Money is `rust_decimal::Decimal` end to end.** Never `f32`, never `f64`, at any layer
   including serialization. A float that reaches a ledger is a defect, not a rounding issue.
10. **Identifiers are UUIDv7, generated by the client for anything the mobile app can create
    offline.** This is what makes an upload idempotent.

**Every one of these is cheap to assert.** Article II's structural tier exists for exactly
this: a `grep` that proves no `format!` sits next to `set_config` is a legitimate assertion,
because the rule *is* textual.

---

## Article V: Rust Engineering Standards `[ALL]`

### 1. Module boundaries

- One crate per bounded context. `mod repo;` is **private** — no other crate can reach the
  SQL or the entities.
- **Two module crates never call each other.** Cross-module work goes through `workflows`.
- Nothing imports `workflows` except `server`, `sync` and `jobs`.
- The crate dependency graph stays acyclic. Cargo enforces this; do not work around it with
  a shared "common" crate that grows into a second kernel.

### 2. Commands and queries

- `commands.rs` holds writes and takes `&Tx`.
- `queries.rs` holds reads and takes `&Db` (or any `ConnectionTrait`).
- The signature says which side a function is on. This is command–query separation by type —
  **not** CQRS. There is no command bus and no separate read store, by decision `[D-98]`.

### 3. Errors

- Each crate defines its error with `thiserror`. Never `anyhow` below the binary.
- The HTTP layer maps domain errors to responses in one place, in `api`. A handler that
  builds its own status code is a defect.
- An error message that reaches a pharmacy owner is plain language with a next action. An
  error message that reaches a log carries the trace id and the detail.

### 4. SQL

- Reports and any non-trivial query use `sqlx::query_as!` over SeaORM's own pool — checked
  against a live database at compile time `[D-99]`.
- Run `cargo sqlx prepare` and **commit `.sqlx/`** whenever a macro query changes, so the
  server builds without a database.
- Entities are generated from the database, never hand-written:
  `sea-orm-cli generate entity -o crates/<module>/src/entity`. Regenerate after every
  migration and commit the result.

### 5. Tests

- Every behavioural rule in Article IV has a test that fails when the rule is removed. A test
  that passes whether or not RLS is enabled is not testing RLS.
- Integration tests run against a real Postgres, not a mock. RLS, triggers and grants are the
  things most likely to be wrong, and they exist only in the database.
- A test is named for the behaviour it protects — `rls_blocks_foreign_batch`, not `test_sale_2`.

### 6. Library API verification `[ALL]`

Never write against a crate's API from recall. Check the version in `Cargo.toml` and the
generated docs (`cargo doc --open -p <crate>`) or the source in `~/.cargo/registry`. Majors
in this stack have moved recently and feature names changed with them — `sqlx` dropped
`runtime-tokio-rustls`, `apalis` moved its Redis backend to a separate crate. Recall is stale.

`[CLAUDE]` equivalent: use the Context7 MCP server where it is connected. If it is not, the
`[ALL]` rule above applies unchanged.

---

## Article VI: Handoff & Status `[ALL]`

- **Status lives in [`INDEX.md`](INDEX.md) and nowhere else.** Not in this file, not in a
  module `SPEC.md`, not in a root-level progress file, not in a commit message alone.
- Update `INDEX.md` when a task or a phase completes. One row per module; the task count is
  `done/total` and must match what `cargo xtask verify --module NN` reports.
- A root-level file that duplicates rules or status is deleted, not updated. Duplicates drift,
  and a drifted duplicate is worse than no file — it reads as authoritative.
- Finish a session by stating what passes, what fails, and what was left out. Never report a
  task complete because it was attempted.

---

## Article VII: Universal Enforcement

| What | Where | Binds |
|---|---|---|
| Tick without `VERIFY:` | `build.rs`, `pre-commit`, agent hooks | everyone |
| Gate 0 violation | `build.rs`, `cargo xtask verify` | everyone |
| Assertion does not hold | `cargo xtask verify --strict` | everyone, manually or in CI |

Install the commit hook once per clone:

```bash
git config core.hooksPath .githooks
```

**Nothing in this system is Claude-exclusive.** The hooks only make the feedback arrive
sooner. The rule is enforced for every agent by `build.rs` and `pre-commit`, both of which
need no editor, no MCP server, and no particular model.
