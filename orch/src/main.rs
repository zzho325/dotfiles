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

mod gh;
mod runs;
mod state;
mod tui;

use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use clap::{Parser, Subcommand};
use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode};

const SCAN_MSG: &str = "[scan]";
const POLL_INTERVAL: Duration = Duration::from_secs(20 * 60);

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
    /// Spawn a worker in a tmux session for a task file
    Spawn {
        /// Session name (e.g. task-tm)
        session: String,
        /// Path to the task file (e.g. ~/tasks/tm.md)
        task_file: String,
        /// Working directory (defaults to $ORCH_REPO/main)
        #[arg(long, short = 'C')]
        dir: Option<String>,
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

fn tasks_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join("tasks")
}

fn inbox_dir() -> PathBuf {
    tasks_dir().join(".inbox")
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

fn has_tmux_session(name: &str) -> bool {
    Command::new("tmux")
        .args(["has-session", "-t", name])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn tmux(args: &[&str]) -> bool {
    Command::new("tmux")
        .args(args)
        .status()
        .is_ok_and(|s| s.success())
}

/// Get last-activity epoch for each task-* tmux session.
/// Used by the daemon to detect worker activity changes.
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
            if !name.starts_with("task-") {
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
    let dir = tasks_dir();
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

fn cmd_spawn(session: &str, task_file: &str, dir: Option<&str>) {
    if has_tmux_session(session) {
        eprintln!("[spawn] session '{session}' already exists");
        return;
    }

    let work_dir = dir
        .map(String::from)
        .unwrap_or_else(|| format!("{}/main", repo_dir()));
    let cmd = format!("claude '/orch:worker {task_file}'");

    if !tmux(&["new-session", "-d", "-s", session, "-c", &work_dir]) {
        eprintln!("[spawn] failed to create tmux session '{session}'");
        return;
    }
    if !tmux(&["send-keys", "-t", session, &cmd, "Enter"]) {
        eprintln!("[spawn] failed to start worker");
        return;
    }

    eprintln!("[spawn] {session} started with {task_file}");
}

/// Background daemon: watches ~/tasks/ for new files and .inbox/ for
/// worker messages. On changes, spawns a one-shot orchestrator agent.
/// Also polls tmux session activity every 20 minutes.
fn cmd_daemon() {
    unsafe { std::env::remove_var("CLAUDECODE") };

    let dir = tasks_dir();
    let inbox = inbox_dir();
    fs::create_dir_all(&dir).ok();
    fs::create_dir_all(&inbox).ok();

    runs::prune_old_runs();
    eprintln!("[orch] reconciling PRs...");
    state::reconcile_prs();
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
        Some(Cmd::Spawn { session, task_file, dir }) => {
            cmd_spawn(&session, &task_file, dir.as_deref())
        }
        Some(Cmd::Daemon) => cmd_daemon(),
        Some(Cmd::Scan) => {
            write_inbox(SCAN_MSG);
            eprintln!("[orch] scan triggered");
        }
        Some(Cmd::Msg { message }) => {
            write_inbox(&message.join(" "));
            eprintln!("[orch] message sent");
        }
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
