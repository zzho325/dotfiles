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
mod linear;
mod remote_agent;
mod runs;
mod state;
mod store;
mod tui3;

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
#[cfg(unix)]
use std::os::unix::process::CommandExt;

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
    /// Manage Linear tickets linked to a task.
    #[command(name = "linear")]
    Linear {
        #[command(subcommand)]
        action: LinearAction,
    },
    /// Run background PR reviews and surface results to agent sessions.
    Review {
        #[command(subcommand)]
        action: ReviewAction,
    },
    /// Start or prompt remote agent sessions.
    Remote {
        #[command(subcommand)]
        action: RemoteAction,
    },
    /// Garbage-collect orphan task worktrees and temporary PR worktrees.
    Gc,
    /// Close a task: kill tmux, archive .md to done/, remove worktree
    /// (unless `--keep-worktree`).
    Close {
        /// Task name (e.g. foo for ~/tasks/foo.md)
        name: String,
        /// Keep the git worktree on disk (overrides cleanup_on_close).
        #[arg(long)]
        keep_worktree: bool,
    },
    /// Internal — Claude Code busy-marker hooks. Reads
    /// `{session_id, cwd}` JSON on stdin.
    Busy {
        #[command(subcommand)]
        action: BusyAction,
    },
    /// Render the new TUI to stdout for debugging (no raw mode).
    /// Useful for capturing what the layout looks like with live data
    /// without needing an interactive terminal.
    RenderDebug {
        /// Terminal width in cells (default 150)
        #[arg(long, default_value = "150")]
        width: u16,
        /// Terminal height in cells (default 40)
        #[arg(long, default_value = "40")]
        height: u16,
        /// Detail tab: overview | prs | linear | panes
        #[arg(long, default_value = "overview")]
        tab: String,
        /// Pane focus: list | details | log
        #[arg(long, default_value = "list")]
        focus: String,
        /// Selected task index (0-based) — defaults to 0
        #[arg(long, default_value = "0")]
        select: usize,
        /// Push the Linear detail view for this issue key (e.g.
        /// ENG-29535). Implies `--tab linear --focus details`.
        #[arg(long)]
        linear_detail: Option<String>,
        /// Set the Linear list cursor to this issue key (e.g. ENG-26405).
        /// Implies `--tab linear --focus details`.
        #[arg(long)]
        linear_cursor: Option<String>,
    },
}

#[derive(Subcommand)]
enum LinearAction {
    /// Add a Linear issue key (e.g. ENG-29151) to a task.
    Add {
        /// Task name (e.g. infra-triage)
        task: String,
        /// Linear issue key (e.g. ENG-29151)
        key: String,
    },
    /// Remove a Linear issue key from a task.
    Rm {
        /// Task name
        task: String,
        /// Linear issue key
        key: String,
    },
    /// Auto-scan task markdown for [A-Z]+-\d+ patterns and link them.
    Scan {
        /// Task name (omit to scan all open tasks)
        task: Option<String>,
    },
    /// List Linear keys linked to a task.
    Ls {
        /// Task name
        task: String,
    },
    /// Remove keys from all task records that the linear cache has
    /// flagged as not-found (deleted issues, typos, non-Linear
    /// patterns like REQ-01).
    Clean,
}

#[derive(Subcommand)]
enum BusyAction {
    /// Write busy marker for the current Claude turn.
    Start,
    /// Remove the busy marker.
    Stop,
}

#[derive(Subcommand)]
enum ReviewAction {
    /// Start a background review for a PR URL, PR number, or prompt.
    Start {
        /// PR URL, PR number, or review target prompt.
        target: String,
        /// Optional display label.
        #[arg(long)]
        label: Option<String>,
        /// Run review with local `codex exec review` instead of remote Codex.
        #[arg(long, conflicts_with = "claude")]
        local: bool,
        /// Run review with remote Claude Code instead of remote Codex.
        #[arg(long, conflicts_with = "local")]
        claude: bool,
    },
    /// Internal — run a queued review by id.
    #[command(hide = true)]
    Run {
        /// Review id.
        id: String,
    },
    /// List review runs.
    List,
    /// Print one ready review. Defaults to the newest ready review.
    Show {
        /// Review id.
        id: Option<String>,
        /// Mark the review consumed after printing.
        #[arg(long)]
        consume: bool,
    },
    /// Internal — Codex hook entry point.
    #[command(hide = true)]
    Hook,
}

