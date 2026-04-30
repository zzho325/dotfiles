//! # orch — Task orchestrator for Claude Code workers
//!
//! ## Architecture
//!
//! ```text
//! ┌──────────┐     ┌──────────────┐     ┌─────────────────┐
//! │  orch    │     │ orch daemon  │     │ orchestrator    │
//! │  (TUI)   │     │ (watcher)    │     │ (Claude agent)  │
//! └──────────┘     └──────┬───────┘     └────────┬────────┘
//!                         │                      │
//!      reads              │ spawns               │ coordinates
//!        │                │                      │
//!        ▼                ▼                      ▼
//! ┌──────────────────────────────────────────────────────┐
//! │                    ~/tasks/                           │
//! │  *.md          — task descriptions (human-written)   │
//! │  .state/*.json — machine state (session, PRs, etc)   │
//! │  .inbox/*.msg  — worker→orchestrator messages        │
//! │  done/         — archived completed tasks            │
//! └──────────────────────────────────────────────────────┘
//!        ▲                ▲
//!        │                │
//!        │          ┌─────┴──────┐
//!        │          │ tmux       │
//!        │          │ task-*     │  ← worker sessions
//!        │          │ sessions   │
//!        │          └────────────┘
//!        │
//! ┌──────┴───────┐
//! │ GitHub API   │  ← PR status, CI, reviews
//! │ (gh CLI)     │
//! └──────────────┘
//! ```
//!
//! ## Modules
//!
//! - `main.rs` — CLI, daemon (file watcher + orchestrator spawner),
//!   legacy status, tmux helpers
//! - `state.rs` — Task/PR/tmux data types, state loading from
//!   ~/tasks/ and tmux, status derivation (ready/working/idle)
//! - `gh.rs` — Background GitHub PR data fetching with cache
//! - `tui.rs` — Interactive ratatui dashboard with Rosé Pine Dawn
//!   palette, fold/expand PRs, tmux session jumping
//!
//! ## Data flow
//!
//! - **Daemon** watches ~/tasks/ for new files and .inbox/ for
//!   worker messages. On changes, spawns a one-shot Claude
//!   orchestrator agent to reconcile state.
//! - **TUI** polls tmux sessions (2s) and GitHub API (30s) to
//!   derive live task status. Reads .state/*.json for PR mappings.
//! - **Workers** run in tmux sessions, communicate via
//!   `orch - "message"` which writes to .inbox/.
//! - **Orchestrator agent** reads task files, tmux state, and
//!   inbox messages to coordinate workers.
//!
//! ## Skills
//!
//! - `orch-worker` — development task execution skill for workers
//! - `codex-review` — runs `codex exec review` on a PR, posts
//!   findings as PR comment, presents proposals to user

mod cache;
mod gh;
mod runs;
mod state;
mod tui;

use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use clap::{Parser, Subcommand};
use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode};

const SCAN_MSG: &str = "[scan]";
const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);

// CLI

#[derive(Parser)]
#[command(name = "orch", about = "Task orchestrator for Claude Code workers")]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the background watcher daemon
    Daemon,
    /// Interactive TUI dashboard
    Tui,
    /// Show status of all tasks and workers (plain text)
    Status,
    /// Attach to a task's tmux session
    Jump { name: String },
    /// Trigger a one-shot orchestrator scan
    Scan,
    /// Spawn (or resume) a worker for a task by name.
    /// Infers session, task file, and worktree from the name.
    Spawn {
        /// Task name (e.g. foo for ~/tasks/foo.md)
        name: String,
    },
    /// Pause a task: kill its tmux session, mark paused.
    /// Orchestrator will not auto-spawn it.
    Pause {
        /// Task name
        name: String,
    },
    /// Resume a paused task: clear paused flag, spawn worker.
    Resume {
        /// Task name
        name: String,
    },
    /// Send a message to the orchestrator
    #[command(name = "-")]
    Msg { message: Vec<String> },
    /// Add a PR to a task's state
    #[command(name = "pr")]
    Pr {
        #[command(subcommand)]
        action: PrAction,
    },
    /// Garbage-collect orphan worktrees: list `task-*` worktrees under
    /// `$ORCH_REPO` whose `~/tasks/<name>.md` is gone, prompt to remove.
    Gc,
    /// Close a task: kill tmux, archive .md to done/, remove worktree.
    Close {
        /// Task name (e.g. foo for ~/tasks/foo.md)
        name: String,
    },
}

