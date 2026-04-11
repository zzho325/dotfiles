use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
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
    pub attached: bool,
    /// Whether claude/node is running in any pane
    pub has_active_process: bool,
    /// Hash of the claude pane's last line (for change detection)
    pub pane_hash: u64,
}

// PR status from GitHub

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum CodexStatus {
    #[default]
    None,      // no codex activity
    Commented, // codex left a review with feedback
    ThumbsUp,  // codex reacted 👍 (no feedback needed)
}

#[derive(Debug, Clone, Default)]
pub struct PrData {
    pub number: u32,
    pub title: String,
    pub ci_pass: Option<bool>,     // None = pending, Some(true) = pass
    pub approved: bool,
    pub codex: CodexStatus,
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
            "#{session_name} #{session_attached}",
        ])
        .stderr(Stdio::null())
        .output()
        .ok();
    let Some(output) = output.filter(|o| o.status.success()) else {
        return HashMap::new();
    };

    let active_sessions = load_active_sessions();
    let pane_hashes = load_pane_hashes();

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, ' ');
            let name = parts.next()?.to_string();
            let attached = parts.next()? != "0";
            let has_active_process =
                active_sessions.contains(&name);
            let pane_hash =
                pane_hashes.get(&name).copied().unwrap_or(0);
            Some((
                name.clone(),
                TmuxSession {
                    name, attached,
                    has_active_process, pane_hash,
                },
            ))
        })
        .collect()
}

/// Check if any pane in each session is running a worker process.
fn load_active_sessions() -> HashSet<String> {
    let output = Command::new("tmux")
        .args([
            "list-panes",
            "-a",
            "-F",
            "#{session_name} #{pane_current_command}",
        ])
        .stderr(Stdio::null())
        .output()
        .ok();
    let Some(output) = output.filter(|o| o.status.success()) else {
        return HashSet::new();
    };
    let mut active = HashSet::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some((session, cmd)) = line.split_once(' ') else {
            continue;
        };
        if is_worker_process(cmd) {
            active.insert(session.to_string());
        }
    }
    active
}

fn is_worker_process(cmd: &str) -> bool {
    cmd == "claude" || cmd == "node" || cmd.starts_with("codex")
}

/// Hash the last few lines of each claude pane for change detection.
fn load_pane_hashes() -> HashMap<String, u64> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let output = Command::new("tmux")
        .args([
            "list-panes", "-a", "-F",
            "#{session_name} #{pane_current_command} #{pane_id}",
        ])
        .stderr(Stdio::null())
        .output()
        .ok();
    let Some(output) = output.filter(|o| o.status.success()) else {
        return HashMap::new();
    };

    let mut hashes = HashMap::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let parts: Vec<&str> = line.splitn(3, ' ').collect();
        if parts.len() < 3 {
            continue;
        }
        let (session, cmd, pane_id) = (parts[0], parts[1], parts[2]);
        if !is_worker_process(cmd) {
            continue;
        }
        // Capture last 3 lines of this pane
        let capture = Command::new("tmux")
            .args(["capture-pane", "-t", pane_id, "-p", "-S", "-3"])
            .stderr(Stdio::null())
            .output()
            .ok();
        if let Some(cap) = capture.filter(|o| o.status.success()) {
            let text = String::from_utf8_lossy(&cap.stdout);
            let mut hasher = DefaultHasher::new();
            text.trim().hash(&mut hasher);
            hashes.insert(session.to_string(), hasher.finish());
        }
    }
    hashes
}

pub fn derive_status(
    meta: &TaskMeta,
    sessions: &HashMap<String, TmuxSession>,
    prev_hashes: &HashMap<String, u64>,
) -> TaskStatus {
    let session = find_session(&meta.session, sessions);

    let Some(session) = session else {
        return TaskStatus::Idle;
    };

    if session.attached {
        return TaskStatus::Attached;
    }

    if meta.needs_input {
        return TaskStatus::Input;
    }

    if !session.has_active_process {
        return TaskStatus::Ready;
    }

    // Compare pane content hash with previous poll.
    // If the hash changed, the worker is actively producing output.
    let prev = prev_hashes.get(&session.name).copied().unwrap_or(0);
    if prev != 0 && prev == session.pane_hash {
        // Content hasn't changed since last poll → ready
        TaskStatus::Ready
    } else {
        TaskStatus::Working
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
    prev_hashes: &HashMap<String, u64>,
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
            let status = derive_status(&meta, sessions, prev_hashes);
            Task {
                name,
                meta,
                status,
                prs: Vec::new(), // Filled by gh polling
            }
        })
        .collect()
}

