# Medicine POS Backend

Multi-tenant pharmacy POS for Bangladesh. Rust workspace: axum, SeaORM over PostgreSQL with
row-level security, Redis. ৳ BDT, Asia/Dhaka. Mobile app and admin panel are separate repos.

**Spec-driven. Read [`specs/CONSTITUTION.md`](specs/CONSTITUTION.md) (~1,300 tokens) before
your first edit.** It is the only rulebook, it carries the commands, and it restates nothing
here. Status is [`specs/INDEX.md`](specs/INDEX.md), or `cargo xtask status`.

Enforced mechanically, not on trust: `crates/server/build.rs` on every compile,
`.githooks/pre-commit` on every commit.

---

## Claude-specific routing

Rules tagged `[CLAUDE]` in the constitution apply to you in full.

- **The tick-time hook is active.** `.claude/settings.json` runs `cargo xtask verify --hook`
  after every `Write` and `Edit`; ticking a task with no `VERIFY:` line blocks the edit
  (exit 2) with the reason fed back. An accelerator, not the rule — `build.rs` and the commit
  hook bind everyone.
- **Git is denied, not discouraged.** `.claude/settings.json` blocks `git commit`, `git push`
  and the history-moving commands — Article V.6–7, mechanically.
- **Gate 5 needs a different agent.** Spawn one for the verification pass, give it only the
  module number, and do not hand it your session context. The point is that it has no memory
  of what you intended.
- **Notion MCP first for a requirement.** `cargo xtask n <topic>` for the id, then
  `notion-fetch` — never `notion-search` for a page the index names. No connector: say so and
  fall back to `docs/`, flagged as the export.
- **Context7 MCP** before writing against axum 0.8, SeaORM 2.0, sqlx 0.9 or utoipa 6 — recent
  breaking majors moved feature names with them. Not connected: read `cargo doc` or the
  installed source.

```bash
git config core.hooksPath .githooks      # once per clone, enables the commit gate
```