#[derive(Subcommand)]
enum RemoteAction {
    /// Create a remote agent session and print its id.
    Start {
        /// Start a remote Claude Code session instead of remote Codex.
        #[arg(long)]
        claude: bool,
        /// Model passed to the remote agent API.
        #[arg(long)]
        model: Option<String>,
        /// Reasoning effort passed to the remote agent API.
        #[arg(long)]
        effort: Option<String>,
    },
    /// Send a prompt to a new or existing remote session and print the answer.
    Ask {
        /// Prompt text. All remaining args are joined with spaces.
        prompt: Vec<String>,
        /// Existing remote session id. Omit to create a new session.
        #[arg(long)]
        session: Option<String>,
        /// Start a remote Claude Code session instead of remote Codex.
        #[arg(long)]
        claude: bool,
        /// Model passed to the remote agent API.
        #[arg(long)]
        model: Option<String>,
        /// Reasoning effort passed to the remote agent API.
        #[arg(long)]
        effort: Option<String>,
        /// Maximum minutes to wait for the remote turn to finish.
        #[arg(long, default_value_t = 45)]
        timeout_minutes: u64,
    },
    /// List remote agent sessions.
    List {
        /// Maximum sessions to fetch.
        #[arg(long, default_value_t = 20)]
        limit: u32,
        /// Filter by source, e.g. api, slack, linear, or all.
        #[arg(long)]
        source: Option<String>,
        /// Print raw JSON.
        #[arg(long)]
        json: bool,
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

    // Closed tasks whose worktree cleanup failed — surface so the user
    // can `git worktree remove --force` deliberately.
    let pending: Vec<store::TaskRecord> = store::Store::default()
        .load_closed_records()
        .into_iter()
        .filter(|r| {
            r.drift.cleanup_failed
                && !r.worktree.path.is_empty()
                && Path::new(&state::expand_home(&r.worktree.path)).exists()
        })
        .collect();
    if !pending.is_empty() {
        println!("## Pending cleanup\n");
        for r in &pending {
            let wt = state::expand_home(&r.worktree.path);
            println!("  {}  [worktree: {wt}]", r.slug);
            if let Some(err) = &r.drift.last_error {
                println!("    {err}");
            }
        }
        println!();
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

    state::ensure_state_files();
    let store = store::Store::default();
    let Some(record) = store.load_record_by_slug(name) else {
        eprintln!("[spawn] no task '{name}' (create ~/tasks/{name}.md first)");
        return;
    };

    let task_file = state::tasks_dir().join(format!("{name}.md"));
    if !task_file.exists() {
        eprintln!(
            "[spawn] task file not found: {}",
            task_file.display(),
        );
        return;
    }
    let allow_existing_dirty = record.desired_state != store::DesiredState::New;
    let work_dir = match state::prepare_task_worktree(
        name,
        &record.worktree.path,
        allow_existing_dirty,
    ) {
        Ok(path) => path,
        Err(e) => {
            eprintln!("[spawn] worktree setup failed: {e}");
            return;
        }
    };
    if let Err(e) = state::ensure_worktree_notes(&work_dir) {
        eprintln!("[spawn] notes setup failed: {e}");
        return;
    }
    let cmd = record.agent.worker_kind.worker_cmd(&task_file);

    if !tmux(&["new-session", "-d", "-s", &session, "-c", &work_dir]) {
        eprintln!("[spawn] failed to create tmux session");
        return;
    }
    if !tmux(&["send-keys", "-t", &session, &cmd, "Enter"]) {
        eprintln!("[spawn] failed to start worker");
        return;
    }

    let now = cache::now_epoch();
    store.update_record_by_slug(name, |r| {
        r.tmux.session_name = session.clone();
        r.worktree.path = work_dir.clone();
        r.desired_state = store::DesiredState::Active;
        if r.started_at.is_none() {
            r.started_at = Some(now);
        }
        r.updated_at = now;
    });

    eprintln!("[spawn] {session} started");
}

fn cmd_pause(name: &str) {
    let store = store::Store::default();
    let session_name = store
        .load_record_by_slug(name)
        .map(|r| r.tmux.session_name)
        .unwrap_or_default();
    if !session_name.is_empty() {
        if let Some(actual) = find_actual_session(&session_name) {
            let _ = Command::new("tmux")
                .args(["kill-session", "-t", &actual])
                .stderr(Stdio::null())
                .status();
        }
    }
    let now = cache::now_epoch();
    store.update_record_by_slug(name, |r| {
        r.desired_state = store::DesiredState::Paused;
        r.paused_at = Some(now);
        r.updated_at = now;
    });
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
    let store = store::Store::default();
    let now = cache::now_epoch();
    store.update_record_by_slug(name, |r| {
        r.desired_state = store::DesiredState::Active;
        r.updated_at = now;
    });
    cmd_spawn(name);
}

// Worktree garbage collection — `orch gc`.
//
// Task worktrees live as `$ORCH_REPO/task-<name>` siblings of `main`.
// A task worktree is "bound" if `~/tasks/<name>.md` exists. Temporary PR
// review worktrees live under `/private/tmp` and are always disposable.

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

fn find_tmp_worktrees() -> Vec<PathBuf> {
    let mut paths = HashSet::new();
    for path in registered_tmp_worktrees() {
        if is_tmp_review_worktree(&path) {
            paths.insert(path);
        }
    }
    if let Ok(entries) = fs::read_dir("/private/tmp") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && is_tmp_review_worktree(&path) {
                paths.insert(path);
            }
        }
    }
    let mut paths: Vec<_> = paths.into_iter().collect();
    paths.sort();
    paths
}

fn registered_tmp_worktrees() -> Vec<PathBuf> {
    let repo = match std::env::var("ORCH_REPO") {
        Ok(r) => PathBuf::from(r),
        Err(_) => return Vec::new(),
    };
    let main = repo.join("main");
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(&main)
        .output()
        .ok();
    let Some(output) = output.filter(|o| o.status.success()) else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .filter(|path| path.starts_with("/private/tmp"))
        .collect()
}

fn is_tmp_review_worktree(path: &Path) -> bool {
    if path.parent() != Some(Path::new("/private/tmp")) {
        return false;
    }
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    if let Ok(prefix) = std::env::var("ORCH_TMP_REVIEW_PREFIX") {
        let prefix = prefix.trim();
        if !prefix.is_empty() && name.starts_with(prefix) {
            return true;
        }
    }
    if name.starts_with("repo-pr") {
        return true;
    }
    let Some(rest) = name.strip_prefix("pr") else {
        return false;
    };
    rest.chars().next().is_some_and(|c| c.is_ascii_digit())
}

fn prune_worktree_metadata() {
    let Ok(repo) = std::env::var("ORCH_REPO") else {
        return;
    };
    let _ = Command::new("git")
        .args(["worktree", "prune"])
        .current_dir(Path::new(&repo).join("main"))
        .status();
}