#[derive(Subcommand)]
enum PrAction {
    /// Add a PR number to a task
    Add {
        /// Task name (e.g. agentserver)
        task: String,
        /// PR number
        number: u32,
    },
    /// Remove a PR number from a task
    Rm {
        /// Task name
        task: String,
        /// PR number
        number: u32,
    },
}

// Paths

fn inbox_dir() -> PathBuf {
    state::tasks_dir().join(".inbox")
}

fn repo_dir() -> String {
    std::env::var("ORCH_REPO").expect("ORCH_REPO must be set")
}

// Inbox — file-based message queue for worker→orchestrator communication.
// Workers write via `orch - "message"`, daemon drains on file-watch events.

fn write_inbox(msg: &str) {
    let dir = inbox_dir();
    fs::create_dir_all(&dir).expect("failed to create inbox dir");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = dir.join(format!("{nanos}-{}.msg", std::process::id()));
    fs::write(path, msg).expect("failed to write inbox message");
}

fn drain_inbox() -> Option<String> {
    let mut entries: Vec<_> = fs::read_dir(inbox_dir())
        .ok()?
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "msg"))
        .collect();
    if entries.is_empty() {
        return None;
    }
    entries.sort_by_key(|e| e.file_name());

    let mut messages = Vec::new();
    for entry in entries {
        if let Ok(msg) = fs::read_to_string(entry.path()) {
            if !msg.trim().is_empty() {
                messages.push(msg);
            }
            let _ = fs::remove_file(entry.path());
        }
    }
    (!messages.is_empty()).then(|| messages.join("\n"))
}

// Orchestrator — spawns a one-shot Claude agent with the orchestrator
// persona to reconcile tasks, workers, and messages.

fn run_orchestrator(message: &str) {
    eprintln!("[orch] {message}");

    // Create per-run output directory
    let run = runs::create_run(message);
    let (run_id, output_file) = match &run {
        Some((id, path)) => {
            eprintln!("[orch] run {id}");
            (
                id.clone(),
                std::fs::File::create(path).ok(),
            )
        }
        None => (String::new(), None),
    };

    // Redirect stdout+stderr to the output file if available,
    // otherwise inherit
    let (stdout_cfg, stderr_cfg) = match &output_file {
        Some(f) => {
            let f2 = f.try_clone().unwrap();
            (Stdio::from(f.try_clone().unwrap()), Stdio::from(f2))
        }
        None => (Stdio::inherit(), Stdio::inherit()),
    };

    let mut child = match Command::new("claude")
        .args([
            "--model", "opus",
            "--agent", "orchestrator",
            "-p",
            "--dangerously-skip-permissions",
        ])
        .env("ORCH_REPO", repo_dir())
        .env_remove("CLAUDECODE")
        .stdin(Stdio::piped())
        .stdout(stdout_cfg)
        .stderr(stderr_cfg)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[orch] failed to run claude: {e}");
            if !run_id.is_empty() {
                runs::finish_run(&run_id, -1);
            }
            return;
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(message.as_bytes());
    }

    let exit_code = match child.wait() {
        Ok(s) => {
            if !s.success() {
                eprintln!("[orch] claude exited with {s}");
            }
            s.code().unwrap_or(-1)
        }
        Err(e) => {
            eprintln!("[orch] claude wait failed: {e}");
            -1
        }
    };

    if !run_id.is_empty() {
        runs::finish_run(&run_id, exit_code);
    }

    // Reconcile PRs after each orchestrator run
    state::reconcile_prs();
}

// Tmux helpers — used by daemon for activity polling and by legacy
// status/jump commands.

/// Check if a tmux session exists, matching numbered prefixes
/// (e.g. "task-foo" matches "3-task-foo").
fn has_tmux_session(name: &str) -> bool {
    let output = Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}"])
        .stderr(Stdio::null())
        .output()
        .ok();
    let Some(output) = output.filter(|o| o.status.success()) else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|n| state::session_matches(n, name))
}

fn tmux(args: &[&str]) -> bool {
    Command::new("tmux")
        .args(args)
        .status()
        .is_ok_and(|s| s.success())
}

/// Get last-activity epoch for each worker tmux session.
/// Accepts both `task-*` and numbered `N-task-*` session names.
fn session_activity() -> HashMap<String, u64> {
    let output = Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name} #{session_activity}"])
        .stderr(Stdio::null())
        .output()
        .ok();
    let Some(output) = output.filter(|o| o.status.success()) else {
        return HashMap::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (name, epoch) = line.split_once(' ')?;
            if !state::strip_numeric_prefix(name).starts_with("task-")
            {
                return None;
            }
            Some((name.to_string(), epoch.parse::<u64>().ok()?))
        })
        .collect()
}

