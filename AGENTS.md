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

## Codex-specific routing

> **How the gate reaches you.** The `build.rs` gate above is your enforcement point — it runs
> on every `cargo build`, `cargo check` and `cargo test`, in any tool, and the `pre-commit`
> hook catches the same violation at commit time. Claude Code additionally has a tick-time
> hook in `.claude/settings.json`; that file is Claude-specific and you can ignore it.
> Nothing about the rule is Claude-only — only the earlier feedback is.
>
> `.codex/hooks.json` carries the same tick-time check for you where your runtime supports it.

Rules tagged `[CLAUDE]` in the constitution do not bind you. Follow the `[ALL]` equivalent
stated alongside each one. Specifically:

| Constitution rule | Your equivalent |
|---|---|
| Art. II.2 — a separate agent runs Gate 5 | Run `cargo xtask verify --module NN --strict` as its own invocation in a fresh session, reading only `tasks.md` and the repo. Never verify from the session that implemented the tasks. |
| Art. V.6 — Context7 MCP | Read the installed source under `~/.cargo/registry`, or run `cargo doc --open -p <crate>`, and check the version in `Cargo.toml`. Never write against a crate API from recall. |

---

## Install the commit gate (once per clone)

```bash
git config core.hooksPath .githooks
```
