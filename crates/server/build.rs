//! The agent-neutral enforcement floor (constitution Article II).
//!
//! Cargo has no `prebuild` step, so the gate lives here instead: every `cargo build`,
//! `cargo check` and `cargo test` that touches the binary runs this first. It binds every
//! agent and every human, with no editor integration, no hook system and no git.
//!
//! This is **structural only** — it parses files and executes nothing. Running assertions
//! from here would re-enter the build without terminating, because many of them invoke
//! `cargo test`. See the constitution, Article II, "Two constraints on enforcement".

use std::path::{Path, PathBuf};

fn main() {
    let Some(root) = workspace_root() else {
        println!("cargo::warning=SDD gate skipped: workspace root not found");
        return;
    };

    // Re-run when a spec changes, not on every unrelated edit.
    println!("cargo::rerun-if-changed={}", root.join("specs").display());
    println!("cargo::rerun-if-changed=build.rs");

    let mut failed = false;

    for v in sdd_lint::gate0(&root) {
        println!("cargo::error=Gate 0 — {v}");
        failed = true;
    }

    for v in sdd_lint::lint(&root) {
        println!("cargo::error=Article II — {v}");
        failed = true;
    }

    if failed {
        println!(
            "cargo::error=Specs are upstream of code. Add the VERIFY: line, or un-tick the task. See specs/CONSTITUTION.md."
        );
        std::process::exit(1);
    }
}

fn workspace_root() -> Option<PathBuf> {
    let here = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").ok()?);
    here.ancestors()
        .find(|p| p.join("specs").join("CONSTITUTION.md").exists())
        .map(Path::to_path_buf)
}
