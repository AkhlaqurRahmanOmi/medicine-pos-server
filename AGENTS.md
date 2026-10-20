# Medicine POS Backend

Multi-tenant pharmacy POS for Bangladesh. Rust workspace: axum, SeaORM over PostgreSQL with
row-level security, Redis. ৳ BDT, Asia/Dhaka. Mobile app and admin panel are separate repos.

**Spec-driven. Read [`specs/CONSTITUTION.md`](specs/CONSTITUTION.md) (~1,400 tokens) before
your first edit.** It is the only rulebook; this file restates none of it and records no
status. Status is [`specs/INDEX.md`](specs/INDEX.md), or `cargo xtask status`.

## Two things that catch people out

**A task is ticked on a command that exits 0, never on intent.** `cargo build` passing is
necessary, never sufficient. Enforced on every compile by `crates/server/build.rs`.

**Never open `PRODUCT_SPEC.md` or fetch Notion to check one decision** — that is ~33,000
tokens for one line. Use `cargo xtask d 73`, which costs about 40.

## Commands

```bash
cargo run -p server                      # the API on :8080
cargo clippy --workspace --all-targets -- -D warnings
cargo xtask verify --module 07           # run a module's assertions
cargo xtask d <number|text>              # look up a decision
cargo xtask status                       # module phases, one line each
cargo xtask new 07-sales                 # scaffold a spec directory
```

---

## Codex-specific routing

**How the gate reaches you.** `crates/server/build.rs` runs on every `cargo build`, `check`
and `test`, in any tool, and `.githooks/pre-commit` catches the same violation at commit
time. `.codex/hooks.json` adds the tick-time check where your runtime supports it.
`.claude/settings.json` is Claude-specific — ignore it. Nothing about the rule is
Claude-only; only the earlier feedback is.

Rules tagged `[CLAUDE]` do not bind you. Use the `[ALL]` equivalent:

| Constitution rule | Your equivalent |
|---|---|
| Art. II.2 — a separate agent runs Gate 5 | Run `cargo xtask verify --module NN --strict` as its own invocation in a fresh session, reading only `tasks.md` and the repo. Never verify from the session that implemented the tasks. |
| `rules/rust.md` §6 — Context7 MCP | Read the installed source under `~/.cargo/registry`, or run `cargo doc --open -p <crate>`, and check the version in `Cargo.toml`. Never write against a crate API from recall. |

```bash
git config core.hooksPath .githooks      # once per clone, enables the commit gate
```
