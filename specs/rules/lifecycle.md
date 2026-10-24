# Rules — The 5-Gate Lifecycle `[ALL]`

> Load this when starting a module, advancing a gate, or scaffolding a spec directory.
> Referenced from [`../CONSTITUTION.md`](../CONSTITUTION.md) Article I.

Specs are the source of truth. Code is strictly downstream.

| Gate | Artifact | Exit condition |
|---|---|---|
| **0** | — | No implementation in `crates/<module>/` before that module's `SPEC.md` exists. |
| **1** | `SPEC.md` + `CONTRACT.md` | What, why, tables owned, HTTP surface. |
| **2** | `plan.md` | Crate layout, migrations, types, commands, queries, test plan. |
| **3** | `tasks.md` | Atomic tasks, each carrying a `VERIFY:` command. |
| **4** | Implementation | One task at a time. `fmt`, `clippy -D warnings`, `test` clean. |
| **5** | Verification | Every `VERIFY:` passes, run by an agent that did not implement it. |

## Gate 0 exemptions

`server` and `migration` are permanently exempt — they are composition, not domain.
Temporary exemptions are registered in [`../PLACEHOLDERS.md`](../PLACEHOLDERS.md). A crate
under `crates/` must appear in [`../MODULES.map`](../MODULES.map), be one of the two exempt
crates, or be registered as a placeholder; anything else fails Gate 0 on every compile.

One spec may cover several crates — module 00 covers five — which is why the mapping is a
file rather than a naming convention. Mechanics in [`enforcement.md`](enforcement.md).

## Advancing a gate

1. `cargo xtask new NN-module-name` scaffolds the directory.
2. Draft the artifact for the gate you are at, and no further. A `plan.md` written before
   `CONTRACT.md` is settled is a plan against a guess.
3. **An open question blocks the gate it would change.** A spec with a requirement question
   does not advance to Gate 2 — record it in the spec's Open Questions table and ask
   ([`requirements.md`](requirements.md)).
4. Update the phase and task count in [`../INDEX.md`](../INDEX.md), the only place status
   lives. The count must match what `cargo xtask verify --module NN` reports.
5. Gate 5 runs as a separate pass, by an agent that did not implement the tasks
   (`../CONSTITUTION.md` Article II.2).

## Optional tasks — the backlog

A module may carry a `BACKLOG.md` beside its other artifacts, holding work that is real but
does not gate: what a review found after Gate 5 closed, or what was deferred on purpose. It
is optional in the strict sense — **nothing can be failed by it.**

| | |
|---|---|
| **Counted?** | No. The module's total in [`../INDEX.md`](../INDEX.md) is what `tasks.md` asserts. A backlog is noted in the module's Notes cell, never added to the count. |
| **Run by the verifier?** | No. The runner walks files named `tasks.md` only, so `--lint`, `build.rs`, the commit hook and the tick hook are all blind to this file by construction, not by politeness. |
| **Ticked in place?** | Never. Completing an item means cutting it into `tasks.md` under a new phase, ticking it there once its command exits 0, and raising the `INDEX.md` count to match. |

Items are written in the `tasks.md` form — a checkbox and a `VERIFY:` line — so that promotion
is a move and not a rewrite. An item whose `VERIFY:` line cannot be written is too vague to
file; split it or leave it out ([`enforcement.md`](enforcement.md)).

**Why this exists.** A module at Phase 5 whose review turns up sixteen findings has only bad
options: claim it is done, or re-open a gate that was honestly passed. The backlog is the
third. The gate records what was asserted at the time; the backlog records what was learned
afterwards. Neither pretends to be the other.

**Two things do not belong here.**

1. **An open requirement question** blocks the gate it would change — record it in the spec's
   Open Questions table and ask (`Art. III`, [`requirements.md`](requirements.md)).
2. **A defect that makes a ticked assertion false** is not optional. It un-ticks that task and
   goes in `INDEX.md`'s *Failing assertions*, because a tick is a claim about proof and the
   proof is gone. Filing one as backlog instead is a decision; make it visibly, on the item.

There is no template and `cargo xtask new` does not scaffold one — a backlog exists only once
something has been learned, and an empty one in every module is noise.

## Why gates rather than a checklist

Each gate exists because its artifact is cheaper to change than the one after it. Changing
`CONTRACT.md` costs minutes; changing an implemented HTTP surface costs a migration, a
client release and a conversation with a pharmacy. The gates are not ceremony — they are the
order in which this system is cheapest to be wrong in.