// Task helpers — used by daemon to detect new task files.

fn known_tasks(dir: &Path) -> HashSet<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return HashSet::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect()
}

/// Extract lines between `heading` and the next `## ` (or EOF).
fn extract_section<'a>(content: &'a str, heading: &str) -> Vec<&'a str> {
    let mut lines = content.lines();
    if !lines.any(|l| l.trim().starts_with(heading)) {
        return Vec::new();
    }
    lines
        .take_while(|l| !l.trim().starts_with("## "))
        .filter(|l| !l.trim().is_empty())
        .collect()
}

// Commands

/// Legacy plain-text status output. Used when stdout is not a TTY
/// (piped, scripted) or via `orch status`.
fn cmd_status() {
    let dir = state::tasks_dir();
    println!("## Tasks\n");

    let Ok(entries) = fs::read_dir(&dir) else {
        println!("  ~/tasks/ not found");
        return;
    };

    let mut found = false;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.extension().is_some_and(|e| e == "md") {
            continue;
        }
        found = true;

        let name = path.file_stem().unwrap_or_default().to_string_lossy();
        let content = fs::read_to_string(&path).unwrap_or_default();
        let summary = extract_section(&content, "## Summary");

        let session = content
            .lines()
            .find_map(|l| {
                l.trim().strip_prefix("session:").map(|s| s.trim().to_string())
            })
            .unwrap_or_else(|| format!("task-{name}"));

        let worker = if has_tmux_session(&session) {
            format!("running ({session})")
        } else {
            "none".into()
        };

        println!("  {name}  [worker: {worker}]");
        if summary.is_empty() {
            let desc = content
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .trim()
                .trim_start_matches('#')
                .trim();
            println!("    {desc}");
        } else {
            for line in &summary {
                println!("    {line}");
            }
        }
        println!();
    }

    if !found {
        println!("  (no tasks)");
    }
}

fn cmd_jump(name: &str) {
    let session = if name.starts_with("task-") {
        name.to_string()
    } else {
        format!("task-{name}")
    };

    if !has_tmux_session(&session) {
        eprintln!("No tmux session '{session}' found.");
        let _ = Command::new("tmux").arg("ls").status();
        return;
    }

    let action = if std::env::var("TMUX").is_ok() {
        "switch-client"
    } else {
        "attach-session"
    };
    let _ = Command::new("tmux").args([action, "-t", &session]).status();
}

fn cmd_spawn(name: &str) {
    let session = format!("task-{name}");
    if has_tmux_session(&session) {
        eprintln!("[spawn] '{name}' already has a tmux session");
        return;
    }

    let mut meta = state::load_task_meta(name);
    let work_dir = if !meta.worktree.is_empty() {
        meta.worktree
            .replace(
                "~",
                &dirs::home_dir().unwrap_or_default().to_string_lossy(),
            )
    } else {
        format!("{}/task-{name}", repo_dir())
    };

    let task_file = state::tasks_dir().join(format!("{name}.md"));
    if !task_file.exists() {
        eprintln!(
            "[spawn] task file not found: {}",
            task_file.display(),
        );
        return;
    }
    let cmd =
        format!("claude '/orch:worker {}'", task_file.display());

    if !tmux(&["new-session", "-d", "-s", &session, "-c", &work_dir]) {
        eprintln!("[spawn] failed to create tmux session");
        return;
    }
    if !tmux(&["send-keys", "-t", &session, &cmd, "Enter"]) {
        eprintln!("[spawn] failed to start worker");
        return;
    }

    // Persist session, worktree, and clear paused flag
    meta.session = session.clone();
    meta.worktree = work_dir;
    meta.paused = false;
    state::save_task_meta(name, &meta);

    eprintln!("[spawn] {session} started");
}

fn cmd_pause(name: &str) {
    let mut meta = state::load_task_meta(name);
    if !meta.session.is_empty() {
        // Find actual tmux name (may be N-task-<name>) and kill it
        if let Some(actual) = find_actual_session(&meta.session) {
            let _ = Command::new("tmux")
                .args(["kill-session", "-t", &actual])
                .stderr(Stdio::null())
                .status();
        }
    }
    meta.paused = true;
    state::save_task_meta(name, &meta);
    eprintln!("[pause] {name} paused");
}

