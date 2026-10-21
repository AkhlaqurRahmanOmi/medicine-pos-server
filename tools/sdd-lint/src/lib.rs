//! Structural parsing and linting for spec-driven development.
//!
//! Deliberately dependency-free: this crate is a build-dependency of `server`, so every
//! dependency added here is paid on every clean build of the binary.
//!
//! Nothing in this crate executes a command. Running assertions is `xtask`'s job, and the
//! separation is what makes it safe to call this from `build.rs` — see
//! `specs/rules/enforcement.md`, "Two constraints".

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
        None => (
            true,
            rest.strip_prefix("[x]")
                .or_else(|| rest.strip_prefix("[X]"))?,
        ),
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
        for task in tasks
            .iter()
            .filter(|t| t.leaf && t.ticked && t.verify.is_none())
        {
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
        let first = line
            .trim_matches('|')
            .split('|')
            .next()
            .unwrap_or("")
            .trim();
        if let Some(name) = first.strip_prefix('`').and_then(|s| s.strip_suffix('`')) {
            names.push(name.to_string());
        }
    }
    names
}

// ---------------------------------------------------------------------------- citations
//
// The entry points and rule files cite the core by number — `Art. II.2`, `Article V.6`.
// Renumbering an article silently invalidates every one of them, and a stale citation reads
// as authoritative. This check resolves each one against the constitution as it is now.

/// One article citation found in a file: `Article V`, `Art. V.6`, `Art. V.6–7`.
#[derive(Debug, PartialEq, Eq)]
pub struct Citation {
    pub label: String,
    /// The numbered items referenced, if any. A range cites both of its ends.
    pub items: Vec<u32>,
}

/// The articles the constitution has, each with its count of top-level numbered items.
///
/// Fenced blocks are skipped: a `1.` inside an example is not a rule.
pub fn constitution_articles(text: &str) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = Vec::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        if let Some(rest) = line.strip_prefix("## Article ") {
            let label = rest.split(':').next().unwrap_or("").trim().to_string();
            if !label.is_empty() {
                out.push((label, 0));
            }
        } else if let Some(n) = numbered_item(line) {
            if let Some(last) = out.last_mut() {
                last.1 = last.1.max(n as usize);
            }
        }
    }
    out
}

/// `3. ` at column zero — a top-level numbered rule, not a nested or indented one.
fn numbered_item(line: &str) -> Option<u32> {
    let digits: String = line.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() || !line[digits.len()..].starts_with(". ") {
        return None;
    }
    digits.parse().ok()
}

/// `Article`, `Articles` or `Art.` followed by a space, not inside a longer word.
fn keyword_at(chars: &[char], i: usize) -> Option<usize> {
    if i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
        return None;
    }
    for kw in ["Articles ", "Article ", "Art. "] {
        let k: Vec<char> = kw.chars().collect();
        if chars[i..].starts_with(&k) {
            return Some(k.len());
        }
    }
    None
}

/// A label (`V`, `III`, `0`) with optional `.6` and optional `–7` range end.
fn citation_at(chars: &[char], start: usize) -> Option<(Citation, usize)> {
    let mut i = start;
    let label: String = chars[i..]
        .iter()
        .take_while(|c| matches!(c, 'I' | 'V' | 'X') || c.is_ascii_digit())
        .collect();
    if label.is_empty() {
        return None;
    }
    let roman = label.chars().all(|c| matches!(c, 'I' | 'V' | 'X'));
    if !roman && label.chars().any(|c| !c.is_ascii_digit()) {
        return None;
    }
    i += label.chars().count();

    let mut items = Vec::new();
    if chars.get(i) == Some(&'.') && chars.get(i + 1).is_some_and(char::is_ascii_digit) {
        i += 1;
        let first: String = chars[i..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        i += first.chars().count();
        if let Ok(n) = first.parse() {
            items.push(n);
        }
        // `V.6–7`, `V.6-7`: both ends must resolve.
        if matches!(chars.get(i), Some('–' | '-' | '—'))
            && chars.get(i + 1).is_some_and(char::is_ascii_digit)
        {
            i += 1;
            let second: String = chars[i..]
                .iter()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            i += second.chars().count();
            if let Ok(n) = second.parse() {
                items.push(n);
            }
        }
    }
    Some((Citation { label, items }, i))
}

/// `Articles III and VI`, `Art. II.2, V.6` — one keyword, several citations.
fn list_separator_at(chars: &[char], i: usize) -> Option<usize> {
    for sep in [" and ", ", "] {
        let s: Vec<char> = sep.chars().collect();
        if chars[i..].starts_with(&s) {
            return Some(i + s.len());
        }
    }
    None
}

/// Every article citation on one line.
pub fn citations_in_line(line: &str) -> Vec<Citation> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let Some(len) = keyword_at(&chars, i) else {
            i += 1;
            continue;
        };
        let mut j = i + len;
        while let Some((cit, next)) = citation_at(&chars, j) {
            out.push(cit);
            j = next;
            match list_separator_at(&chars, j) {
                Some(k) => j = k,
                None => break,
            }
        }
        i = j.max(i + 1);
    }
    out
}

