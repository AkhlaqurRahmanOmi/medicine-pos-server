# Rules — Requirements, Specs & Handoff `[ALL]`

> Load this at Gate 1, when drafting or changing a `SPEC.md` or `CONTRACT.md`.
> Referenced from [`../CONSTITUTION.md`](../CONSTITUTION.md) Articles III and VI.

## Where requirements live

Product requirements and the decisions log are in **Notion**. This repository owns
architecture, data model, API design, security and the module specs derived from them.

**Never read `PRODUCT_SPEC.md` or fetch a Notion page to check one decision.** The spec is
~33,000 tokens and the Notion page is comparable. Use the local index instead:

```bash
cargo xtask d 73          # one decision, ~40 tokens
cargo xtask d "staging"   # search by topic or text
```

`specs/DECISIONS.tsv` holds all 103 decisions in ~1,800 tokens. Read the whole file only if
you genuinely need to survey them; read Notion only when the decision's *reasoning* matters,
which is rare and usually means you are about to propose changing it.

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