pub fn save_task_meta(name: &str, meta: &TaskMeta) {
    let dir = state_dir();
    fs::create_dir_all(&dir).ok();
    if let Ok(json) = serde_json::to_string_pretty(meta) {
        let path = dir.join(format!("{name}.json"));
        let tmp = path.with_extension("tmp");
        if fs::write(&tmp, &json).is_ok() {
            let _ = fs::rename(&tmp, &path);
        }
    }
}

/// Reconcile PR state: discover open PRs via jj op log and prune
/// closed/merged ones. Called by the daemon periodically.
pub fn reconcile_prs() {
    let repo_dir = match std::env::var("ORCH_REPO") {
        Ok(r) => format!("{r}/main"),
        Err(_) => return,
    };

    // 1. Get all open PRs by me: branch → PR numbers
    let output = Command::new("gh")
        .args([
            "pr", "list", "--author", "@me", "--state", "open",
            "--json", "number,headRefName",
        ])
        .current_dir(&repo_dir)
        .stderr(Stdio::null())
        .output()
        .ok();

    let Some(output) = output.filter(|o| o.status.success()) else {
        return;
    };
    let Ok(prs) =
        serde_json::from_slice::<Vec<serde_json::Value>>(&output.stdout)
    else {
        return;
    };

    let mut branch_to_prs: HashMap<String, Vec<u32>> = HashMap::new();
    for pr in &prs {
        let branch = pr["headRefName"].as_str().unwrap_or("");
        let number = pr["number"].as_u64().unwrap_or(0) as u32;
        if !branch.is_empty() && number > 0 {
            branch_to_prs
                .entry(branch.to_string())
                .or_default()
                .push(number);
        }
    }

    // 2. For each task, check which bookmarks were pushed from its
    //    worktree via jj op log
    let dir = tasks_dir();
    let home = dirs::home_dir().unwrap_or_default();

    for name in load_task_names(&dir) {
        let mut meta = load_task_meta(&name);
        let worktree = meta
            .worktree
            .replace("~", &home.to_string_lossy());

        if worktree.is_empty() {
            continue;
        }

        let wt_path = Path::new(&worktree);
        if !wt_path.join(".jj").exists() {
            continue;
        }

        // Get bookmarks pushed from this workspace
        let op_output = Command::new("jj")
            .args([
                "op", "log",
                "--repository", &worktree,
                "--no-graph",
                "-T", "description ++ \"\\n\"",
            ])
            .stderr(Stdio::null())
            .output()
            .ok();

        let pushed_branches: HashSet<String> = op_output
            .filter(|o| o.status.success())
            .map(|o| {
                let mut branches = HashSet::new();
                for line in String::from_utf8_lossy(&o.stdout).lines() {
                    // "push bookmark X to git remote origin"
                    if let Some(rest) = line.strip_prefix("push bookmark ") {
                        if let Some(name) = rest.split(" to git remote").next() {
                            branches.insert(name.trim().to_string());
                        }
                    }
                    // "push bookmarks X, Y, Z to git remote origin"
                    if let Some(rest) = line.strip_prefix("push bookmarks ") {
                        if let Some(names) = rest.split(" to git remote").next() {
                            for name in names.split(", ") {
                                branches.insert(name.trim().to_string());
                            }
                        }
                    }
                }
                branches
            })
            .unwrap_or_default();

        // Match pushed branches against open PRs
        let mut task_prs: Vec<u32> = Vec::new();
        for branch in &pushed_branches {
            if let Some(nums) = branch_to_prs.get(branch) {
                task_prs.extend(nums);
            }
        }
        task_prs.sort();
        task_prs.dedup();

        if meta.prs != task_prs {
            meta.prs = task_prs;
            save_task_meta(&name, &meta);
        }
    }
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
