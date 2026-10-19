//! Structural parsing and linting for spec-driven development.
//!
//! Deliberately dependency-free: this crate is a build-dependency of `server`, so every
//! dependency added here is paid on every clean build of the binary.
//!
//! Nothing in this crate executes a command. Running assertions is `xtask`'s job, and the
//! separation is what makes it safe to call this from `build.rs` — see constitution
//! Article II, "Two constraints on enforcement".

use std::fs;
use std::path::{Path, PathBuf};

/// Crates that never need a spec, because they are composition rather than domain.
pub const EXEMPT_CRATES: &[&str] = &["server", "migration"];

#[derive(Debug, Clone)]
pub struct Task {
    pub file: PathBuf,
    pub line: usize,
    pub indent: usize,
    pub ticked: bool,
    pub title: String,
    pub verify: Option<String>,
    /// A task with no nested children. Only leaves carry assertions.
    pub leaf: bool,
}

#[derive(Debug, Clone)]
pub struct Violation {
    pub file: PathBuf,
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line > 0 {
            write!(f, "{}:{}: {}", self.file.display(), self.line, self.message)
        } else {
            write!(f, "{}: {}", self.file.display(), self.message)
        }
    }
}

/// Width of the leading whitespace, counting a tab as four columns.
fn indent_of(line: &str) -> usize {
    let mut n = 0;
    for c in line.chars() {
        match c {
            ' ' => n += 1,
            '\t' => n += 4,
            _ => break,
        }
    }
    n
}

/// Matches `- [ ] title` / `- [x] title`, returning (ticked, title).
fn parse_task_line(line: &str) -> Option<(bool, String)> {
    let t = line.trim_start();
    let rest = t.strip_prefix("- ")?;
    let rest = rest.trim_start();
    let (mark, rest) = match rest.strip_prefix("[ ]") {
        Some(r) => (false, r),
        None => (true, rest.strip_prefix("[x]").or_else(|| rest.strip_prefix("[X]"))?),
    };
    let title = rest.trim();
    if title.is_empty() {
        return None;
    }
    Some((mark, title.to_string()))
}

fn parse_verify_line(line: &str) -> Option<String> {
    let t = line.trim_start();
    let rest = t.strip_prefix("VERIFY:")?;
    let cmd = rest.trim();
    if cmd.is_empty() {
        None
    } else {
        Some(cmd.to_string())
    }
}

/// Parse one `tasks.md` into its task list, with `VERIFY:` lines attached and leaf status set.
pub fn parse_tasks(path: &Path) -> std::io::Result<Vec<Task>> {
    let text = fs::read_to_string(path)?;
    let lines: Vec<&str> = text.lines().collect();
    let mut tasks: Vec<Task> = Vec::new();

    for (i, raw) in lines.iter().enumerate() {
        let Some((ticked, title)) = parse_task_line(raw) else {
            continue;
        };

        // Scan forward for this task's VERIFY line. Stop at the next task or a heading,
        // so an assertion can never be attached to the wrong task.
        let mut verify = None;
        for next in lines.iter().skip(i + 1) {
            if parse_task_line(next).is_some() || next.trim_start().starts_with('#') {
                break;
            }
            if let Some(cmd) = parse_verify_line(next) {
                verify = Some(cmd);
                break;
            }
        }

        tasks.push(Task {
            file: path.to_path_buf(),
            line: i + 1,
            indent: indent_of(raw),
            ticked,
            title,
            verify,
            leaf: true, // corrected below
        });
    }

    // A task is a parent exactly when the task that follows it is nested deeper.
    for i in 0..tasks.len() {
        if let Some(next) = tasks.get(i + 1) {
            if next.indent > tasks[i].indent {
                tasks[i].leaf = false;
            }
        }
    }

    Ok(tasks)
}

/// Every `specs/**/tasks.md`, sorted so output is stable between runs.
pub fn find_task_files(specs: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(specs, &mut found);
    found.sort();
    found
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.file_name().is_some_and(|n| n == "tasks.md") {
            out.push(path);
        }
    }
}