/// Close a task. FSM transition runs first; archive failure aborts the
/// rest of cleanup since the .md is the only durable history. tmux and
/// worktree failures warn but don't roll back.
fn cmd_close(name: &str, keep_worktree: bool) {
    let store = store::Store::default();
    let Some(record) = store.load_record_by_slug(name) else {
        eprintln!("[close] no task '{name}'");
        return;
    };

    let session_name = record.tmux.session_name.clone();
    let worktree_path = record.worktree.path.clone();
    let cleanup_on_close = record.worktree.cleanup_on_close;
    let id = record.id;
    let now = cache::now_epoch();
    let archive_path = state::tasks_dir()
        .join("done")
        .join(format!("{id}-{name}.md"));

    store.update_record_by_slug(name, |r| {
        r.desired_state = store::DesiredState::Closed;
        r.closed_at = Some(now);
        r.archived_task_file = Some(archive_path.clone());
        r.updated_at = now;
    });
    if let Some(mut registry) = store.load_registry() {
        registry.open_order.retain(|i| *i != id);
        if !registry.closed_order.contains(&id) {
            registry.closed_order.push(id);
        }
        store.save_registry(&registry);
    }

    // Archive .md before destructive cleanup — the .md is the only
    // durable handle to the task's history.
    let dir = state::tasks_dir();
    let md = dir.join(format!("{name}.md"));
    if md.exists() {
        if fs::create_dir_all(archive_path.parent().unwrap_or(&dir)).is_err() {
            eprintln!("[close] FAIL: could not create done/ — aborting cleanup");
            return;
        }
        if let Err(e) = fs::rename(&md, &archive_path) {
            eprintln!(
                "[close] FAIL: could not archive {name}.md ({e}) — aborting cleanup"
            );
            return;
        }
        eprintln!("[close] archived {} -> {}", md.display(), archive_path.display());
    }

    if !session_name.is_empty() {
        if let Some(actual) = find_actual_session(&session_name) {
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

    if !keep_worktree && cleanup_on_close && !worktree_path.is_empty() {
        let wt = state::expand_home(&worktree_path);
        let path = Path::new(&wt);
        if path.exists() {
            match state::remove_worktree(path) {
                Ok(()) => {
                    store.update_record_by_slug(name, |r| {
                        r.drift.cleanup_failed = false;
                        r.drift.cleanup_pending = false;
                        r.drift.last_error = None;
                    });
                    eprintln!("[close] removed worktree {wt}");
                }
                Err(e) => {
                    let err_str = store.mark_worktree_cleanup_failed(name, &e);
                    eprintln!(
                        "[close] WARNING: worktree {wt} not removed ({err_str})\n  run `git worktree remove --force {wt}` to override",
                    );
                }
            }
        } else {
            store.update_record_by_slug(name, |r| {
                r.drift.cleanup_failed = false;
                r.drift.cleanup_pending = false;
                r.drift.last_error = None;
            });
        }
    }

    eprintln!("[close] {name} closed");
}

// Busy-marker hooks. Failures are silent — the hook fires on every
// prompt and partial stdin shouldn't surface to the user.
// ORCH_HOOK_DEBUG routes diagnostics to stderr.

fn parse_busy_input(json: &str) -> Option<(String, String)> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let session_id = v["session_id"].as_str()?.to_string();
    if session_id.is_empty() {
        return None;
    }
    let cwd = v["cwd"].as_str().unwrap_or("").to_string();
    Some((session_id, cwd))
}

fn write_busy_marker(busy_dir: &Path, session_id: &str, cwd: &str) -> bool {
    let path = busy_dir.join(session_id);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    // started_at is informational only; staleness is computed from
    // the marker file's mtime, not this field.
    let payload = serde_json::json!({
        "cwd": cwd,
        "started_at": cache::now_epoch(),
        "pid": std::process::id(),
    });
    state::atomic_write(&path, &payload.to_string())
}

fn busy_debug() -> bool {
    std::env::var_os("ORCH_HOOK_DEBUG").is_some()
}

fn read_stdin_to_string() -> String {
    use std::io::Read;
    let mut buf = String::new();
    let _ = io::stdin().read_to_string(&mut buf);
    buf
}

fn cmd_busy_start() {
    let json = read_stdin_to_string();
    let Some((session_id, cwd)) = parse_busy_input(&json) else {
        if busy_debug() {
            eprintln!("[orch busy start] missing session_id or bad JSON");
        }
        return;
    };
    let dir = state::busy_dir();
    if !write_busy_marker(&dir, &session_id, &cwd) && busy_debug() {
        eprintln!(
            "[orch busy start] atomic_write failed: {}",
            dir.join(&session_id).display(),
        );
    }
}

fn cmd_busy_stop() {
    let json = read_stdin_to_string();
    let Some((session_id, _cwd)) = parse_busy_input(&json) else {
        if busy_debug() {
            eprintln!("[orch busy stop] missing session_id or bad JSON");
        }
        return;
    };
    let _ = fs::remove_file(state::busy_dir().join(&session_id));
}

// Review mailbox — background Codex PR reviews that surface through hooks.

const REVIEW_TTL_SECS: u64 = 24 * 3600;
const REVIEW_MAX_RUNS: usize = 100;
const REVIEW_BACKEND_REMOTE: &str = "remote";
const REVIEW_BACKEND_REMOTE_CLAUDE: &str = "remote-claude";
const REVIEW_BACKEND_LOCAL: &str = "local";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ReviewMeta {
    id: String,
    target: String,
    label: String,
    cwd: String,
    git_root: String,
    status: String,
    started_at: u64,
    #[serde(default)]
    finished_at: Option<u64>,
    #[serde(default)]
    exit_code: Option<i32>,
    #[serde(default)]
    consumed_at: Option<u64>,
    #[serde(default)]
    stop_notified_at: Option<u64>,
    #[serde(default)]
    backend: Option<String>,
    #[serde(default)]
    remote_session_id: Option<String>,
}

fn reviews_dir() -> PathBuf {
    state::tasks_dir().join(".orch").join("reviews")
}

fn review_dir(id: &str) -> PathBuf {
    reviews_dir().join(id)
}

fn review_meta_path(id: &str) -> PathBuf {
    review_dir(id).join("meta.json")
}

fn save_review_meta(meta: &ReviewMeta) -> bool {
    let path = review_meta_path(&meta.id);
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_err() {
            return false;
        }
    }
    let Ok(json) = serde_json::to_string_pretty(meta) else {
        return false;
    };
    state::atomic_write(&path, &json)
}

fn load_review_meta(id: &str) -> Option<ReviewMeta> {
    fs::read_to_string(review_meta_path(id))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

fn load_review_metas() -> Vec<ReviewMeta> {
    let Ok(entries) = fs::read_dir(reviews_dir()) else {
        return Vec::new();
    };
    let mut metas: Vec<ReviewMeta> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().to_string();
            load_review_meta(&id)
        })
        .collect();
    metas.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    metas
}

fn prune_old_reviews() {
    let Ok(entries) = fs::read_dir(reviews_dir()) else {
        return;
    };
    let cutoff = cache::now_epoch().saturating_sub(REVIEW_TTL_SECS);
    let mut dirs: Vec<(PathBuf, u64)> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().to_string();
            let meta = load_review_meta(&id)?;
            Some((e.path(), meta.started_at))
        })
        .collect();

    dirs.sort_by(|a, b| b.1.cmp(&a.1));
    for (i, (path, started_at)) in dirs.iter().enumerate() {
        if *started_at < cutoff || i >= REVIEW_MAX_RUNS {
            let _ = fs::remove_dir_all(path);
        }
    }
}

fn git_root_for(cwd: &Path) -> Option<PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--show-toplevel"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let root = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!root.is_empty()).then(|| PathBuf::from(root))
}

fn review_target_label(target: &str, label: Option<&str>) -> String {
    if let Some(label) = label.filter(|s| !s.trim().is_empty()) {
        return label.trim().to_string();
    }
    target
        .trim()
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(target)
        .chars()
        .take(80)
        .collect()
}

fn review_backend(local: bool, claude: bool) -> &'static str {
    if claude {
        REVIEW_BACKEND_REMOTE_CLAUDE
    } else if local {
        REVIEW_BACKEND_LOCAL
    } else {
        REVIEW_BACKEND_REMOTE
    }
}

fn cmd_review_start(target: &str, label: Option<&str>, local: bool, claude: bool) {
    prune_old_reviews();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let git_root = git_root_for(&cwd).unwrap_or_else(|| cwd.clone());
    let id = format!("{}-{}", cache::now_epoch(), std::process::id());
    let backend = review_backend(local, claude);
    let meta = ReviewMeta {
        id: id.clone(),
        target: target.to_string(),
        label: review_target_label(target, label),
        cwd: cwd.to_string_lossy().to_string(),
        git_root: git_root.to_string_lossy().to_string(),
        status: "running".into(),
        started_at: cache::now_epoch(),
        finished_at: None,
        exit_code: None,
        consumed_at: None,
        stop_notified_at: None,
        backend: Some(backend.to_string()),
        remote_session_id: None,
    };
    if !save_review_meta(&meta) {
        eprintln!("[review] failed to create review run");
        return;
    }

    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("orch"));
    let log_path = review_dir(&id).join("runner.log");
    let Ok(log) = fs::File::create(&log_path) else {
        eprintln!("[review] failed to create {}", log_path.display());
        return;
    };
    let log2 = log.try_clone().ok();
    let mut cmd = Command::new(exe);
    cmd.args(["review", "run", &id])
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log));
    if let Some(log2) = log2 {
        cmd.stderr(Stdio::from(log2));
    }
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }
    match cmd.spawn() {
        Ok(_) => {
            eprintln!("[review] started {id}");
            eprintln!("[review] backend: {backend}");
            eprintln!("[review] target: {target}");
            eprintln!("[review] result: {}", review_dir(&id).display());
        }
        Err(e) => {
            let mut failed = meta;
            failed.status = "failed".into();
            failed.finished_at = Some(cache::now_epoch());
            failed.exit_code = Some(-1);
            save_review_meta(&failed);
            eprintln!("[review] failed to spawn runner: {e}");
        }
    }
}

