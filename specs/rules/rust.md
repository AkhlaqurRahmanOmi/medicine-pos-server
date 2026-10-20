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

## 4. SQL, migrations and generated code

- Reports and any non-trivial query use `sqlx::query_as!` over SeaORM's own pool — checked
  against a live database at compile time `[D-99]`.
- Run `cargo sqlx prepare` and **commit `.sqlx/`** whenever a macro query changes, so the
  server builds without a database.

### The one-way street

```
hand-written SQL migration  →  database  →  generated entities
```

Never the other way. SeaORM will not derive a migration from an entity, and neither will you
by hand — a schema that exists in two places diverges, and the database is the one that is
actually true.

- **Migrations are hand-written raw SQL** in `crates/migration`. Each one creates its tables,
  enables `FORCE ROW LEVEL SECURITY`, writes the policy, and sets the grants, in one file.
  A migration that creates a tenant-owned table without its policy is incomplete.
- **Entities are generated**, never hand-written:
  `sea-orm-cli generate entity -o crates/<module>/src/entity`. Commit the result.
- **Never hand-edit a generated entity.** It is overwritten the next time anyone regenerates,
  silently, and the edit returns as a bug weeks later. If the entity is wrong, the migration
  is wrong.

### What an agent may not run

Constitution Article V, stated here in full because this is where it bites:

| Command | Rule |
|---|---|
| `sea-orm-cli migrate up` / `down` | **Only when asked in this session.** Otherwise print it and stop |
| `sea-orm-cli migrate fresh` / `refresh` / `reset` | **Never. On any database.** They drop data |
| `sea-orm-cli generate entity` | Only when asked. It overwrites every file in the target directory, committed or not |
| Any `DROP`, `TRUNCATE`, or `ALTER … DROP COLUMN` outside a reviewed migration file | Never |

Writing a migration file is not running it. Write it, say what it will do, and let a human
apply it.

**A migration that has been applied anywhere but your own machine is immutable.** Fix it with
a new migration. An edited one is applied on some databases and not others, and nothing in
the tooling will tell you which.

### Assertable

```bash
# every migration that creates a tenant-owned table also forces RLS
VERIFY: cargo test -p migration every_tenant_table_forces_rls -- --exact
# every generated entity still carries the generator header — nobody hand-edited one
VERIFY: test -z "$(grep -rLs 'sea-orm-codegen' crates/*/src/entity/*.rs)"
```

> `grep -L` lists the files that **lack** the pattern, so the assertion is "that list is
> empty". Do not write `! grep -rLq …` — `-q` suppresses the output that `-L` exists to
> produce, and the command then passes on a hand-edited file. Tested; it does.
>
> Confirm the exact header string once the first entity is generated — SeaORM 2.0 may word it
> differently from 1.x.

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

## 7. SOLID, in this codebase

Stated as what to do here, not as five words to agree with. Every one of these is a rule you
can be shown to have broken.

**S — one reason to change.** The file split *is* the principle: `commands.rs` writes,
`queries.rs` reads, `repo.rs` holds SQL, `rest.rs` translates HTTP. A file doing two of those
jobs is split, not excused. A command that both posts a purchase and emails the supplier has
two reasons to change — the email belongs in `jobs`.

**O — extend by adding, not by editing.** Adding a payment method must not mean editing the
sale command. Adding a report must not mean editing an existing one. When a change means
inserting a branch into a `match` in a function you did not come to change, stop: the thing
you are adding wants its own function, or its own variant handled where variants are handled.

> The honest exception: in Rust an exhaustive `match` is a *feature*. Adding an enum variant
> and letting the compiler list every place that must handle it is better than a trait object
> that hides them. Open/closed means do not edit *logic* to add a case — it does not mean
> avoid enums.

**L — an implementation honours its trait's contract.** A `Clock` that returns a time going
backwards, a `FileStorage::put` that silently drops the file, a `MailSender` that panics where
the trait returns `Result` — each is a substitution failure, and each will be found in
production rather than in a test. If a trait method can fail, say so in the signature; never
`unwrap()` inside an implementation to satisfy a return type.

**I — many narrow traits, not one wide one.** Do not write a `Repository` trait with twenty
methods so that four callers can use three of them. A function should take the smallest bound
that does its job: `fn f(clock: &impl Clock)`, not `fn f(services: &AppState)`. Passing the
whole state is how a function acquires dependencies nobody can see at the call site.

**D — modules depend on `core` traits, never on `infra` types.** Only `server` and `jobs` may
name a vendor SDK. A module crate that imports `lettre` or an S3 client has inverted the
dependency the wrong way, and Cargo will show it in that crate's `Cargo.toml`.

### DRY, with the caveat

Three similar-looking report queries are not duplication. **Duplication is a repeated
*decision*, not a repeated shape.** A VAT rate written in two files is duplication; two
`GROUP BY` queries that happen to resemble each other are two queries. Deduplicating the
second kind produces a function with five boolean parameters, which is worse than both.

The test: if the requirement changed, would you have to edit both places *for the same
reason*? Only then is it duplication.

### Assertable

```bash
# D — no module crate names a vendor SDK
VERIFY: test -z "$(grep -lsE '^(lettre|aws-sdk|rusoto)' crates/{catalog,sales,inventory,customers}/Cargo.toml)"
# S — the HTTP layer holds no SQL
VERIFY: test -z "$(grep -rlsE '(sea_orm|sqlx)::' crates/*/src/rest.rs)"
# S — queries do not write
VERIFY: test -z "$(grep -rlsiE '\b(INSERT|UPDATE|DELETE)\b' crates/*/src/queries.rs)"
```

All three use `test -z "$(grep -l …)"` rather than `! grep -q …`. The reason is the empty
case: when the files do not exist yet, `grep` exits 2, and `! grep` turns that failure into a
**pass**. An assertion that passes because its subject is missing is worse than no assertion.
`-s` silences the error and `-l` produces an empty list, so the empty case passes honestly.
