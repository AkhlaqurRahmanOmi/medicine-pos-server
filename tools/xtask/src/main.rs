//! `cargo xtask` — repository automation.
//!
//! The spec-driven development verifier (constitution Article II). Executes every `VERIFY:`
//! assertion attached to a ticked task and reports which completion claims are true.
//!
//! Never call `verify` (without `--lint`) from `build.rs` or from a `VERIFY:` line: many
//! assertions run `cargo test`, which re-runs `build.rs`, which would re-enter the runner
//! without terminating.

use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use sdd_lint::{citation_drift, find_task_files, gate0, lint, parse_tasks, Task};

const TIMEOUT: Duration = Duration::from_secs(180);

struct Opts {
    module: Option<String>,
    all: bool,
    strict: bool,
    lint_only: bool,
    /// Agent hook mode: structural only, and exit 2 rather than 1 so Claude Code blocks the
    /// edit and feeds the reason back, instead of merely reporting a failure.
    hook: bool,
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let root = repo_root();

    match args.first().map(String::as_str) {
        Some("verify") => {
            let opts = parse_opts(&args[1..]);
            let code = verify(&root, &opts);
            // Exit 2 in hook mode so the agent is blocked and told why, rather than the
            // failure merely being printed after the edit has already landed.
            std::process::exit(if opts.hook && code != 0 { 2 } else { code });
        }
        Some("d") | Some("decision") => {
            std::process::exit(decision(&root, &args[1..]));
        }
        Some("n") | Some("notion") => {
            std::process::exit(notion(&root, &args[1..]));
        }
        Some("status") => {
            std::process::exit(status(&root));
        }
        Some("sync-decisions") => {
            let Some(src) = args.get(1) else {
                eprintln!("usage: cargo xtask sync-decisions <path-to-PRODUCT_SPEC.md>");
                std::process::exit(2);
            };
            std::process::exit(sync_decisions(&root, Path::new(src)));
        }
        Some("new") => {
            let Some(name) = args.get(1) else {
                eprintln!("usage: cargo xtask new <NN-module-name>");
                std::process::exit(2);
            };
            std::process::exit(new_module(&root, name));
        }
        _ => {
            eprintln!(
                "usage:\n  \
                 cargo xtask verify [--module NN] [--all] [--strict] [--lint]\n  \
                 cargo xtask d <number|text>        look up a decision (~40 tokens, not 33k)\n  \
                 cargo xtask n [topic]              the Notion page that holds it, name + id\n  \
                 cargo xtask status                 module phases, one line each\n  \
                 cargo xtask new <NN-module-name>   scaffold a spec directory\n  \
                 cargo xtask sync-decisions <path>  refresh specs/DECISIONS.tsv"
            );
            std::process::exit(2);
        }
    }
}

fn parse_opts(args: &[String]) -> Opts {
    let mut opts = Opts {
        module: None,
        all: false,
        strict: false,
        lint_only: false,
        hook: false,
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--module" | "-m" => {
                opts.module = args.get(i + 1).cloned();
                i += 1;
            }
            "--all" => opts.all = true,
            "--strict" => opts.strict = true,
            "--lint" => opts.lint_only = true,
            "--hook" => {
                opts.hook = true;
                opts.lint_only = true;
            }
            other => eprintln!("{} unknown option {other}", paint("warning:", YELLOW)),
        }
        i += 1;
    }
    opts
}

