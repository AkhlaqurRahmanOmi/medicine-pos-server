# Medicine POS Backend

Multi-tenant pharmacy POS for Bangladesh. Rust workspace: axum, SeaORM over PostgreSQL with
row-level security, Redis. ৳ BDT, Asia/Dhaka. Mobile app and admin panel are separate repos.

**Spec-driven. Read [`specs/CONSTITUTION.md`](specs/CONSTITUTION.md) (~1,300 tokens) before
your first edit.** It is the only rulebook, it carries the commands, and it restates nothing
here. Status is [`specs/INDEX.md`](specs/INDEX.md), or `cargo xtask status`.

Enforced mechanically, not on trust: `crates/server/build.rs` on every compile,
`.githooks/pre-commit` on every commit.

---

## Antigravity / Gemini routing

The hook files under `.claude/`, `.codex/` and `.cursor/` are tool-specific — ignore them.
`build.rs` and `.githooks/pre-commit` reach you regardless.

Rules tagged `[CLAUDE]` do not bind you. Use the `[ALL]` equivalent:

| Constitution rule | Your equivalent |
|---|---|
| Art. II.2 — a separate agent runs Gate 5 | Run `cargo xtask verify --module NN --strict` in a fresh session that did not implement the tasks. |
| Art. V.6 — an agent never commits | Leave the work in the tree and say what changed. Nothing blocks the command for you; the rule is the same. |
| `rules/rust.md` §6 — Context7 MCP | Read the installed source under `~/.cargo/registry`, or `cargo doc --open -p <crate>`. Never write against a crate API from recall. |
| Art. III — Notion MCP | `cargo xtask n <topic>` gives the page id. Without a connector, ask for the page rather than working from `docs/` silently. |

```bash
git config core.hooksPath .githooks      # once per clone, enables the commit gate
```
