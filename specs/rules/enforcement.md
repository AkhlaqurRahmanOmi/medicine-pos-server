# Rules — Enforcement Internals `[ALL]`

> Load this when changing the verifier, the hooks or the build gate. Day-to-day work needs
> only the `VERIFY:` rule in [`../CONSTITUTION.md`](../CONSTITUTION.md) Article II.

## Writing an assertion

A task carries its proof, written under the checkbox:

```markdown
- [ ] Reject a sale line whose batch belongs to another tenant
      VERIFY: cargo test -p sales rls_blocks_foreign_batch -- --exact
```

Two rules the core states only in passing (Article II):

- **If a `VERIFY:` command cannot be written for a task, the task is too vague — split it.**
  A task whose completion cannot be expressed as a command is a task nobody can check.
- **Prefer `cargo test` to `grep`.** `cargo build` and `cargo clippy` passing are necessary
  and never sufficient; a `grep` that asserts a behaviour is a false assertion.

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

## Citation drift

The entry points and these rule files cite the core by number — `Art. II.2`, `Art. V.6`.
Renumbering an article invalidates every such reference silently, and a stale citation still
reads as authoritative.

`citation_drift` resolves each one against the constitution as it is now: an unknown article,
or an item number past the end of that article's numbered list, fails. It runs wherever
`--lint` runs, so the build and the commit gate both catch it.

- Recognised forms: `Article V`, `Articles III and VI`, `Art. II.2`, `Art. V.6–7` (a range
  must have both ends resolve).
- Scanned: every `.md`, `.mdc` and `.rs` outside `target/`, plus `.githooks/`. `docs/` is
  excluded — it is a product export, where "Article" means something else.
- A numbered item counts only at column zero in the constitution, and never inside a fenced
  block, where a `1.` is an example rather than a rule.

```bash
VERIFY: cargo test -p sdd-lint citation -- --exact
```

## The four tiers

| Tier | Trigger | Cost | Binds |
|---|---|---|---|
| Agent hook | tick time, automatic | ~ms | Claude Code, Codex, Cursor |
| `build.rs` | every `cargo build` / `check` / `test` | ~ms | **everyone** |
| `pre-commit` | every `git commit` | ~ms | **everyone** |
| `verify --strict` | manual / CI | minutes | everyone; the only tier that proves assertions *hold* |

> **Agents do not commit** (Constitution Art. V.6), so the `pre-commit` tier fires when a
> human commits an agent's work. That is the point: the gate runs on a diff somebody is about
> to own. `.claude/settings.json` denies `git commit`, `git push` and the history-moving
> commands outright, so for Claude the rule is mechanical and not a matter of good intent;
> other runtimes enforce it by the constitution alone.

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