/// Walk up from the manifest directory to the workspace root.
fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.ancestors()
        .find(|p| p.join("specs").join("CONSTITUTION.md").exists())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn verify(root: &Path, opts: &Opts) -> i32 {
    // Gate 0 runs first, always, before any assertion is considered.
    let gate0_violations = gate0(root);
    if !gate0_violations.is_empty() {
        println!(
            "{} Gate 0 — implementation without a spec\n",
            paint("FAIL", RED)
        );
        for v in &gate0_violations {
            println!("  {v}");
        }
        println!();
        return 1;
    }

    // Citation drift: an `Art. V.6` that no longer resolves. Cheap, parses only.
    let drift = citation_drift(root);
    if !drift.is_empty() {
        if opts.hook {
            eprintln!("Blocked — a citation of specs/CONSTITUTION.md no longer resolves.\n");
            for v in &drift {
                eprintln!("  {v}");
            }
            eprintln!("\nFix the citation, or renumber the article back.");
        } else {
            println!(
                "{} citation drift — a reference to the core does not resolve\n",
                paint("FAIL", RED)
            );
            for v in &drift {
                println!("  {v}");
            }
            println!();
        }
        return 1;
    }

    let structural = lint(root);
    if !structural.is_empty() {
        // In hook mode the agent reads stderr, so the reason reaches it with the block.
        if opts.hook {
            eprintln!("Blocked by specs/CONSTITUTION.md Article II — a task was ticked with no VERIFY: line.\n");
            for v in &structural {
                eprintln!("  {v}");
            }
            eprintln!("\nAdd an assertion that exits 0 when the task is genuinely done, or un-tick the box.");
        } else {
            println!("{} ticked without an assertion\n", paint("FAIL", RED));
            for v in &structural {
                println!("  {v}");
            }
            println!();
        }
        if opts.lint_only || opts.strict {
            return 1;
        }
    }

    if opts.lint_only {
        if !opts.hook {
            println!(
                "{} structure clean — every ticked task carries a VERIFY: line, every citation resolves",
                paint("OK", GREEN)
            );
        }
        return 0;
    }

    let files = task_files_for(root, opts.module.as_deref());
    if files.is_empty() {
        match &opts.module {
            Some(m) => println!(
                "{} no tasks.md found for module {m}",
                paint("warning:", YELLOW)
            ),
            None => println!(
                "{} no tasks.md found under specs/ — nothing to verify yet",
                paint("note:", DIM)
            ),
        }
        return 0;
    }

    let started = Instant::now();
    let (mut pass, mut fail, mut skipped, mut probe_pass, mut probe_fail) = (0, 0, 0, 0, 0);
    let mut failures: Vec<String> = Vec::new();

    for file in &files {
        let Ok(tasks) = parse_tasks(file) else {
            continue;
        };
        let leaves: Vec<&Task> = tasks.iter().filter(|t| t.leaf).collect();
        if leaves.is_empty() {
            continue;
        }

        println!("\n{}", paint(&rel(root, file), BOLD));

        for task in leaves {
            let title = short(&task.title);
            match (&task.verify, task.ticked) {
                (Some(cmd), true) => {
                    let outcome = run(cmd, root);
                    if outcome.ok {
                        pass += 1;
                        println!("  {} {title}", paint("pass", GREEN));
                    } else {
                        fail += 1;
                        println!("  {} {title}", paint("FAIL", RED));
                        println!("       {}", paint(cmd, DIM));
                        println!("       {}", paint(&outcome.reason, DIM));
                        failures.push(format!("{}:{} {title}", rel(root, file), task.line));
                    }
                }
                (None, true) => {
                    skipped += 1;
                    println!("  {} {title} (no VERIFY: line)", paint("????", YELLOW));
                }
                (Some(cmd), false) if opts.all => {
                    if run(cmd, root).ok {
                        probe_pass += 1;
                        println!(
                            "  {} {title} — unticked, but its assertion holds",
                            paint("ready", BLUE)
                        );
                    } else {
                        probe_fail += 1;
                        println!("  {} {title}", paint("todo", DIM));
                    }
                }
                _ => {}
            }
        }
    }

    println!("\n{}", "─".repeat(64));
    let mut parts = vec![
        format!("{} passed", paint(&pass.to_string(), GREEN)),
        format!(
            "{} failed",
            paint(&fail.to_string(), if fail > 0 { RED } else { DIM })
        ),
    ];
    if skipped > 0 {
        parts.push(format!(
            "{} unverifiable",
            paint(&skipped.to_string(), YELLOW)
        ));
    }
    if opts.all {
        parts.push(format!("{probe_pass} ready to tick, {probe_fail} not yet"));
    }
    println!(
        "{}  in {:.1}s",
        parts.join(" · "),
        started.elapsed().as_secs_f64()
    );

    if !failures.is_empty() {
        println!(
            "\n{} these completion claims are not true:",
            paint("Untrue:", RED)
        );
        for f in &failures {
            println!("  {f}");
        }
        println!("\nFix the code, or un-tick the box. Record it in specs/INDEX.md until resolved.");
    }

    let strict_failure = opts.strict && skipped > 0;
    i32::from(fail > 0 || strict_failure)
}

fn task_files_for(root: &Path, module: Option<&str>) -> Vec<PathBuf> {
    let specs = root.join("specs");
    let all = find_task_files(&specs);
    let Some(m) = module else { return all };

    // Accept "07", "07-sales" or "sales".
    all.into_iter()
        .filter(|p| {
            p.parent()
                .and_then(|d| d.file_name())
                .and_then(|n| n.to_str())
                .is_some_and(|dir| {
                    dir == m || dir.starts_with(&format!("{m}-")) || dir.ends_with(&format!("-{m}"))
                })
        })
        .collect()
}

