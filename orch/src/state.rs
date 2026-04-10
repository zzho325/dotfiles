use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

// Task state persisted in ~/tasks/.state/<name>.json

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskMeta {
    #[serde(default)]
    pub session: String,
    #[serde(default)]
    pub worktree: String,
    #[serde(default)]
    pub prs: Vec<u32>,
    #[serde(default)]
    pub ready: bool,
    /// Set via `orch - "task-foo: needs input: <question>"`
    #[serde(default)]
    pub needs_input: bool,
}

// Tmux session snapshot

#[derive(Debug, Clone)]
pub struct TmuxSession {
    pub name: String,
    pub activity: u64,
    pub attached: bool,
    /// Whether claude/node is running in the active pane
    pub has_active_process: bool,
}

// PR status from GitHub

#[derive(Debug, Clone, Default)]
pub struct PrData {
    pub number: u32,
    pub title: String,
    pub ci_pass: Option<bool>,     // None = pending, Some(true) = pass
    pub approved: bool,
    pub codex_reviewed: bool,
}

// Derived task state

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Ready,
    Working,
    Input,
    Idle,
    Attached,
}

const IDLE_THRESHOLD_SECS: u64 = 60;

// Combined task view

#[derive(Debug, Clone)]
pub struct Task {
    pub name: String,
    pub meta: TaskMeta,
    pub status: TaskStatus,
    pub prs: Vec<PrData>,
}

// Loading

pub fn tasks_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join("tasks")
}

fn state_dir() -> PathBuf {
    tasks_dir().join(".state")
}

pub fn load_task_names(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<_> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
        .filter_map(|e| {
            e.path()
                .file_stem()
                .and_then(|s| s.to_str().map(String::from))
        })
        .collect();
    names.sort();
    names
}

pub fn load_task_meta(name: &str) -> TaskMeta {
    let path = state_dir().join(format!("{name}.json"));
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn load_tmux_sessions() -> HashMap<String, TmuxSession> {
    let output = Command::new("tmux")
        .args([
            "list-sessions",
            "-F",
            "#{session_name} #{session_activity} #{session_attached}",
        ])
        .stderr(Stdio::null())
        .output()
        .ok();
    let Some(output) = output.filter(|o| o.status.success()) else {
        return HashMap::new();
    };

    // Get active pane commands per session to detect running processes
    let pane_cmds = load_pane_commands();

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, ' ');
            let name = parts.next()?.to_string();
            let activity = parts.next()?.parse().ok()?;
            let attached = parts.next()? != "0";
            let has_active_process = pane_cmds
                .get(&name)
                .is_some_and(|cmd| is_worker_process(cmd));
            Some((
                name.clone(),
                TmuxSession { name, activity, attached, has_active_process },
            ))
        })
        .collect()
}

/// Get the current command running in each session's active pane.
fn load_pane_commands() -> HashMap<String, String> {
    let output = Command::new("tmux")
        .args([
            "list-panes",
            "-a",
            "-F",
            "#{session_name} #{pane_active} #{pane_current_command}",
        ])
        .stderr(Stdio::null())
        .output()
        .ok();
    let Some(output) = output.filter(|o| o.status.success()) else {
        return HashMap::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, ' ');
            let session = parts.next()?;
            let active = parts.next()?;
            let cmd = parts.next()?;
            // Only care about the active pane
            if active == "1" {
                Some((session.to_string(), cmd.to_string()))
            } else {
                None
            }
        })
        .collect()
}

/// Check if the command is a worker process (claude/node), not just a shell.
fn is_worker_process(cmd: &str) -> bool {
    matches!(cmd, "claude" | "node" | "codex")
}

pub fn derive_status(
    meta: &TaskMeta,
    sessions: &HashMap<String, TmuxSession>,
) -> TaskStatus {
    let session = find_session(&meta.session, sessions);

    let Some(session) = session else {
        return TaskStatus::Idle;
    };

    if session.attached {
        return TaskStatus::Attached;
    }

    // Explicit input flag from orch messages
    if meta.needs_input {
        return TaskStatus::Input;
    }

    // Use active process (claude/node running) as primary signal,
    // fall back to activity timestamp
    if session.has_active_process {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let idle_secs = now.saturating_sub(session.activity);
        if idle_secs > IDLE_THRESHOLD_SECS {
            TaskStatus::Ready
        } else {
            TaskStatus::Working
        }
    } else {
        // No worker process running — session is idle
        TaskStatus::Ready
    }
}

fn find_session<'a>(
    session_name: &str,
    sessions: &'a HashMap<String, TmuxSession>,
) -> Option<&'a TmuxSession> {
    // Direct match
    if let Some(s) = sessions.get(session_name) {
        return Some(s);
    }
    // Match by suffix (handles numeric prefix like "1-task-foo")
    sessions.values().find(|s| {
        s.name
            .trim_start_matches(|c: char| c.is_ascii_digit() || c == '-')
            == session_name
    })
}

pub fn load_tasks(
    order: &[String],
    sessions: &HashMap<String, TmuxSession>,
) -> Vec<Task> {
    let dir = tasks_dir();
    let all_names = load_task_names(&dir);

    // Ordered names first, then any remaining
    let mut ordered: Vec<String> = order
        .iter()
        .filter(|n| all_names.contains(n))
        .cloned()
        .collect();
    for name in &all_names {
        if !ordered.contains(name) {
            ordered.push(name.clone());
        }
    }

    ordered
        .into_iter()
        .map(|name| {
            let meta = load_task_meta(&name);
            let status = derive_status(&meta, sessions);
            Task {
                name,
                meta,
                status,
                prs: Vec::new(), // Filled by gh polling
            }
        })
        .collect()
}

// Order persistence

pub fn load_order() -> Vec<String> {
    let path = state_dir().join("order.json");
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_order(order: &[String]) {
    let dir = state_dir();
    fs::create_dir_all(&dir).ok();
    let json = serde_json::to_string_pretty(order).unwrap_or_default();
    fs::write(dir.join("order.json"), json).ok();
}