fn cmd_review_run(id: &str) {
    let Some(mut meta) = load_review_meta(id) else {
        eprintln!("[review] no review id {id}");
        return;
    };
    match meta.backend.as_deref().unwrap_or(REVIEW_BACKEND_LOCAL) {
        REVIEW_BACKEND_REMOTE => {
            let options = remote_agent::AskOptions::default();
            cmd_review_run_remote(&mut meta, options)
        }
        REVIEW_BACKEND_REMOTE_CLAUDE | "claude" => {
            let options = remote_agent::AskOptions {
                config: default_remote_claude_config(),
                timeout: Duration::from_secs(45 * 60),
            };
            cmd_review_run_remote(&mut meta, options)
        }
        _ => cmd_review_run_local(&mut meta),
    }
}

fn cmd_review_run_local(meta: &mut ReviewMeta) {
    let id = meta.id.clone();
    let dir = review_dir(&id);
    let answer = dir.join("answer.md");
    let events = dir.join("events.jsonl");
    let stderr = dir.join("stderr.log");
    let Ok(stdout) = fs::File::create(&events) else {
        eprintln!("[review] failed to create {}", events.display());
        return;
    };
    let Ok(stderr_file) = fs::File::create(&stderr) else {
        eprintln!("[review] failed to create {}", stderr.display());
        return;
    };

    let status = Command::new("codex")
        .args([
            "exec",
            "review",
            "--disable",
            "hooks",
            "--json",
            "--ephemeral",
        ])
        .arg("--output-last-message")
        .arg(&answer)
        .arg(&meta.target)
        .current_dir(state::expand_home(&meta.git_root))
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr_file))
        .status();

    let (status_name, exit_code) = match status {
        Ok(s) => {
            let has_answer = fs::metadata(&answer).map(|m| m.len() > 0).unwrap_or(false);
            let name = if s.success() && has_answer {
                "ready"
            } else {
                "failed"
            };
            (name.to_string(), s.code().unwrap_or(-1))
        }
        Err(e) => {
            let _ = fs::write(&stderr, format!("failed to run codex: {e}\n"));
            ("failed".into(), -1)
        }
    };
    meta.status = status_name;
    meta.finished_at = Some(cache::now_epoch());
    meta.exit_code = Some(exit_code);
    save_review_meta(meta);
}

fn cmd_review_run_remote(meta: &mut ReviewMeta, options: remote_agent::AskOptions) {
    let dir = review_dir(&meta.id);
    let answer = dir.join("answer.md");
    let events = dir.join("events.jsonl");
    let stderr = dir.join("stderr.log");
    let prompt = remote_review_prompt(meta);
    let client = remote_agent::RemoteAgentClient::from_env();

    let result = (|| {
        let session = client.create_session(&options.config)?;
        meta.remote_session_id = Some(session.session_id.clone());
        save_review_meta(meta);
        client.ask_existing_session(&session.session_id, &prompt, options.timeout)
    })();

    match result {
        Ok(result) => {
            let _ = fs::write(&answer, result.answer);
            let _ = fs::write(&events, result.transcript_jsonl);
            meta.remote_session_id = Some(result.session_id);
            meta.status = "ready".into();
            meta.exit_code = Some(0);
        }
        Err(e) => {
            let _ = fs::write(&stderr, format!("{e}\n"));
            if let Some(session_id) = &meta.remote_session_id {
                if let Ok(transcript) = client.transcript_jsonl(session_id) {
                    let _ = fs::write(&events, transcript);
                }
            }
            meta.status = "failed".into();
            meta.exit_code = Some(-1);
        }
    }
    meta.finished_at = Some(cache::now_epoch());
    save_review_meta(meta);
}

fn remote_review_prompt(meta: &ReviewMeta) -> String {
    format!(
        "You are running an orch background PR review.\n\n\
Target:\n\
{}\n\n\
Task:\n\
- Fetch or check out the PR/branch as needed.\n\
- Review for correctness bugs, regressions, missing tests, and risky behavior.\n\
- Do not edit files or commit.\n\
- Return findings first, ordered by severity.\n\
- Include file/line references where possible.\n\
- If there are no findings, say that clearly and mention residual risk or test gaps.\n",
        meta.target,
    )
}

fn default_remote_claude_config() -> remote_agent::AgentConfig {
    remote_agent::AgentConfig::claude_code(
        remote_agent::DEFAULT_CLAUDE_MODEL,
        remote_agent::DEFAULT_EFFORT,
    )
}

fn remote_agent_config(
    claude: bool,
    model: Option<&str>,
    effort: Option<&str>,
) -> remote_agent::AgentConfig {
    let effort = effort.unwrap_or(remote_agent::DEFAULT_EFFORT);
    if claude {
        let model = model.unwrap_or(remote_agent::DEFAULT_CLAUDE_MODEL);
        remote_agent::AgentConfig::claude_code(model, effort)
    } else {
        let model = model.unwrap_or(remote_agent::DEFAULT_MODEL);
        remote_agent::AgentConfig::codex(model, effort)
    }
}