struct Outcome {
    ok: bool,
    reason: String,
}

/// Resolve the shell that runs assertions.
///
/// On Windows this must be an explicit path. `CreateProcess` searches `System32` *before*
/// `PATH`, and `System32\bash.exe` is the WSL launcher — so spawning the bare name `bash`
/// finds WSL rather than Git Bash, even when Git Bash is first on `PATH`. Every assertion
/// then fails with a WSL relay error that looks nothing like the real cause.
///
/// Override with `SDD_BASH` if Git is installed somewhere unusual.
fn bash() -> PathBuf {
    static CACHED: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    CACHED
        .get_or_init(|| {
            if let Some(explicit) = env::var_os("SDD_BASH") {
                return PathBuf::from(explicit);
            }
            if !cfg!(windows) {
                return PathBuf::from("bash");
            }

            let roots = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
                .iter()
                .filter_map(env::var_os)
                .map(PathBuf::from);

            for root in roots {
                for suffix in [
                    "Git/bin/bash.exe",
                    "Git/usr/bin/bash.exe",
                    "Programs/Git/bin/bash.exe",
                ] {
                    let candidate = root.join(suffix);
                    if candidate.is_file() {
                        return candidate;
                    }
                }
            }

            // Last resort: walk PATH ourselves, skipping the WSL launcher locations.
            if let Some(path) = env::var_os("PATH") {
                for dir in env::split_paths(&path) {
                    let lower = dir.to_string_lossy().to_lowercase();
                    if lower.contains("system32") || lower.contains("windowsapps") {
                        continue;
                    }
                    let candidate = dir.join("bash.exe");
                    if candidate.is_file() {
                        return candidate;
                    }
                }
            }

            PathBuf::from("bash")
        })
        .clone()
}

/// Run one assertion through Git Bash from the repo root, with a hard timeout.
///
/// Output pipes are drained on their own threads, so a chatty command cannot deadlock the
/// poll loop by filling the pipe buffer.
fn run(cmd: &str, root: &Path) -> Outcome {
    let spawned = Command::new(bash())
        .arg("-c")
        .arg(cmd)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            return Outcome {
                ok: false,
                reason: format!(
                    "could not start {} ({e}) — install Git Bash, or set SDD_BASH to its path",
                    bash().display()
                ),
            }
        }
    };

    let (tx, rx) = mpsc::channel::<String>();

    // Drain both pipes on their own threads.
    let mut handles = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        let tx = tx.clone();
        handles.push(thread::spawn(move || {
            let mut s = String::new();
            let _ = out.read_to_string(&mut s);
            let _ = tx.send(s);
        }));
    }
    if let Some(mut err) = child.stderr.take() {
        let tx = tx.clone();
        handles.push(thread::spawn(move || {
            let mut s = String::new();
            let _ = err.read_to_string(&mut s);
            let _ = tx.send(s);
        }));
    }
    drop(tx);

    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                thread::sleep(Duration::from_millis(40));
            }
            Err(e) => {
                return Outcome {
                    ok: false,
                    reason: format!("wait failed: {e}"),
                };
            }
        }
    };

    for h in handles {
        let _ = h.join();
    }
    let output: String = rx.iter().collect();

    match status {
        None => Outcome {
            ok: false,
            reason: format!("timed out after {}s", TIMEOUT.as_secs()),
        },
        Some(s) if s.success() => Outcome {
            ok: true,
            reason: String::new(),
        },
        Some(s) => {
            let code = s
                .code()
                .map_or_else(|| "signal".to_string(), |c| format!("exit {c}"));
            let tail = last_line(&output);
            Outcome {
                ok: false,
                reason: if tail.is_empty() {
                    code
                } else {
                    format!("{code} — {tail}")
                },
            }
        }
    }
}

