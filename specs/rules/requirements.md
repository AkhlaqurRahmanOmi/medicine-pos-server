# Rules — Requirements, Specs & Handoff `[ALL]`

> Load this at Gate 1, when drafting or changing a `SPEC.md` or `CONTRACT.md`.
> Referenced from [`../CONSTITUTION.md`](../CONSTITUTION.md) Articles III and VI.
>
> [`../agents/backend-architect.md`](../agents/backend-architect.md) is the design role for
> this gate — reference only, narrowed to this architecture. **This file wins**, and that
> role's *Not in this system* table is what it will refuse to design.

## Where requirements live

**Notion is the source of truth.** Fetch one named page by its id, from the local index:

```bash
cargo xtask n api          # name + id of the page that holds it
cargo xtask n              # all ten, and the nine not written yet
```

The index is [`../NOTION.map`](../NOTION.map) — a TSV, read it directly only to survey it.
Never search or crawl the workspace for a page the index already names, and never fetch a
second page speculatively.

This repository owns the specs derived from those pages — `SPEC.md`, `CONTRACT.md`, `plan.md`,
`tasks.md` — plus the tenancy, engineering and enforcement rules in this directory.

Order of lookup, cheapest first:

1. **One decision, by number or topic** — ~40 tokens, from the cached table:

   ```bash
   cargo xtask d 73          # one decision
   cargo xtask d "staging"   # search by topic or text
   ```

   `specs/DECISIONS.tsv` holds all 103 in ~1,800 tokens; read it whole only to survey them.

2. **Anything larger than one line**, or a decision whose *reasoning* matters — fetch the one
   Notion page that holds it. One page, never the project.

3. **`docs/` only when Notion does not have it**, or cannot be reached. That directory is an
   export — `PRODUCT_SPEC.md`, `RUST_STACK.md`, `DESIGN_BRIEF.md`, `SCREEN_SPECS.md` — and an
   export drifts. **Where it disagrees with Notion, Notion wins.** Open
   `docs/PRODUCT_SPEC.md` whole only to work through a whole section (§20 the data model,
   §§16–19 the stack), and only after Notion has failed you.

## When Notion is missing something

A gap in Notion is not yours to fill locally.

1. **Say what is missing and where you looked** — name the page id, not "Notion".
2. If the export has it, use it to *propose* the answer, marked as coming from `docs/`, and
   ask for it to be written into the Notion page. A requirement that lives only in an export
   is one the next session will not find.
3. If neither has it, it goes in the spec's Open Questions table and the module does not
   advance past Gate 2. **Never invent it, and never infer it from the data model.**
4. Cite the decision number once it exists — never the Notion page, never the export.

If a decision is in Notion with no row in `specs/DECISIONS.tsv`, the cache is stale: say so.
Refreshing it means re-exporting `docs/PRODUCT_SPEC.md` and running
`cargo xtask sync-decisions docs/PRODUCT_SPEC.md`, which regenerates the file — never edit it
by hand — and leaves a diff for a human to commit (Constitution Art. V.6).

## Drafting a spec

1. Read the module's row in [`../INDEX.md`](../INDEX.md), or run `cargo xtask status`.
2. Record the tables the module owns in `CONTRACT.md` — names, keys, tenant-scoped uniques,
   and which ledgers are append-only.
3. Record the HTTP surface in `CONTRACT.md` — path, method, permission, request and response
   shape, error cases.
4. Cite the decision number for anything a reader might otherwise think arbitrary, as
   `[D-73]`. **Never restate the reasoning** — a copy drifts, and a drifted copy still reads
   as authoritative.

## When a requirement is missing

**Stop and ask.** Do not invent one, and do not infer it from the data model. A requirement
invented inside a spec is indistinguishable from a real one three weeks later, and it will
be implemented.

Record it in the spec's Open Questions table until it is answered. A spec with a question
blocking Gate 2 does not advance.

## When you disagree with a spec

Say so before writing code, then:

- **Requirement question** — pricing, permissions, what a pharmacy expects: not yours to
  decide. Record it and ask.
- **Engineering question** — crate layout, query shape, error type: propose the alternative
  with its reasoning, and change the plan if it is accepted.

Never implement something other than what the spec says and leave the spec standing.

## Status and handoff

- **Status lives in [`../INDEX.md`](../INDEX.md) and nowhere else.** Not in the constitution,
  not in a module spec, not in a root-level progress file.
- Update it when a task or phase completes. The task count must match what
  `cargo xtask verify --module NN` reports — if it does not, the count is wrong.
- A root-level file duplicating rules or status is deleted, not updated.
- Finish a session by stating what passes, what fails, and what was left out. Never report a
  task complete because it was attempted.
- **You do not commit the work, and you do not push it** — Constitution Art. V.6. Leave it
  in the working tree, name the files you touched and the `VERIFY:` commands that pass, and
  draft the commit message if you like. A human reads the diff and owns it from there.