/// Constitution Article II: a ticked leaf with no `VERIFY:` line is a claim with no proof.
///
/// Executes nothing. Safe to call from `build.rs`.
pub fn lint(root: &Path) -> Vec<Violation> {
    let specs = root.join("specs");
    let mut violations = Vec::new();

    for file in find_task_files(&specs) {
        let Ok(tasks) = parse_tasks(&file) else {
            violations.push(Violation {
                file: file.clone(),
                line: 0,
                message: "could not be read".into(),
            });
            continue;
        };
        for task in tasks.iter().filter(|t| t.leaf && t.ticked && t.verify.is_none()) {
            violations.push(Violation {
                file: task.file.clone(),
                line: task.line,
                message: format!(
                    "ticked with no VERIFY: line — \"{}\". A task is ticked on a command that exits 0, never on intent.",
                    truncate(&task.title, 72)
                ),
            });
        }
    }

    violations
}

/// Constitution Article I, Gate 0: no implementation before its `SPEC.md`.
///
/// The crate-to-module mapping is `specs/MODULES.map`, because one spec can cover several
/// crates (module 00 covers `core`, `db`, `api` and `cache`).
pub fn gate0(root: &Path) -> Vec<Violation> {
    let specs = root.join("specs");
    let map_path = specs.join("MODULES.map");
    let mut violations = Vec::new();

    let Ok(map_text) = fs::read_to_string(&map_path) else {
        return vec![Violation {
            file: map_path,
            line: 0,
            message: "missing — Gate 0 cannot be checked without the crate-to-module map".into(),
        }];
    };

    // "00-foundation = core db api cache"
    let mut crate_to_module: Vec<(String, String)> = Vec::new();
    for line in map_text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((module, crates)) = line.split_once('=') else {
            continue;
        };
        let module = module.trim().to_string();
        for name in crates.split_whitespace() {
            crate_to_module.push((name.to_string(), module.clone()));
        }
    }

    let placeholders = registered_placeholders(&specs);

    let Ok(entries) = fs::read_dir(root.join("crates")) else {
        return violations;
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();

    for name in names {
        if EXEMPT_CRATES.contains(&name.as_str()) || placeholders.contains(&name) {
            continue;
        }
        let crate_dir = root.join("crates").join(&name);
        match crate_to_module.iter().find(|(c, _)| *c == name) {
            None => violations.push(Violation {
                file: crate_dir,
                line: 0,
                message: format!(
                    "crate `{name}` is not mapped to a module in specs/MODULES.map, not exempt, and not a registered placeholder"
                ),
            }),
            Some((_, module)) => {
                let spec = specs.join(module).join("SPEC.md");
                if !spec.exists() {
                    violations.push(Violation {
                        file: crate_dir,
                        line: 0,
                        message: format!(
                            "crate `{name}` exists but {} does not — Gate 0 forbids implementation before its spec",
                            spec.display()
                        ),
                    });
                }
            }
        }
    }

    violations
}

/// Crate names in the "Registered placeholders" table of `PLACEHOLDERS.md`, which is
/// everything above the "Permanently exempt" heading.
fn registered_placeholders(specs: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(specs.join("PLACEHOLDERS.md")) else {
        return Vec::new();
    };
    let head = match text.find("## Permanently exempt") {
        Some(i) => &text[..i],
        None => &text[..],
    };

    let mut names = Vec::new();
    for line in head.lines() {
        let line = line.trim();
        if !line.starts_with('|') {
            continue;
        }
        let first = line.trim_matches('|').split('|').next().unwrap_or("").trim();
        if let Some(name) = first.strip_prefix('`').and_then(|s| s.strip_suffix('`')) {
            names.push(name.to_string());
        }
    }
    names
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let kept: String = s.chars().take(max).collect();
        format!("{kept}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tick_states() {
        assert_eq!(parse_task_line("- [ ] do a thing"), Some((false, "do a thing".into())));
        assert_eq!(parse_task_line("  - [x] done"), Some((true, "done".into())));
        assert_eq!(parse_task_line("  - [X] done"), Some((true, "done".into())));
        assert_eq!(parse_task_line("not a task"), None);
        assert_eq!(parse_task_line("- [ ]"), None);
    }

    #[test]
    fn parses_verify_lines() {
        assert_eq!(parse_verify_line("      VERIFY: cargo test -p x"), Some("cargo test -p x".into()));
        assert_eq!(parse_verify_line("VERIFY:   "), None);
        assert_eq!(parse_verify_line("| `VERIFY: x` | no |"), None);
    }

    #[test]
    fn indent_counts_tabs_as_four() {
        assert_eq!(indent_of("    - [ ] x"), 4);
        assert_eq!(indent_of("\t- [ ] x"), 4);
    }
}
