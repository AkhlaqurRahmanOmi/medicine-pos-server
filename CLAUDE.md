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
- **The `backend-architect` agent is installed.** `.claude/agents/backend-architect.md` is a
  symlink to `specs/agents/backend-architect.md` — spawn it for a Gate 1–2 design pass on one
  module, handing it the module number and the Notion requirement, not your session context.
  It is narrowed to this architecture, not a generic one — its *Not in this system* table is
  the part that keeps a design pass from inventing brokers and sagas — and it is never the
  Gate 5 verifier (Art. II.2). Edit the file under `specs/`, never the symlink.
- **Notion MCP first for a requirement.** `cargo xtask n <topic>` for the id, then
  `notion-fetch` — never `notion-search` for a page the index names. No connector: say so and
  fall back to `docs/`, flagged as the export.
- **The `rust-pro` skill is installed.** `.claude/skills/rust-pro/SKILL.md` is a symlink to
  `specs/skills/rust-pro/SKILL.md` — invoke it before writing code in a module crate, next to
  `rules/rust.md` (Art. VI). Borrowed expertise, not a rule: the house rules win, and the
  skill's *House overrides* table says where. Edit the file under `specs/`, never the symlink.
- **Context7 MCP** before writing against axum 0.8, SeaORM 2.0, sqlx 0.9 or utoipa 6 — recent
  breaking majors moved feature names with them. Not connected: read `cargo doc` or the
  installed source.

```bash
git config core.hooksPath .githooks      # once per clone, enables the commit gate
```