/// Print one decision, or the ones matching a search term.
///
/// Exists so an agent never opens `PRODUCT_SPEC.md` (~33,000 tokens) or fetches the Notion
/// page to check a single line. One decision costs about 40 tokens this way.
fn decision(root: &Path, args: &[String]) -> i32 {
    let path = root.join("specs").join("DECISIONS.tsv");
    let Ok(text) = fs::read_to_string(&path) else {
        eprintln!("{} {} not found", paint("error:", RED), path.display());
        return 1;
    };

    let rows: Vec<(&str, &str, &str)> = text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let mut f = l.splitn(3, '\t');
            Some((f.next()?, f.next()?, f.next()?))
        })
        .collect();

    let Some(query) = args.first() else {
        println!("{} decisions. Give a number or a search term.", rows.len());
        return 0;
    };

    // A bare number is an exact lookup; anything else is a case-insensitive search.
    let matches: Vec<_> = if query.chars().all(|c| c.is_ascii_digit()) {
        rows.iter().filter(|(n, _, _)| n == query).collect()
    } else {
        let needle = query.to_lowercase();
        rows.iter()
            .filter(|(_, t, d)| {
                t.to_lowercase().contains(&needle) || d.to_lowercase().contains(&needle)
            })
            .collect()
    };

    if matches.is_empty() {
        println!("No decision matches \"{query}\".");
        return 1;
    }
    for (n, topic, body) in matches {
        println!(
            "{} {}\n   {body}",
            paint(&format!("[D-{n}]"), BOLD),
            paint(topic, DIM)
        );
    }
    0
}

/// One Notion page, by topic. Constitution Article III.
///
/// The page ids used to live in the constitution as a ten-row table, which every session paid
/// for whether or not it fetched anything. This is the same trade as `d`: a lookup costs ~20
/// tokens, the table cost ~280 per session.
fn notion(root: &Path, args: &[String]) -> i32 {
    let path = root.join("specs").join("NOTION.map");
    let Ok(text) = fs::read_to_string(&path) else {
        eprintln!("{} {} not found", paint("error:", RED), path.display());
        return 1;
    };

    let rows: Vec<(&str, &str, &str, &str)> = text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let mut f = l.splitn(4, '\t');
            Some((f.next()?, f.next()?, f.next()?, f.next()?))
        })
        .collect();

    let show = |(status, page, id, holds): &(&str, &str, &str, &str)| {
        if *status == "written" {
            println!("{}  {}\n   {}", paint(page, BOLD), id, paint(holds, DIM));
        } else {
            println!(
                "{}  {}",
                paint(page, BOLD),
                paint("NOT WRITTEN — ask, do not infer", YELLOW)
            );
        }
    };

    let Some(query) = args.first() else {
        for row in rows.iter().filter(|r| r.0 == "written") {
            show(row);
        }
        let unwritten = rows.iter().filter(|r| r.0 != "written").count();
        println!(
            "\n{}",
            paint(
                &format!("{unwritten} more documents in the Notion index are not written yet — `cargo xtask n <topic>` names them."),
                DIM
            )
        );
        return 0;
    };

    // `data-model`, `data_model` and `data model` are the same query — an agent types
    // whichever it is holding, and a near miss costs a page fetch.
    let flatten = |s: &str| s.to_lowercase().replace(['-', '_'], " ");
    let needle = flatten(query);
    let matches: Vec<_> = rows
        .iter()
        .filter(|(_, page, _, holds)| {
            flatten(page).contains(&needle) || flatten(holds).contains(&needle)
        })
        .collect();

    if matches.is_empty() {
        println!("No Notion page matches \"{query}\". `cargo xtask n` lists them all.");
        return 1;
    }
    for row in matches {
        show(row);
    }
    0
}

/// One line per module: phase, tasks done, and whether its spec exists.
///
/// Cheaper than reading INDEX.md, and it reports what is true on disk rather than what the
/// index claims — the two drift, and the disk is right.
fn status(root: &Path) -> i32 {
    let specs = root.join("specs");
    let Ok(map) = fs::read_to_string(specs.join("MODULES.map")) else {
        eprintln!("{} specs/MODULES.map not found", paint("error:", RED));
        return 1;
    };

    for line in map.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((module, crates)) = line.split_once('=') else {
            continue;
        };
        let module = module.trim();
        let dir = specs.join(module);

        // `new` scaffolds all four files, so existence alone would report every fresh
        // module as Gate 3. A file still carrying the template's placeholder heading has
        // not been written.
        let gate = |f: &str| is_filled(&dir.join(f));
        let phase = match (gate("SPEC.md"), gate("plan.md"), gate("tasks.md")) {
            (false, _, _) => "0 no spec",
            (true, false, _) => "1 spec",
            (true, true, false) => "2 plan",
            (true, true, true) => "3 tasks",
        };

        let (done, total) = match parse_tasks(&dir.join("tasks.md")) {
            Ok(tasks) if gate("tasks.md") => {
                let leaves: Vec<_> = tasks.iter().filter(|t| t.leaf).collect();
                (leaves.iter().filter(|t| t.ticked).count(), leaves.len())
            }
            _ => (0, 0),
        };

        let built = crates
            .split_whitespace()
            .filter(|c| root.join("crates").join(c).exists())
            .count();

        println!(
            "{:<14} {:<10} {:>3}/{:<3} {} crate(s) built of {}",
            module,
            phase,
            done,
            total,
            built,
            crates.split_whitespace().count()
        );
    }
    0
}

