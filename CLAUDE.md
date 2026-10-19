# Medicine POS Backend — Agent Entry Point

Multi-tenant pharmacy point-of-sale for Bangladesh, sold as a subscription. Rust workspace:
axum, SeaORM over PostgreSQL with row-level security, Redis. Currency ৳ (BDT), timezone
Asia/Dhaka. The mobile app and admin panel are separate repositories.

---

## Read this first

> **All engineering rules live in [`specs/CONSTITUTION.md`](specs/CONSTITUTION.md).**
> **All status lives in [`specs/INDEX.md`](specs/INDEX.md).**
>
> This file is an entry point, not a rulebook. It does not restate rules, and it is not where
> progress is recorded. If either of those files disagrees with this one, they win.

Read `specs/CONSTITUTION.md` before your first edit in a session.

## Where requirements live

Product requirements and the decisions log are in **Notion**, not in this repository. This
repo owns architecture, data model, API design, security and the module specs derived from
them. Cite a decision by number — `[D-73]` — and never restate its reasoning.

**If a requirement is missing or contradictory, ask.** Do not infer it from the data model
and do not invent it. Constitution Article III.

## The five gates

```
Gate 1  SPEC.md + CONTRACT.md   what & why, tables owned, HTTP surface
Gate 2  plan.md                 crate layout, migrations, types, commands, queries, test plan
Gate 3  tasks.md                atomic tasks, each with a VERIFY: line
Gate 4  implement               one task at a time, fmt + clippy + test clean
Gate 5  verify                  every VERIFY: passes, by an agent that did not implement it
```

**A task is ticked on a command that exits 0, never on intent.** `cargo build` and
`cargo clippy` passing is necessary, never sufficient.

## Commands

```bash
cargo run -p server                      # the API on :8080
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p <crate>
```

```bash
cargo xtask verify                       # every ticked assertion
cargo xtask verify --module 07           # one module
cargo xtask verify --all                 # probe unticked tasks: which are ready to tick?
cargo xtask verify --strict              # also fail on a ticked task with no VERIFY: line
cargo xtask new 07-sales                 # scaffold a spec directory from the templates
```

**`cargo build` is gated.** `crates/server/build.rs` runs the structural lint on every
compile — it parses files and executes nothing. Tick a task without a `VERIFY:` line and the
build fails until you add one. This is the enforcement floor for **every** agent: it needs no
editor, no hook system and no git.

---

## Claude-specific routing

> **The tick-time hook is active for you.** `.claude/settings.json` runs
> `cargo xtask verify --hook` after every `Write` and `Edit`. Tick a task with no `VERIFY:`
> line and the edit is blocked (exit 2) with the reason fed back, so the assertion gets added
> before work continues.
>
> This is an accelerator, **not** the rule. The rule binds everyone through `build.rs` and
> the `pre-commit` hook. Nothing here is Claude-exclusive except the speed of the feedback.

Rules tagged `[CLAUDE]` in the constitution apply to you in full.

### MCP

- **Context7** — `resolve-library-id` → `query-docs`. Use it before writing against axum 0.8,
  SeaORM 2.0, sqlx 0.9 or utoipa 6. Several of these had breaking majors recently and feature
  names moved with them; recall is stale. If Context7 is not connected, constitution
  Article V.6 `[ALL]` applies instead — read the installed source or `cargo doc`.

### Subagents

Gate 5 must be run by an agent that did not implement the tasks (Article II.2). Spawn a
separate agent for the verification pass, give it only the module number, and let it read
`tasks.md` and the repo. Do not hand it your session context — the whole point is that it has
no memory of what you intended.

---

## Install the commit gate (once per clone)

```bash
git config core.hooksPath .githooks
```