/// Find the actual tmux session name for an expected session,
/// handling numbered prefixes (`task-foo` → `3-task-foo`).
fn find_actual_session(expected: &str) -> Option<String> {
    let output = Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find(|n| state::session_matches(n, expected))
        .map(String::from)
}

fn cmd_resume(name: &str) {
    let mut meta = state::load_task_meta(name);
    meta.paused = false;
    state::save_task_meta(name, &meta);
    cmd_spawn(name);
}

// Worktree garbage collection — `orch gc`.
//
// Worktrees live as `$ORCH_REPO/task-<name>` siblings of `main`. A
// worktree is "bound" if `~/tasks/<name>.md` exists. Anything else is an
// orphan that the user can remove. Removal uses plain `git worktree
// remove` (no `--force`) — dirty worktrees fail with a warning so the
// user can intervene.

/// Find worktrees under `$ORCH_REPO` matching `task-*` whose
/// corresponding `~/tasks/<name>.md` is gone.
fn find_orphan_worktrees() -> Vec<PathBuf> {
    let repo = match std::env::var("ORCH_REPO") {
        Ok(r) => PathBuf::from(r),
        Err(_) => return Vec::new(),
    };
    let Ok(entries) = fs::read_dir(&repo) else {
        return Vec::new();
    };
    let task_dir = state::tasks_dir();
    let mut orphans = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(basename) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(task_name) = basename.strip_prefix("task-") else {
            continue;
        };
        if task_dir.join(format!("{task_name}.md")).exists() {
            continue;
        }
        orphans.push(path);
    }
    orphans.sort();
    orphans
}

/// Remove a worktree. First tries `git worktree remove`. If git says
/// "not a working tree" (already disowned — common after a partial
/// cleanup), the directory is a pure orphan with no git state, so we
/// `rm -rf` it. Other errors (notably "contains modified or untracked
/// files") propagate so the caller can warn the user.
fn remove_worktree(path: &Path) -> Result<(), String> {
    let repo = std::env::var("ORCH_REPO")
        .map_err(|_| "ORCH_REPO not set".to_string())?;
    let main = format!("{repo}/main");
    let path_str = path.to_str().ok_or_else(|| "non-utf8 path".to_string())?;
    let output = Command::new("git")
        .args(["worktree", "remove", path_str])
        .current_dir(&main)
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let msg = stderr.trim().to_string();

    // Disowned worktree: git no longer tracks it, no .git/.jj inside.
    // Safe to delete the orphan directory.
    if msg.contains("is not a working tree")
        && !path.join(".git").exists()
        && !path.join(".jj").exists()
    {
        fs::remove_dir_all(path).map_err(|e| e.to_string())?;
        return Ok(());
    }

    Err(msg)
}

/// Close a task: kill its tmux session, archive ~/tasks/<name>.md to
/// done/, remove the worktree, drop the state file. Each step warns
/// loudly on failure but does not abort — the user can clean up the
/// remaining drift manually (or with `orch gc`).
fn cmd_close(name: &str) {
    let meta = state::load_task_meta(name);

    if !meta.session.is_empty() {
        if let Some(actual) = find_actual_session(&meta.session) {
            let killed = Command::new("tmux")
                .args(["kill-session", "-t", &actual])
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            if killed {
                eprintln!("[close] killed tmux session {actual}");
            } else {
                eprintln!("[close] WARNING: failed to kill tmux session {actual}");
            }
        }
    }

    let dir = state::tasks_dir();
    let md = dir.join(format!("{name}.md"));
    if md.exists() {
        let done_dir = dir.join("done");
        fs::create_dir_all(&done_dir).ok();
        let archived = done_dir.join(format!("{name}.md"));
        match fs::rename(&md, &archived) {
            Ok(()) => eprintln!(
                "[close] archived {name}.md -> done/",
            ),
            Err(e) => eprintln!(
                "[close] WARNING: failed to archive {name}.md: {e}",
            ),
        }
    }

    if !meta.worktree.is_empty() {
        let home = dirs::home_dir().unwrap_or_default();
        let wt = meta.worktree.replace("~", &home.to_string_lossy());
        let path = Path::new(&wt);
        if path.exists() {
            match remove_worktree(path) {
                Ok(()) => eprintln!("[close] removed worktree {wt}"),
                Err(e) => eprintln!(
                    "[close] WARNING: worktree {wt} not removed ({e})\n  run `git worktree remove --force {wt}` to override",
                ),
            }
        }
    }

    let state_path = dir.join(".state").join(format!("{name}.json"));
    if state_path.exists() {
        let _ = fs::remove_file(&state_path);
    }

    eprintln!("[close] {name} closed");
}

