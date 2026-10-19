# TASKS: [NN] — [Module Title]

> Governed by [`specs/CONSTITUTION.md`](../CONSTITUTION.md) **Article II** — every leaf task
> carries a `VERIFY:` line. A task is ticked **only** when its command exits 0. Run:
>
> ```bash
> cargo xtask verify --module NN
> ```
>
> The agent that implements a task does not verify it. `cargo build` and `cargo clippy`
> passing is necessary, never sufficient.

---

- [ ] **Phase 1: Schema**
  - [ ] [Migration creating the module's tables with FORCE RLS and tenant policy]
        VERIFY: cargo test -p [name] schema_has_force_rls -- --exact
  - [ ] [Insert-only grants on the append-only ledger]
        VERIFY: cargo test -p [name] ledger_rejects_update -- --exact

- [ ] **Phase 2: Repository and entities**
  - [ ] [Generate entities and keep them private to the crate]
        VERIFY: ! grep -rq "pub mod entity" crates/[name]/src/lib.rs

- [ ] **Phase 3: Commands**
  - [ ] [One write, named for the behaviour]
        VERIFY: cargo test -p [name] [behaviour_test_name] -- --exact

- [ ] **Phase 4: Queries**
  - [ ] [One read]
        VERIFY: cargo test -p [name] [behaviour_test_name] -- --exact

- [ ] **Phase 5: HTTP**
  - [ ] [Routes registered, handlers carry no logic]
        VERIFY: ! grep -rqE "(sea_orm|sqlx)::" crates/[name]/src/rest.rs

- [ ] **Phase 6: Isolation proof**
  - [ ] [Tenant A cannot read tenant B's rows]
        VERIFY: cargo test -p [name] rls_blocks_cross_tenant_read -- --exact
  - [ ] [A write naming another tenant's id fails]
        VERIFY: cargo test -p [name] rls_blocks_cross_tenant_write -- --exact

---

## Writing a good VERIFY line

| Do | Don't |
|---|---|
| `cargo test -p sales rls_blocks_foreign_batch -- --exact` | `grep -q "tenant_id" crates/sales/src/commands.rs` |
| `! grep -rq "format!" crates/db/src/tenant.rs` | `test -f crates/db/src/tenant.rs` |
| `cargo sqlx prepare --check` | `echo done` |

A `grep` that claims to assert a behaviour is a false assertion. Use `grep` for textual
architectural rules only — "no module imports another's repo", "no `format!` beside
`set_config`". For anything about what the code *does*, write the test.

Commands run through Git Bash from the repo root. POSIX syntax only.
