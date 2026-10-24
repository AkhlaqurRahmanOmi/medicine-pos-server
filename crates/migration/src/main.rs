//! The migrator's command line.
//!
//! Constitution **Article V**: an agent writes migrations and never runs them. This binary
//! exists so a human can — `cargo run -p migration -- up`, with no global tool to install.
//!
//! It connects with `DATABASE_URL`, which is the **owner** role. The running server uses
//! `APP_DATABASE_URL` (the non-owner role) and must never be used here: migrations need to
//! own the schema, and the server must not be able to alter it.

use migration::Migrator;
use sea_orm_migration::prelude::*;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    cli::run_cli(Migrator).await;
}