/// A spec artifact counts as written only once the template's placeholder heading is gone.
///
/// Every template's H1 carries `[Module Title]`. Scaffolding creates all four files at once,
/// so without this check a module would report Gate 3 the moment it was created.
fn is_filled(path: &Path) -> bool {
    match fs::read_to_string(path) {
        Ok(text) => !text.contains("[Module Title]"),
        Err(_) => false,
    }
}

/// Regenerate `specs/DECISIONS.tsv` from the decisions table in `PRODUCT_SPEC.md`.
fn sync_decisions(root: &Path, source: &Path) -> i32 {
    let Ok(text) = fs::read_to_string(source) else {
        eprintln!("{} cannot read {}", paint("error:", RED), source.display());
        return 1;
    };

    let mut rows = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if !t.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = t.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() < 3 || !cells[0].chars().all(|c| c.is_ascii_digit()) || cells[0].is_empty() {
            continue;
        }
        rows.push(format!("{}\t{}\t{}", cells[0], cells[1], cells[2]));
    }

    if rows.is_empty() {
        eprintln!(
            "{} no decision rows found in {}",
            paint("error:", RED),
            source.display()
        );
        return 1;
    }

    let path = root.join("specs").join("DECISIONS.tsv");
    let header = match fs::read_to_string(&path) {
        Ok(existing) => existing
            .lines()
            .take_while(|l| l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n"),
        Err(_) => "# Decisions log — local index. Notion is canonical.".into(),
    };

    match fs::write(&path, format!("{header}\n{}\n", rows.join("\n"))) {
        Ok(()) => {
            println!(
                "{} {} decisions written to specs/DECISIONS.tsv",
                paint("OK", GREEN),
                rows.len()
            );
            0
        }
        Err(e) => {
            eprintln!("{} {e}", paint("error:", RED));
            1
        }
    }
}

fn new_module(root: &Path, name: &str) -> i32 {
    let dir = root.join("specs").join(name);
    if dir.exists() {
        eprintln!("{} specs/{name} already exists", paint("error:", RED));
        return 1;
    }
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("{} {e}", paint("error:", RED));
        return 1;
    }

    let templates = root.join(".specify").join("templates");
    for (template, target) in [
        ("spec-template.md", "SPEC.md"),
        ("contract-template.md", "CONTRACT.md"),
        ("plan-template.md", "plan.md"),
        ("tasks-template.md", "tasks.md"),
    ] {
        match fs::read_to_string(templates.join(template)) {
            Ok(body) => {
                if let Err(e) = fs::write(dir.join(target), body) {
                    eprintln!("{} writing {target}: {e}", paint("error:", RED));
                    return 1;
                }
            }
            Err(e) => {
                eprintln!("{} reading {template}: {e}", paint("error:", RED));
                return 1;
            }
        }
    }

    println!(
        "{} specs/{name}/ created from the templates",
        paint("OK", GREEN)
    );
    println!("\nNext:");
    println!("  1. map its crate(s) in specs/MODULES.map");
    println!("  2. add its row to specs/INDEX.md");
    println!("  3. fill SPEC.md and CONTRACT.md — Gate 1");
    0
}

// ---- presentation ----

const RED: &str = "31";
const GREEN: &str = "32";
const YELLOW: &str = "33";
const BLUE: &str = "34";
const DIM: &str = "2";
const BOLD: &str = "1";

fn paint(s: &str, code: &str) -> String {
    if env::var_os("NO_COLOR").is_some() {
        s.to_string()
    } else {
        format!("\x1b[{code}m{s}\x1b[0m")
    }
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .display()
        .to_string()
        .replace('\\', "/")
}

fn short(title: &str) -> String {
    let plain = title.replace("**", "");
    if plain.chars().count() <= 68 {
        plain
    } else {
        format!("{}…", plain.chars().take(68).collect::<String>())
    }
}

fn last_line(output: &str) -> String {
    output
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("")
        .chars()
        .take(120)
        .collect()
}