fn cmd_remote_start(claude: bool, model: Option<&str>, effort: Option<&str>) {
    let client = remote_agent::RemoteAgentClient::from_env();
    let config = remote_agent_config(claude, model, effort);
    match client.create_session(&config) {
        Ok(session) => println!("{}", session.session_id),
        Err(e) => {
            eprintln!("[remote] failed to create session: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_remote_ask(
    prompt_parts: &[String],
    session: Option<&str>,
    claude: bool,
    model: Option<&str>,
    effort: Option<&str>,
    timeout_minutes: u64,
) {
    let prompt = prompt_parts.join(" ");
    if prompt.trim().is_empty() {
        eprintln!("[remote] prompt is required");
        std::process::exit(2);
    }

    let client = remote_agent::RemoteAgentClient::from_env();
    let timeout = Duration::from_secs(timeout_minutes.saturating_mul(60));
    let (session_id, result) = if let Some(session_id) = session {
        (
            session_id.to_string(),
            client.ask_existing_session(session_id, &prompt, timeout),
        )
    } else {
        let config = remote_agent_config(claude, model, effort);
        match client.create_session(&config) {
            Ok(session) => {
                eprintln!("[remote] session: {}", session.session_id);
                let result =
                    client.ask_existing_session(&session.session_id, &prompt, timeout);
                (session.session_id, result)
            }
            Err(e) => {
                eprintln!("[remote] failed to create session: {e}");
                std::process::exit(1);
            }
        }
    };

    match result {
        Ok(result) => println!("{}", result.answer),
        Err(e) => {
            eprintln!("[remote] session: {session_id}");
            eprintln!("[remote] failed: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_remote_list(limit: u32, source: Option<&str>, json: bool) {
    let client = remote_agent::RemoteAgentClient::from_env();
    let result = client.list_sessions(limit, source);
    let sessions = match result {
        Ok(response) => response.sessions,
        Err(e) => {
            eprintln!("[remote] failed to list sessions: {e}");
            std::process::exit(1);
        }
    };

    if json {
        match serde_json::to_string_pretty(&sessions) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("[remote] failed to encode sessions: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    if sessions.is_empty() {
        eprintln!("[remote] no sessions");
        return;
    }

    println!(
        "{:<40}  {:<12}  {:<7}  {:<10}  {:<8}  {:<6}  {}",
        "SESSION", "STATUS", "SOURCE", "MODEL", "EFFORT", "AGE", "ACTIVE",
    );
    for session in sessions {
        let config = session.options.agent_config.unwrap_or_default();
        println!(
            "{:<40}  {:<12}  {:<7}  {:<10}  {:<8}  {:<6}  {}",
            session.session_id,
            dash_if_empty(&session.status),
            dash_if_empty(&session.source),
            dash_if_empty(&config.model),
            dash_if_empty(&config.effort_level),
            format_age(session.created_at),
            format_age(session.last_active_at),
        );
    }
}

fn dash_if_empty(s: &str) -> &str {
    if s.trim().is_empty() {
        "-"
    } else {
        s
    }
}

fn format_age(epoch: u64) -> String {
    let epoch = normalized_epoch_secs(epoch);
    if epoch == 0 {
        return "-".to_string();
    }
    let now = cache::now_epoch();
    if epoch > now {
        return "0s".to_string();
    }
    let secs = now - epoch;
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

fn normalized_epoch_secs(epoch: u64) -> u64 {
    if epoch > 1_000_000_000_000 {
        epoch / 1000
    } else {
        epoch
    }
}

fn cmd_review_list() {
    prune_old_reviews();
    let metas = load_review_metas();
    if metas.is_empty() {
        eprintln!("[review] no review runs");
        return;
    }
    for m in metas {
        let consumed = if m.consumed_at.is_some() {
            " consumed"
        } else {
            ""
        };
        eprintln!("{}  {}{}  {}", m.id, m.status, consumed, m.label);
    }
}

fn newest_ready_review_id() -> Option<String> {
    prune_old_reviews();
    load_review_metas()
        .into_iter()
        .find(|m| m.status == "ready" && m.consumed_at.is_none())
        .map(|m| m.id)
}

fn cmd_review_show(id: Option<&str>, consume: bool) {
    prune_old_reviews();
    let id = match id.map(String::from).or_else(newest_ready_review_id) {
        Some(id) => id,
        None => {
            eprintln!("[review] no ready unconsumed review");
            return;
        }
    };
    let Some(mut meta) = load_review_meta(&id) else {
        eprintln!("[review] no review id {id}");
        return;
    };
    let answer = review_dir(&id).join("answer.md");
    match fs::read_to_string(&answer) {
        Ok(s) if !s.trim().is_empty() => {
            println!("{s}");
            if consume && meta.consumed_at.is_none() {
                meta.consumed_at = Some(cache::now_epoch());
                save_review_meta(&meta);
            }
        }
        _ => {
            eprintln!("[review] no answer for {id}");
            eprintln!(
                "[review] stderr: {}",
                review_dir(&id).join("stderr.log").display()
            );
        }
    }
}

fn hook_input_value() -> serde_json::Value {
    serde_json::from_str(&read_stdin_to_string()).unwrap_or_default()
}

fn cwd_matches_review(hook_cwd: &str, meta: &ReviewMeta) -> bool {
    let cwd = state::expand_home(hook_cwd);
    let root = state::expand_home(&meta.git_root);
    cwd == root || cwd.starts_with(&format!("{}/", root.trim_end_matches('/')))
}

fn pending_review_for_hook(hook_cwd: &str) -> Option<ReviewMeta> {
    load_review_metas().into_iter().find(|m| {
        m.status == "ready"
            && m.consumed_at.is_none()
            && cwd_matches_review(hook_cwd, m)
    })
}

fn review_hook_message(meta: &ReviewMeta) -> String {
    format!(
        "A background PR review is ready: {} ({})\n\
Run `orch review show {} --consume`, present the findings first, \
and state whether you agree, disagree, or need to inspect further.",
        meta.label, meta.target, meta.id,
    )
}

fn cmd_review_hook() {
    prune_old_reviews();
    let input = hook_input_value();
    let event = input["hook_event_name"].as_str().unwrap_or("");
    let cwd = input["cwd"].as_str().unwrap_or("");
    let Some(mut meta) = pending_review_for_hook(cwd) else {
        return;
    };
    let message = review_hook_message(&meta);

    if event == "UserPromptSubmit" {
        let out = serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "UserPromptSubmit",
                "additionalContext": message,
            },
        });
        println!("{out}");
        return;
    }

    if event == "Stop"
        && !input["stop_hook_active"].as_bool().unwrap_or(false)
        && meta.stop_notified_at.is_none()
    {
        meta.stop_notified_at = Some(cache::now_epoch());
        save_review_meta(&meta);
        let out = serde_json::json!({
            "decision": "block",
            "reason": message,
        });
        println!("{out}");
    }
}

// Linear ticket linkage.

fn linear_key_pattern() -> regex::Regex {
    // PROJ-N or PROJ-NN... — Linear's ticket format. Case-insensitive
    // because branches commonly use lowercase (`azhou/eng-29592-...`);
    // matches are uppercased before storage.
    regex::Regex::new(r"(?i)\b[A-Z][A-Z0-9_]+-\d+\b").expect("valid regex")
}

fn cmd_linear_add(task: &str, key: &str) {
    let s = store::Store::default();
    let Some(mut record) = s.load_record_by_slug(task) else {
        eprintln!("[linear] no task: {task}");
        return;
    };
    if record.links.linear_issues.iter().any(|li| li.key == key) {
        eprintln!("[linear] {key} already linked to {task}");
        return;
    }
    record.links.linear_issues.push(store::LinearLink {
        key: key.to_string(),
        source: store::LinkSource::Manual,
        last_verified_at: None,
    });
    record.updated_at = cache::now_epoch();
    s.save_record(&record);
    eprintln!("[linear] linked {key} to {task}");
}

fn cmd_linear_rm(task: &str, key: &str) {
    let s = store::Store::default();
    let Some(mut record) = s.load_record_by_slug(task) else {
        eprintln!("[linear] no task: {task}");
        return;
    };
    let before = record.links.linear_issues.len();
    record.links.linear_issues.retain(|li| li.key != key);
    if record.links.linear_issues.len() == before {
        eprintln!("[linear] {key} not linked to {task}");
        return;
    }
    record.updated_at = cache::now_epoch();
    s.save_record(&record);
    eprintln!("[linear] unlinked {key} from {task}");
}

fn cmd_linear_ls(task: &str) {
    let s = store::Store::default();
    let Some(record) = s.load_record_by_slug(task) else {
        eprintln!("[linear] no task: {task}");
        return;
    };
    if record.links.linear_issues.is_empty() {
        eprintln!("[linear] {task}: no linked issues");
        return;
    }
    for li in &record.links.linear_issues {
        let src = match li.source {
            store::LinkSource::Manual => "manual",
            store::LinkSource::BranchDiscovery => "branch",
            store::LinkSource::MarkdownScan => "scan",
            store::LinkSource::Migration => "migration",
        };
        eprintln!("  {}  ({src})", li.key);
    }
}

/// Append LinearLinks for any `[A-Z]+-\d+` keys in `texts` that aren't
/// already on the record and aren't in the cache's `not_found` set
/// (those were already tried and Linear said they don't resolve).
/// Returns count added. Caller is responsible for `save_record` if > 0.
fn link_keys_in_record(
    record: &mut store::TaskRecord,
    texts: impl IntoIterator<Item = impl AsRef<str>>,
    source: store::LinkSource,
    not_found: &HashSet<String>,
) -> usize {
    let pattern = linear_key_pattern();
    let mut added = 0;
    for text in texts {
        for m in pattern.find_iter(text.as_ref()) {
            // Linear identifiers are uppercase by convention; lowercase
            // matches (from branch names) are normalized so dedup and
            // not_found checks against the cache work.
            let key = m.as_str().to_uppercase();
            if record.links.linear_issues.iter().any(|li| li.key == key)
                || not_found.contains(&key)
            {
                continue;
            }
            record.links.linear_issues.push(store::LinearLink {
                key,
                source,
                last_verified_at: None,
            });
            added += 1;
        }
    }
    added
}

/// Scan the task's .md file for Linear keys. New links use `MarkdownScan`.
fn scan_task_md_for_keys(
    record: &mut store::TaskRecord,
    not_found: &HashSet<String>,
) -> usize {
    let md = state::tasks_dir().join(format!("{}.md", record.slug));
    let Ok(content) = fs::read_to_string(&md) else {
        return 0;
    };
    link_keys_in_record(
        record,
        [content.as_str()],
        store::LinkSource::MarkdownScan,
        not_found,
    )
}

/// Scan the task's worktree bookmarks for Linear keys. Tries jj first
/// (`bookmark list -r ::@` for the whole stack), falls back to git's
/// current branch. New links use `BranchDiscovery`.
fn scan_worktree_bookmark_for_keys(
    record: &mut store::TaskRecord,
    not_found: &HashSet<String>,
) -> usize {
    let wt = state::expand_home(&record.worktree.path);
    if wt.is_empty() {
        return 0;
    }
    let names = worktree_bookmark_names(&wt);
    if names.is_empty() {
        return 0;
    }
    link_keys_in_record(
        record,
        names.iter().map(String::as_str),
        store::LinkSource::BranchDiscovery,
        not_found,
    )
}

fn worktree_bookmark_names(worktree: &str) -> Vec<String> {
    if let Some(names) = jj_bookmarks(worktree) {
        return names;
    }
    git_branch(worktree).map(|b| vec![b]).unwrap_or_default()
}

fn jj_bookmarks(worktree: &str) -> Option<Vec<String>> {
    if !Path::new(worktree).join(".jj").exists() {
        return None;
    }
    // `trunk()..@` = bookmarks on this worktree's stack, excluding
    // anything reachable from trunk. Without the exclusion, `::@`
    // walks all of main's history and pulls in every bookmark in
    // the repo (e.g. `task-review-25597`, `task-app-triage-2`).
    let out = Command::new("jj")
        .args([
            "bookmark", "list",
            "--repository", worktree,
            "-r", "trunk()..@",
            "-T", r#"name ++ "\n""#,
        ])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let names: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    (!names.is_empty()).then_some(names)
}

fn git_branch(worktree: &str) -> Option<String> {
    let out = Command::new("git")
        .args(["-C", worktree, "branch", "--show-current"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

fn cmd_linear_clean() {
    let s = store::Store::default();
    let cache = cache::read_linear();
    if cache.not_found.is_empty() {
        eprintln!("[linear] no not-found keys to clean");
        return;
    }
    let bad: HashSet<String> = cache.not_found.iter().cloned().collect();
    eprintln!("[linear] cleaning {} not-found key(s)", bad.len());

    let registry = match s.load_registry() {
        Some(r) => r,
        None => {
            eprintln!("[linear] no registry");
            return;
        }
    };
    let mut total_removed = 0;
    let mut tasks_touched = 0;
    for id in registry.open_order.iter().chain(registry.closed_order.iter()) {
        let Some(mut record) = s.load_record(*id) else { continue };
        let before = record.links.linear_issues.len();
        record.links.linear_issues.retain(|li| !bad.contains(&li.key));
        let removed = before - record.links.linear_issues.len();
        if removed > 0 {
            record.updated_at = cache::now_epoch();
            s.save_record(&record);
            tasks_touched += 1;
            total_removed += removed;
            eprintln!("  {}: removed {removed} key(s)", record.slug);
        }
    }
    eprintln!(
        "[linear] cleaned {total_removed} link(s) from {tasks_touched} task(s)"
    );
}

fn cmd_linear_scan(task: Option<&str>) {
    let s = store::Store::default();
    let tasks: Vec<String> = match task {
        Some(t) => vec![t.to_string()],
        None => state::ordered_open_slugs(),
    };
    let not_found: HashSet<String> = cache::read_linear()
        .not_found
        .into_iter()
        .collect();
    let mut total = 0;
    for t in &tasks {
        let Some(mut record) = s.load_record_by_slug(t) else { continue };
        let md_n = scan_task_md_for_keys(&mut record, &not_found);
        let bm_n = scan_worktree_bookmark_for_keys(&mut record, &not_found);
        if md_n > 0 {
            eprintln!("[linear] {t}: linked {md_n} key(s) from md");
            total += md_n;
        }
        if bm_n > 0 {
            eprintln!("[linear] {t}: linked {bm_n} key(s) from bookmark");
            total += bm_n;
        }
        if md_n + bm_n > 0 {
            record.updated_at = cache::now_epoch();
            s.save_record(&record);
        }
    }
    if total == 0 {
        eprintln!("[linear] no new keys found");
    }
}

fn cmd_gc() {
    let task_orphans = find_orphan_worktrees();
    let tmp_worktrees = find_tmp_worktrees();
    if task_orphans.is_empty() && tmp_worktrees.is_empty() {
        eprintln!("[gc] no orphan worktrees");
        return;
    }

    if !task_orphans.is_empty() {
        eprintln!(
            "[gc] found {} orphan task worktree(s):",
            task_orphans.len(),
        );
        for path in &task_orphans {
            eprintln!("  {}", path.display());
        }
    }
    if !tmp_worktrees.is_empty() {
        eprintln!("[gc] found {} tmp PR worktree(s):", tmp_worktrees.len());
        for path in &tmp_worktrees {
            eprintln!("  {}", path.display());
        }
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

    for path in task_orphans {
        match state::remove_worktree(&path) {
            Ok(()) => eprintln!("[gc] removed {}", path.display()),
            Err(e) => eprintln!(
                "[gc] failed to remove {} ({e}); run `git worktree remove --force {}` to override",
                path.display(),
                path.display(),
            ),
        }
    }
    for path in tmp_worktrees {
        match fs::remove_dir_all(&path) {
            Ok(()) => eprintln!("[gc] removed {}", path.display()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => eprintln!("[gc] failed to remove {} ({e})", path.display()),
        }
    }
    prune_worktree_metadata();
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
            let sessions = state::load_tmux_sessions();
            let tasks = state::load_tasks(&sessions, stale_secs);

            let mut cached_tasks = HashMap::new();
            for task in &tasks {
                let session = &task.record.tmux.session_name;
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
                            state::TaskStatus::Error => "error",
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

/// Background thread: refreshes Linear issue data every 2 min.
/// Writes `.orch/cache/linear.json`. Disconnected (no key, network
/// failure) is cached as a flag so the TUI can show it without
/// retrying every render.
fn spawn_linear_loop() {
    use std::thread;
    thread::spawn(|| {
        loop {
            let api_key = match linear::api_key_from_env() {
                Some(k) => k,
                None => {
                    let mut cache = cache::read_linear();
                    cache.disconnected = true;
                    cache.generated_at = cache::now_epoch();
                    cache::write_linear(&cache);
                    std::thread::sleep(Duration::from_secs(120));
                    continue;
                }
            };

            // Auto-scan every open task's md file and worktree bookmarks
            // for Linear keys before fetching, so newly-mentioned or
            // newly-named keys get linked without an explicit
            // `orch linear scan`. Idempotent.
            let store = store::Store::default();
            let mut records = store.load_open_records();
            let not_found: HashSet<String> = cache::read_linear()
                .not_found
                .into_iter()
                .collect();
            for record in records.iter_mut() {
                let md_n = scan_task_md_for_keys(record, &not_found);
                let bm_n = scan_worktree_bookmark_for_keys(record, &not_found);
                if md_n + bm_n > 0 {
                    record.updated_at = cache::now_epoch();
                    store.save_record(record);
                }
            }

            // Collect every distinct linear key across open tasks.
            let mut keys: Vec<String> = Vec::new();
            for record in &records {
                for li in &record.links.linear_issues {
                    if !keys.contains(&li.key) {
                        keys.push(li.key.clone());
                    }
                }
            }

            if keys.is_empty() {
                cache::write_linear(&cache::LinearCache {
                    generated_at: cache::now_epoch(),
                    issues: std::collections::HashMap::new(),
                    not_found: Vec::new(),
                    disconnected: false,
                });
                std::thread::sleep(Duration::from_secs(120));
                continue;
            }

            let mut cached_issues = std::collections::HashMap::new();
            let mut not_found = Vec::new();
            let mut hard_failures = 0u32;
            for key in &keys {
                fetch_into_cache(
                    &api_key,
                    key,
                    &mut cached_issues,
                    &mut not_found,
                    &mut hard_failures,
                );
            }
            // BFS sub-issues as top-level entries up to depth 5.
            let mut depth = 0u32;
            loop {
                let candidates: Vec<String> = cached_issues
                    .values()
                    .flat_map(|c: &cache::CachedLinear| {
                        c.children.iter().map(|ch| ch.identifier.clone())
                    })
                    .filter(|k| {
                        !cached_issues.contains_key(k)
                            && !not_found.contains(k)
                    })
                    .collect();
                if candidates.is_empty() || depth >= 5 {
                    break;
                }
                let mut seen: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                for key in &candidates {
                    if !seen.insert(key.clone()) {
                        continue;
                    }
                    fetch_into_cache(
                        &api_key,
                        key,
                        &mut cached_issues,
                        &mut not_found,
                        &mut hard_failures,
                    );
                }
                depth += 1;
            }
            let disconnected = hard_failures > 0
                && cached_issues.is_empty();
            cache::write_linear(&cache::LinearCache {
                generated_at: cache::now_epoch(),
                issues: cached_issues,
                not_found,
                disconnected,
            });

            std::thread::sleep(Duration::from_secs(120));
        }
    });
}

/// Fetch one Linear key into the cache.
fn fetch_into_cache(
    api_key: &str,
    key: &str,
    cached_issues: &mut std::collections::HashMap<String, cache::CachedLinear>,
    not_found: &mut Vec<String>,
    hard_failures: &mut u32,
) {
    // Single retry rescues 502s.
    let mut last_err: Option<String> = None;
    for attempt in 0..2 {
        match linear::fetch_issue(api_key, key) {
            Ok(Some(issue)) => {
                cached_issues.insert(
                    key.to_string(),
                    cache::CachedLinear::from_issue(&issue),
                );
                return;
            }
            Ok(None) => {
                if !not_found.contains(&key.to_string()) {
                    not_found.push(key.to_string());
                }
                return;
            }
            Err(e) => {
                last_err = Some(e.to_string());
                if attempt == 0 {
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
    }
    *hard_failures += 1;
    if let Some(e) = last_err {
        eprintln!("[orch] linear fetch {key} failed (after retry): {e}");
    }
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
            let store = store::Store::default();
            let all_prs: Vec<u32> = store
                .load_open_records()
                .into_iter()
                .flat_map(|r| {
                    r.links
                        .prs
                        .into_iter()
                        .map(|p| p.number)
                        .collect::<Vec<_>>()
                })
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

    // Idempotent after first success — short-circuits on store.version=v2.
    // Failure exits the daemon.
    let store = store::Store::default();
    match store.migrate_from_legacy(&state::tasks_dir()) {
        Ok(0) => {}
        Ok(n) => eprintln!("[orch] migrated {n} tasks to v2 store"),
        Err(e) => {
            eprintln!("[orch] FATAL: v2 migration failed: {e}");
            std::process::exit(1);
        }
    }

    state::ensure_state_files();
    state::sweep_stale_markers(state::busy_stale_secs());
    eprintln!("[orch] reconciling PRs...");
    state::reconcile_prs();

    // Start background cache loops
    spawn_status_loop();
    spawn_pr_loop();
    spawn_linear_loop();

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

fn run_tui() {
    tui3::run().expect("TUI failed");
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        // Default: TUI if interactive terminal, plain status otherwise.
        // ORCH_TUI=legacy keeps the pre-Phase-3 layout for emergency
        // rollback; default is the new three-pane TUI.
        None => {
            if atty::is(atty::Stream::Stdout) {
                run_tui();
            } else {
                cmd_status();
            }
        }
        Some(Cmd::Tui) => run_tui(),
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
        Some(Cmd::Close { name, keep_worktree }) => cmd_close(&name, keep_worktree),
        Some(Cmd::Busy { action }) => match action {
            BusyAction::Start => cmd_busy_start(),
            BusyAction::Stop => cmd_busy_stop(),
        },
        Some(Cmd::RenderDebug { width, height, tab, focus, select, linear_detail, linear_cursor }) => {
            tui3::render_debug(
                width,
                height,
                &tab,
                &focus,
                select,
                linear_detail.as_deref(),
                linear_cursor.as_deref(),
            )
        }
        Some(Cmd::Review { action }) => match action {
            ReviewAction::Start {
                target,
                label,
                local,
                claude,
            } => {
                cmd_review_start(&target, label.as_deref(), local, claude)
            }
            ReviewAction::Run { id } => cmd_review_run(&id),
            ReviewAction::List => cmd_review_list(),
            ReviewAction::Show { id, consume } => cmd_review_show(id.as_deref(), consume),
            ReviewAction::Hook => cmd_review_hook(),
        },
        Some(Cmd::Remote { action }) => match action {
            RemoteAction::Start {
                claude,
                model,
                effort,
            } => cmd_remote_start(claude, model.as_deref(), effort.as_deref()),
            RemoteAction::Ask {
                prompt,
                session,
                claude,
                model,
                effort,
                timeout_minutes,
            } => cmd_remote_ask(
                &prompt,
                session.as_deref(),
                claude,
                model.as_deref(),
                effort.as_deref(),
                timeout_minutes,
            ),
            RemoteAction::List {
                limit,
                source,
                json,
            } => cmd_remote_list(limit, source.as_deref(), json),
        },
        Some(Cmd::Linear { action }) => match action {
            LinearAction::Add { task, key } => cmd_linear_add(&task, &key),
            LinearAction::Rm { task, key } => cmd_linear_rm(&task, &key),
            LinearAction::Scan { task } => cmd_linear_scan(task.as_deref()),
            LinearAction::Ls { task } => cmd_linear_ls(&task),
            LinearAction::Clean => cmd_linear_clean(),
        },
        Some(Cmd::Pr { action }) => match action {
            PrAction::Add { task, number } => {
                let store = store::Store::default();
                let already = store
                    .load_record_by_slug(&task)
                    .map(|r| r.links.prs.iter().any(|p| p.number == number))
                    .unwrap_or(false);
                if already {
                    eprintln!("[orch] PR #{number} already in {task}");
                    return;
                }
                let now = cache::now_epoch();
                let updated = store.update_record_by_slug(&task, |r| {
                    r.links.prs.push(store::PrLink {
                        number,
                        source: store::LinkSource::Manual,
                        ..Default::default()
                    });
                    r.updated_at = now;
                });
                if updated {
                    eprintln!("[orch] added PR #{number} to {task}");
                } else {
                    eprintln!("[orch] no task '{task}'");
                }
            }
            PrAction::Rm { task, number } => {
                let store = store::Store::default();
                let now = cache::now_epoch();
                let updated = store.update_record_by_slug(&task, |r| {
                    r.links.prs.retain(|p| p.number != number);
                    r.updated_at = now;
                });
                if updated {
                    eprintln!("[orch] removed PR #{number} from {task}");
                } else {
                    eprintln!("[orch] no task '{task}'");
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_tmp_review_worktree_matches_pr_review_dirs() {
        assert!(is_tmp_review_worktree(Path::new("/private/tmp/repo-pr29145")));
        assert!(is_tmp_review_worktree(Path::new("/private/tmp/repo-pr-29145")));
        assert!(is_tmp_review_worktree(Path::new("/private/tmp/pr29150")));
        assert!(is_tmp_review_worktree(Path::new("/private/tmp/pr29150.bamegW")));
        assert!(is_tmp_review_worktree(Path::new("/private/tmp/pr29150-review")));
        assert!(is_tmp_review_worktree(Path::new("/private/tmp/pr29150-worktree")));
        assert!(!is_tmp_review_worktree(Path::new("/private/tmp/project")));
        assert!(!is_tmp_review_worktree(Path::new("/tmp/pr29150")));
    }

    #[test]
    fn parse_busy_input_extracts_session_and_cwd() {
        let json = r#"{"session_id":"abc-123","cwd":"/tmp/wt"}"#;
        let (sid, cwd) = parse_busy_input(json).unwrap();
        assert_eq!(sid, "abc-123");
        assert_eq!(cwd, "/tmp/wt");
    }

    #[test]
    fn parse_busy_input_allows_missing_cwd() {
        let json = r#"{"session_id":"abc-123"}"#;
        let (sid, cwd) = parse_busy_input(json).unwrap();
        assert_eq!(sid, "abc-123");
        assert_eq!(cwd, "");
    }

    #[test]
    fn parse_busy_input_rejects_bad_json() {
        assert!(parse_busy_input("not json").is_none());
        assert!(parse_busy_input("").is_none());
    }

    #[test]
    fn parse_busy_input_rejects_missing_session_id() {
        assert!(parse_busy_input(r#"{"cwd":"/tmp"}"#).is_none());
        assert!(parse_busy_input(r#"{"session_id":""}"#).is_none());
    }

    #[test]
    fn remote_review_prompt_is_review_only() {
        let meta = ReviewMeta {
            id: "review-1".to_string(),
            target: "https://github.com/example/repo/pull/123".to_string(),
            label: "123".to_string(),
            cwd: "/tmp/wt".to_string(),
            git_root: "/tmp/wt".to_string(),
            status: "running".to_string(),
            started_at: 1,
            finished_at: None,
            exit_code: None,
            consumed_at: None,
            stop_notified_at: None,
            backend: Some("remote".to_string()),
            remote_session_id: None,
        };
        let prompt = remote_review_prompt(&meta);
        assert!(prompt.contains("https://github.com/example/repo/pull/123"));
        assert!(prompt.contains("Do not edit files or commit."));
        assert!(prompt.contains("Return findings first"));
    }

    #[test]
    fn review_backend_selects_remote_claude_for_claude_flag() {
        assert_eq!(review_backend(false, false), REVIEW_BACKEND_REMOTE);
        assert_eq!(review_backend(true, false), REVIEW_BACKEND_LOCAL);
        assert_eq!(review_backend(false, true), REVIEW_BACKEND_REMOTE_CLAUDE);
    }

    #[test]
    fn remote_agent_config_selects_agent_defaults() {
        let codex = remote_agent_config(false, None, None);
        assert_eq!(codex.agent, remote_agent::DEFAULT_AGENT);
        assert_eq!(codex.model, remote_agent::DEFAULT_MODEL);
        assert_eq!(codex.effort_level, remote_agent::DEFAULT_EFFORT);

        let claude = remote_agent_config(true, None, None);
        assert_eq!(claude.agent, remote_agent::DEFAULT_CLAUDE_AGENT);
        assert_eq!(claude.model, remote_agent::DEFAULT_CLAUDE_MODEL);
        assert_eq!(claude.effort_level, remote_agent::DEFAULT_EFFORT);
    }

    #[test]
    fn write_and_remove_busy_marker_round_trip() {
        let dir = std::env::temp_dir().join("orch-busy-marker-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        assert!(write_busy_marker(&dir, "sid-1", "/tmp/wt"));
        let path = dir.join("sid-1");
        assert!(path.exists());

        let content = fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["cwd"], "/tmp/wt");
        assert!(v["started_at"].is_number());

        // No leftover .tmp file
        assert!(!dir.join("sid-1.tmp").exists());

        // Removal: rebuild path then remove
        let _ = fs::remove_file(&path);
        assert!(!path.exists());

        let _ = fs::remove_dir_all(&dir);
    }
}