fn cmd_gc() {
    let orphans = find_orphan_worktrees();
    if orphans.is_empty() {
        eprintln!("[gc] no orphan worktrees");
        return;
    }

    eprintln!("[gc] found {} orphan worktree(s):", orphans.len());
    for path in &orphans {
        eprintln!("  {}", path.display());
    }
    eprint!("[gc] remove all? [y/N] ");
    io::stderr().flush().ok();

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        eprintln!("[gc] aborted");
        return;
    }
    if !input.trim().eq_ignore_ascii_case("y") {
        eprintln!("[gc] aborted");
        return;
    }

    for path in orphans {
        match remove_worktree(&path) {
            Ok(()) => eprintln!("[gc] removed {}", path.display()),
            Err(e) => eprintln!(
                "[gc] failed to remove {} ({e}); run `git worktree remove --force {}` to override",
                path.display(),
                path.display(),
            ),
        }
    }
}

/// Background thread: polls tmux every 2s, writes status cache.
/// Sweeps stale busy markers every 5 minutes.
fn spawn_status_loop() {
    use std::thread;
    thread::spawn(|| {
        let stale_secs = state::busy_stale_secs();
        let mut sweep_counter: u32 = 0;

        loop {
            state::ensure_state_files();
            let order = state::load_order();
            let sessions = state::load_tmux_sessions();
            state::auto_pause_orphaned(&sessions);
            let tasks =
                state::load_tasks(&order, &sessions, stale_secs);

            let mut cached_tasks = HashMap::new();
            for task in &tasks {
                let session = &task.meta.session;
                let matched = sessions.values().find(|s| {
                    state::session_matches(&s.name, session)
                });
                cached_tasks.insert(
                    task.name.clone(),
                    cache::CachedTask {
                        session: session.clone(),
                        actual_session: matched
                            .map(|s| s.name.clone())
                            .unwrap_or_default(),
                        status: match task.status {
                            state::TaskStatus::Ready => "ready",
                            state::TaskStatus::Working => "working",
                            state::TaskStatus::Input => "input",
                            state::TaskStatus::Idle => "idle",
                            state::TaskStatus::Paused => "paused",
                            state::TaskStatus::Attached => "attached",
                        }
                        .to_string(),
                        has_active_process: matched
                            .is_some_and(|s| s.has_active_process),
                    },
                );
            }

            cache::write_status(&cache::StatusCache {
                generated_at: cache::now_epoch(),
                tasks: cached_tasks,
            });
            cache::write_lease();

            // Sweep stale busy markers every 150 ticks (~5 min at 2s/tick)
            sweep_counter = sweep_counter.wrapping_add(1);
            if sweep_counter % 150 == 0 {
                state::sweep_stale_markers(stale_secs);
            }

            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

/// Background thread: reconciles PRs and fetches PR data every 30s.
fn spawn_pr_loop() {
    use std::thread;
    thread::spawn(|| {
        let pr_cache = gh::PrCache::new();
        loop {
            // Reconcile: add new PRs, remove merged/closed
            state::reconcile_prs();

            // Fetch PR data for all tracked PRs
            let dir = state::tasks_dir();
            let names = state::load_task_names(&dir);
            let all_prs: Vec<u32> = names
                .iter()
                .flat_map(|n| state::load_task_meta(n).prs)
                .collect();

            pr_cache.refresh(all_prs.clone());
            std::thread::sleep(Duration::from_secs(3));

            let mut cached_prs = HashMap::new();
            for num in &all_prs {
                if let Some(data) = pr_cache.get(*num) {
                    cached_prs.insert(
                        *num,
                        cache::CachedPr::from_pr_data(&data),
                    );
                }
            }
            cache::write_prs(&cache::PrCache {
                generated_at: cache::now_epoch(),
                prs: cached_prs,
            });

            std::thread::sleep(Duration::from_secs(27));
        }
    });
}

/// Background daemon: watches ~/tasks/ for new files and .inbox/ for
/// worker messages. On changes, spawns a one-shot orchestrator agent.
fn cmd_daemon() {
    let dir = state::tasks_dir();
    let inbox = inbox_dir();
    fs::create_dir_all(&dir).ok();
    fs::create_dir_all(&inbox).ok();

    runs::prune_old_runs();
    state::ensure_state_files();
    state::sweep_stale_markers(state::busy_stale_secs());
    eprintln!("[orch] reconciling PRs...");
    state::reconcile_prs();

    // Start background cache loops
    spawn_status_loop();
    spawn_pr_loop();

    eprintln!("[orch] daemon started, watching {}", dir.display());

    // Fold pending inbox messages into the initial scan
    let mut startup_msg = String::new();
    if let Some(msgs) = drain_inbox() {
        startup_msg.push_str("[message] ");
        startup_msg.push_str(&msgs);
        startup_msg.push_str("\n\n");
    }
    startup_msg.push_str(SCAN_MSG);
    eprintln!("[orch] running initial scan...");
    run_orchestrator(&startup_msg);

    let mut tasks = known_tasks(&dir);
    let (tx, rx) = mpsc::channel();
    let mut debouncer =
        new_debouncer(Duration::from_secs(3), tx).expect("failed to create watcher");
    debouncer
        .watcher()
        .watch(&dir, RecursiveMode::Recursive)
        .expect("failed to watch ~/tasks");

    let mut last_activity = session_activity();
    eprintln!("[orch] watching for changes (activity poll every 20m)...");

    loop {
        match rx.recv_timeout(POLL_INTERVAL) {
            Ok(Ok(events)) => {
                // Check for inbox messages triggered by file events
                let inbox_msgs = events
                    .iter()
                    .any(|e| e.path.starts_with(&inbox))
                    .then(|| drain_inbox())
                    .flatten();

                // Detect new task files
                let current = known_tasks(&dir);
                let new_tasks: Vec<_> =
                    current.difference(&tasks).cloned().collect();
                tasks = current;

                let mut parts = Vec::new();
                if let Some(msgs) = inbox_msgs {
                    parts.push(format!("[message] {msgs}"));
                }
                for task in &new_tasks {
                    parts.push(format!("[new-task] {task}"));
                }
                if !parts.is_empty() {
                    last_activity = session_activity();
                    run_orchestrator(&parts.join("\n\n"));
                }
            }
            Ok(Err(e)) => eprintln!("[orch] watch error: {e:?}"),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Reconcile PRs on each poll
                state::reconcile_prs();

                // Poll tmux activity — trigger scan for sessions
                // that have new output since last check
                let current = session_activity();
                let changed: Vec<_> = current
                    .iter()
                    .filter(|(name, epoch)| {
                        last_activity
                            .get(name.as_str())
                            .map_or(true, |prev| *epoch > prev)
                    })
                    .map(|(name, _)| name.clone())
                    .collect();
                last_activity = current;

                if !changed.is_empty() {
                    let list = changed.join(", ");
                    eprintln!("[orch] activity in: {list}");
                    run_orchestrator(&format!("[scan] {list}"));
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        // Default: TUI if interactive terminal, plain status otherwise
        None => {
            if atty::is(atty::Stream::Stdout) {
                tui::run().expect("TUI failed");
            } else {
                cmd_status();
            }
        }
        Some(Cmd::Tui) => tui::run().expect("TUI failed"),
        Some(Cmd::Status) => cmd_status(),
        Some(Cmd::Jump { name }) => cmd_jump(&name),
        Some(Cmd::Spawn { name }) => cmd_spawn(&name),
        Some(Cmd::Pause { name }) => cmd_pause(&name),
        Some(Cmd::Resume { name }) => cmd_resume(&name),
        Some(Cmd::Daemon) => cmd_daemon(),
        Some(Cmd::Scan) => {
            write_inbox(SCAN_MSG);
            eprintln!("[orch] scan triggered");
        }
        Some(Cmd::Msg { message }) => {
            write_inbox(&message.join(" "));
            eprintln!("[orch] message sent");
        }
        Some(Cmd::Gc) => cmd_gc(),
        Some(Cmd::Close { name }) => cmd_close(&name),
        Some(Cmd::Pr { action }) => match action {
            PrAction::Add { task, number } => {
                let mut meta = state::load_task_meta(&task);
                if !meta.prs.contains(&number) {
                    meta.prs.push(number);
                    state::save_task_meta(&task, &meta);
                    eprintln!("[orch] added PR #{number} to {task}");
                } else {
                    eprintln!("[orch] PR #{number} already in {task}");
                }
            }
            PrAction::Rm { task, number } => {
                let mut meta = state::load_task_meta(&task);
                meta.prs.retain(|&n| n != number);
                state::save_task_meta(&task, &meta);
                eprintln!("[orch] removed PR #{number} from {task}");
            }
        },
    }
}
