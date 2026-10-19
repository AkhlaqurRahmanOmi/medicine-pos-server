# Spec-Driven Development — Process Guide

> The rules are in [`specs/CONSTITUTION.md`](../../specs/CONSTITUTION.md). This file is the
> procedure: what to do, in what order, with which command. It adds no rules.

---

## Starting a module

```bash
cargo xtask new 07-sales
```

Creates `specs/07-sales/` from the templates with `SPEC.md`, `CONTRACT.md`, `plan.md` and
`tasks.md`, and adds the row to `specs/INDEX.md` if it is missing.

---

## The five gates, as a procedure

### Gate 1 — `SPEC.md` + `CONTRACT.md`

1. Read the module's bounded context in the data model and the relevant decisions in Notion.
2. Fill `SPEC.md`: purpose, what is **not** this module's job, decisions cited by number,
   EARS acceptance criteria, tenancy and permissions.
3. Fill `CONTRACT.md`: tables owned, HTTP surface, public Rust surface, offline behaviour.
4. Any requirement you cannot find — **ask**. Do not infer it from the data model, and do not
   invent it. Record it in the spec's Open Questions table until it is answered.
5. Set the module to `Phase 1` in `INDEX.md`.

### Gate 2 — `plan.md`

Crate layout, migrations with their RLS and grants, types, commands, queries, and the test
plan that maps every `[AC-*]` to a test name. Set `Phase 2`.

### Gate 3 — `tasks.md`

Atomic tasks grouped into phases, each leaf carrying a `VERIFY:` line. Then check the
structure before writing any code:

```bash
cargo xtask verify --lint
```

Set `Phase 3` / `[SPEC APPROVED]`.

### Gate 4 — implement

One task at a time. After each:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p <crate>
```

Tick the task **only** when its own `VERIFY:` command exits 0 when you run it yourself. Set
`Phase 4` / `[WIP]`.

### Gate 5 — verify

Run as a **separate pass**, by an agent that did not implement the tasks:

```bash
cargo xtask verify --module NN --strict
```

Exit 0 and every leaf ticked means `Phase 5` / `[DONE]`. Update `INDEX.md`. A failure means
either the code is wrong or the tick was premature — fix the code or un-tick the box, and
record the failure in the Failing Assertions section of `INDEX.md` until it is resolved.

---

## Everyday commands

```bash
cargo xtask verify --lint          # structural, executes nothing, ~ms
cargo xtask verify                 # run every ticked assertion
cargo xtask verify --module 07     # one module
cargo xtask verify --all           # probe unticked tasks: which are ready to tick?
cargo xtask verify --strict        # also fail on a ticked task with no VERIFY: line
```

```bash
cargo run -p server                # the API
cargo sqlx prepare --workspace     # after changing any query_as! macro — commit .sqlx/
sea-orm-cli generate entity -o crates/<module>/src/entity   # after any migration
```

---

## Install the commit gate (once per clone)

```bash
git config core.hooksPath .githooks
```

Without this, the `pre-commit` tier is inactive and only `build.rs` and the agent hooks bind.

---

## What to do when you disagree with a spec

Say so, in the session, before writing code. Then do one of two things:

- If it is a **requirement** question — pricing, permissions, what a pharmacy expects — it is
  not yours to decide. Record it in the spec's Open Questions table and ask.
- If it is an **engineering** question — crate layout, query shape, error type — propose the
  alternative with its reasoning, and change the plan if it is accepted.

Do not implement something other than what the spec says and leave the spec standing. A spec
that disagrees with its code is worse than no spec, because it is still read as true.
