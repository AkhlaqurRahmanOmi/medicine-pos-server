# Medicine POS Backend

Multi-tenant pharmacy POS for Bangladesh. Rust workspace: axum, SeaORM over PostgreSQL with
row-level security, Redis. ৳ BDT, Asia/Dhaka. Mobile app and admin panel are separate repos.

**Spec-driven. Read [`specs/CONSTITUTION.md`](specs/CONSTITUTION.md) (~1,300 tokens) before
your first edit.** It is the only rulebook, it carries the commands, and it restates nothing
here. Status is [`specs/INDEX.md`](specs/INDEX.md), or `cargo xtask status`.

Enforced mechanically, not on trust: `crates/server/build.rs` on every compile,
`.githooks/pre-commit` on every commit.

---

## Codex-specific routing

`.codex/hooks.json` adds the tick-time check where your runtime supports it. `.claude/` is
Claude-specific — ignore it. Nothing about the rules is Claude-only; only the feedback speed
is.

Rules tagged `[CLAUDE]` do not bind you. Use the `[ALL]` equivalent:

| Constitution rule | Your equivalent |
|---|---|
| Art. II.2 — a separate agent runs Gate 5 | Run `cargo xtask verify --module NN --strict` as its own invocation in a fresh session, reading only `tasks.md` and the repo. Never verify from the session that implemented the tasks. |
| Art. V.6 — an agent never commits | Leave the work in the tree and say what changed. No deny list enforces this for you; the rule is the same. |
| `rules/rust.md` §6 — Context7 MCP | Read the installed source under `~/.cargo/registry`, or `cargo doc --open -p <crate>`, and check the version in `Cargo.toml`. Never write against a crate API from recall. |
| Art. III — Notion MCP | `cargo xtask n <topic>` gives the page id. Without a connector, ask for the page rather than working from `docs/` silently. |

```bash
git config core.hooksPath .githooks      # once per clone, enables the commit gate
```
