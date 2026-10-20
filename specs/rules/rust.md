# Rules — Rust Engineering Standards `[ALL]`

> Load this before writing code in a module crate. Referenced from
> [`../CONSTITUTION.md`](../CONSTITUTION.md) Article V.

## 1. Module boundaries

- One crate per bounded context. `mod repo;` is **private** — no other crate can reach the
  SQL or the entities.
- **Two module crates never call each other.** Cross-module work goes through `workflows`
  `[D-97]`.
- Nothing imports `workflows` except `server`, `sync` and `jobs`.
- The crate dependency graph stays acyclic. Cargo enforces this; do not work around it with
  a shared "common" crate that grows into a second kernel.

## 2. Commands and queries

- `commands.rs` holds writes and takes `&Tx`.
- `queries.rs` holds reads and takes `&Db` (or any `ConnectionTrait`).
- The signature says which side a function is on. This is command–query separation by type —
  **not** CQRS. There is no command bus and no separate read store `[D-98]`.

## 3. Errors

- Each crate defines its error with `thiserror`. Never `anyhow` below the binary.
- The HTTP layer maps domain errors to responses in one place, in `api`. A handler that
  builds its own status code is a defect.
- An error that reaches a pharmacy owner is plain language with a next action. An error that
  reaches a log carries the trace id and the detail.

## 4. SQL

- Reports and any non-trivial query use `sqlx::query_as!` over SeaORM's own pool — checked
  against a live database at compile time `[D-99]`.
- Run `cargo sqlx prepare` and **commit `.sqlx/`** whenever a macro query changes, so the
  server builds without a database.
- Entities are generated from the database, never hand-written:
  `sea-orm-cli generate entity -o crates/<module>/src/entity`. Regenerate after every
  migration and commit the result.

## 5. Tests

- Every behavioural rule in [`tenancy.md`](tenancy.md) has a test that fails when the rule is
  removed.
- Integration tests run against a real Postgres, not a mock. RLS, triggers and grants are the
  things most likely to be wrong, and they exist only in the database.
- A test is named for the behaviour it protects — `rls_blocks_foreign_batch`, not `test_sale_2`.

## 6. Library API verification

Never write against a crate's API from recall. Check the version in `Cargo.toml` and read
`cargo doc --open -p <crate>` or the source in `~/.cargo/registry`.

Majors in this stack moved recently and feature names moved with them — `sqlx` dropped
`runtime-tokio-rustls` in favour of `runtime-tokio` plus `tls-rustls-ring`, and `apalis`
moved its Redis backend into a separate `apalis-redis` crate. Recall is stale.

`[CLAUDE]` equivalent: use the Context7 MCP server where it is connected. If it is not, the
rule above applies unchanged.
