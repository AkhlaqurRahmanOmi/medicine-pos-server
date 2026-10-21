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

## Why gates rather than a checklist

Each gate exists because its artifact is cheaper to change than the one after it. Changing
`CONTRACT.md` costs minutes; changing an implemented HTTP surface costs a migration, a
client release and a conversation with a pharmacy. The gates are not ceremony — they are the
order in which this system is cheapest to be wrong in.
