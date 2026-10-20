# Rules — Enforcement Internals `[ALL]`

> Load this when changing the verifier, the hooks or the build gate. Day-to-day work needs
> only the `VERIFY:` rule in [`../CONSTITUTION.md`](../CONSTITUTION.md) Article II.

## Assertion strength

Use the strongest form available. This repository is Rust, so most assertions can be real.

| Strength | Form | Use for |
|---|---|---|
| Strongest | `cargo test -p <crate> <name> -- --exact` | Behaviour. Anything about what the code *does*. |
| Strong | `cargo sqlx prepare --check` · `cargo clippy -p <crate> -- -D warnings` | Query correctness, lint-expressible rules. |
| Structural | `! grep -rq "<pattern>" crates/<x>/src` | Architectural invariants — "no module imports another's repo". |
| Weakest | `test -f <path>` | File existence only. Acceptable for a scaffolding task, never for behaviour. |

**A `grep` that asserts a behaviour is a false assertion.** `grep -q "tenant_id" sales.rs`
proves the string is present, not that isolation works. If the task is about behaviour and
you reach for `grep`, write the test instead.

## The runner

```bash
cargo xtask verify                  # all modules, ticked tasks only
cargo xtask verify --module 07      # one module
cargo xtask verify --all            # also probe unticked tasks: which are ready to tick?
cargo xtask verify --strict         # also fail on a ticked task with no VERIFY: line
cargo xtask verify --lint           # structural only, executes nothing (~ms)
cargo xtask verify --hook           # as --lint, but exits 2 so an agent is blocked
```

Only *leaf* tasks are asserted — a group header with nested children is skipped. Assertions
run from the repo root through Git Bash, with a 180s timeout each.

> **Windows.** The runner resolves Git Bash by explicit path. It must: `CreateProcess`
> searches `System32` before `PATH`, and `System32\bash.exe` is the WSL launcher, so
> spawning the bare name `bash` finds WSL rather than Git Bash even when Git Bash is first
> on `PATH` — and every assertion then fails with a WSL relay error that looks nothing like
> the real cause. Override with `SDD_BASH` if Git is installed somewhere unusual.
>
> Write POSIX commands in `VERIFY:` lines — `test -f`, `grep`, `&&`. Never PowerShell.

## The four tiers

| Tier | Trigger | Cost | Binds |
|---|---|---|---|
| Agent hook | tick time, automatic | ~ms | Claude Code, Codex, Cursor |
| `build.rs` | every `cargo build` / `check` / `test` | ~ms | **everyone** |
| `pre-commit` | every `git commit` | ~ms | **everyone** |
| `verify --strict` | manual / CI | minutes | everyone; the only tier that proves assertions *hold* |

## Two constraints

1. **Never run `cargo xtask verify` (without `--lint`) from `build.rs` or from a `VERIFY:`
   line.** Many assertions invoke `cargo test`, which re-runs `build.rs`, which would invoke
   the runner again, without termination. Only `--lint` is safe inside the build graph: it
   parses files and executes nothing.
2. **The `pre-commit` hook is bypassable** with `git commit --no-verify`. That is acceptable.
   This system exists to stop an agent ticking a box it cannot defend, not to stop a
   determined human. Evading it is a decision, and a decision leaves a trace.

## Install the commit gate (once per clone)

```bash
git config core.hooksPath .githooks
```

## Gate 0 mechanics

`specs/MODULES.map` maps crates to spec directories, because one spec may cover several
crates. A crate under `crates/` must appear there, be in `EXEMPT_CRATES` (`server`,
`migration`), or be registered in [`../PLACEHOLDERS.md`](../PLACEHOLDERS.md). A crate that is
none of those fails Gate 0.