/// Constitution Article VI: a citation that no longer resolves is a rule nobody can read.
///
/// Executes nothing. Safe to call from `build.rs`.
pub fn citation_drift(root: &Path) -> Vec<Violation> {
    let constitution = root.join("specs").join("CONSTITUTION.md");
    let Ok(text) = fs::read_to_string(&constitution) else {
        return vec![Violation {
            file: constitution,
            line: 0,
            message: "the constitution is missing — every citation is unresolvable".into(),
        }];
    };
    let articles = constitution_articles(&text);

    let mut out = Vec::new();
    for file in citing_files(root) {
        let Ok(body) = fs::read_to_string(&file) else {
            continue;
        };
        for (n, line) in body.lines().enumerate() {
            for cit in citations_in_line(line) {
                let Some((_, items)) = articles.iter().find(|(l, _)| *l == cit.label) else {
                    out.push(Violation {
                        file: file.clone(),
                        line: n + 1,
                        message: format!(
                            "cites Article {} — the constitution has no such article",
                            cit.label
                        ),
                    });
                    continue;
                };
                for item in &cit.items {
                    if *item as usize > *items {
                        out.push(Violation {
                            file: file.clone(),
                            line: n + 1,
                            message: format!(
                                "cites Article {}.{item}, but Article {} has {items} numbered items",
                                cit.label, cit.label
                            ),
                        });
                    }
                }
            }
        }
    }
    out
}

/// Files that may cite the core: rules, entry points, hooks and source comments.
///
/// `docs/` is excluded — it is a product export, not a rulebook, and "Article" there means
/// something else entirely.
fn citing_files(root: &Path) -> Vec<PathBuf> {
    const SKIP: &[&str] = &["target", ".git", "docs", "node_modules"];
    let mut out = Vec::new();
    fn rec(dir: &Path, out: &mut Vec<PathBuf>, skip: &[&str]) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if !skip.contains(&name.as_str()) {
                    rec(&path, out, skip);
                }
            } else {
                let citing_ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| matches!(e, "md" | "mdc" | "rs"));
                let in_hooks = dir.file_name().is_some_and(|d| d == ".githooks");
                if citing_ext || in_hooks {
                    out.push(path);
                }
            }
        }
    }
    rec(root, &mut out, SKIP);
    out.sort();
    out
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
        assert_eq!(
            parse_task_line("- [ ] do a thing"),
            Some((false, "do a thing".into()))
        );
        assert_eq!(parse_task_line("  - [x] done"), Some((true, "done".into())));
        assert_eq!(parse_task_line("  - [X] done"), Some((true, "done".into())));
        assert_eq!(parse_task_line("not a task"), None);
        assert_eq!(parse_task_line("- [ ]"), None);
    }

    #[test]
    fn parses_verify_lines() {
        assert_eq!(
            parse_verify_line("      VERIFY: cargo test -p x"),
            Some("cargo test -p x".into())
        );
        assert_eq!(parse_verify_line("VERIFY:   "), None);
        assert_eq!(parse_verify_line("| `VERIFY: x` | no |"), None);
    }

    #[test]
    fn reads_articles_and_their_item_counts() {
        let text = "## Article II: Proof\n\n1. one\n2. two\n\n```markdown\n1. not a rule\n2. nor this\n3. nor this\n```\n\n## Article VI: Loading\n\nno items here\n";
        let got = constitution_articles(text);
        assert_eq!(got, vec![("II".to_string(), 2), ("VI".to_string(), 0)]);
    }

    #[test]
    fn finds_citations_in_their_written_forms() {
        let one = |s: &str| citations_in_line(s);
        assert_eq!(
            one("see Article V"),
            vec![Citation {
                label: "V".into(),
                items: vec![]
            }]
        );
        assert_eq!(
            one("Art. II.2 — a separate agent"),
            vec![Citation {
                label: "II".into(),
                items: vec![2]
            }]
        );
        assert_eq!(
            one("blocked by Article V.6–7, mechanically"),
            vec![Citation {
                label: "V".into(),
                items: vec![6, 7]
            }]
        );
        assert_eq!(
            one("Referenced from Articles III and VI."),
            vec![
                Citation {
                    label: "III".into(),
                    items: vec![]
                },
                Citation {
                    label: "VI".into(),
                    items: vec![]
                }
            ]
        );
        // Not citations: a word that merely starts the same way, and prose about articles.
        assert!(one("Articles of association").is_empty());
        assert!(one("the Articled clerk").is_empty());
    }

    #[test]
    fn indent_counts_tabs_as_four() {
        assert_eq!(indent_of("    - [ ] x"), 4);
        assert_eq!(indent_of("\t- [ ] x"), 4);
    }
}
