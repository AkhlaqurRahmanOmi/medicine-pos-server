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

## Claude-specific routing

Rules tagged `[CLAUDE]` in the constitution apply to you in full.

**The tick-time hook is active for you.** `.claude/settings.json` runs
`cargo xtask verify --hook` after every `Write` and `Edit`; ticking a task with no `VERIFY:`
line blocks the edit (exit 2) with the reason fed back. This is an accelerator, not the rule —
`build.rs` and `.githooks/pre-commit` bind everyone.

**Gate 5 needs a different agent.** Spawn a separate one for the verification pass, give it
only the module number, and do not hand it your session context. The point is that it has no
memory of what you intended.

**Context7 MCP** before writing against axum 0.8, SeaORM 2.0, sqlx 0.9 or utoipa 6 — all had
breaking majors recently and feature names moved with them. If it is not connected, read
`cargo doc` or the installed source instead.

```bash
git config core.hooksPath .githooks      # once per clone, enables the commit gate
```
