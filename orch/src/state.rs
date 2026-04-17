use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde::{Deserialize, Serialize};

/// Atomic write: write to .tmp then rename to avoid partial reads.
pub fn atomic_write(path: &Path, content: &str) -> bool {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content).is_ok()
        && fs::rename(&tmp, path).is_ok()
}

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

    let (active_sessions, pane_hashes) = load_pane_info();

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

fn is_worker_process(cmd: &str) -> bool {
    cmd == "claude" || cmd == "node" || cmd.starts_with("codex")
}

/// Check if a tmux session name matches an expected name,
/// accounting for numeric prefixes (e.g. "3-task-foo" matches
/// "task-foo").
pub fn session_matches(actual: &str, expected: &str) -> bool {
    actual == expected
        || actual.ends_with(&format!("-{expected}"))
}

/// Single tmux list-panes call to detect active workers and hash
/// pane content for change detection.
fn load_pane_info() -> (HashSet<String>, HashMap<String, u64>) {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let output = Command::new("tmux")
        .args([
            "list-panes", "-a", "-F",
            "#{session_name} #{pane_current_command} #{pane_id} \
             #{cursor_x} #{cursor_y}",
        ])
        .stderr(Stdio::null())
        .output()
        .ok();
    let Some(output) = output.filter(|o| o.status.success()) else {
        return (HashSet::new(), HashMap::new());
    };

    let mut active = HashSet::new();
    let mut hashes = HashMap::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let parts: Vec<&str> = line.splitn(5, ' ').collect();
        if parts.len() < 5 {
            continue;
        }
        let (session, cmd, pane_id, cx, cy) =
            (parts[0], parts[1], parts[2], parts[3], parts[4]);
        if !is_worker_process(cmd) {
            continue;
        }
        active.insert(session.to_string());

        // Capture visible pane content (full screen) + cursor.
        // Full capture catches streaming token changes; cursor
        // position catches typing/prompt movement even when
        // content is static (reduce motion).
        let capture = Command::new("tmux")
            .args(["capture-pane", "-t", pane_id, "-p"])
            .stderr(Stdio::null())
            .output()
            .ok();
        if let Some(cap) = capture.filter(|o| o.status.success()) {
            let text = String::from_utf8_lossy(&cap.stdout);
            let mut hasher = DefaultHasher::new();
            text.trim().hash(&mut hasher);
            cx.hash(&mut hasher);
            cy.hash(&mut hasher);
            hashes.insert(session.to_string(), hasher.finish());
        }
    }
    (active, hashes)
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

/// Merge stored order with current task names: ordered first,
/// then remaining alphabetically.
pub fn ordered_task_names(order: &[String]) -> Vec<String> {
    let dir = tasks_dir();
    let all_names = load_task_names(&dir);
    let mut result: Vec<String> = order
        .iter()
        .filter(|n| all_names.contains(n))
        .cloned()
        .collect();
    for name in &all_names {
        if !result.contains(name) {
            result.push(name.clone());
        }
    }
    result
}

pub fn load_tasks(
    order: &[String],
    sessions: &HashMap<String, TmuxSession>,
    prev_hashes: &HashMap<String, u64>,
) -> Vec<Task> {
    let ordered = ordered_task_names(order);

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

/// Ensure every task .md file has a corresponding state file
/// with session and worktree populated (by convention).
pub fn ensure_state_files() {
    let dir = tasks_dir();
    let repo = std::env::var("ORCH_REPO").ok();
    for name in load_task_names(&dir) {
        let mut meta = load_task_meta(&name);
        let mut changed = false;
        if meta.session.is_empty() {
            meta.session = format!("task-{name}");
            changed = true;
        }
        if meta.worktree.is_empty() {
            if let Some(ref r) = repo {
                let wt = format!("{r}/task-{name}");
                if Path::new(&wt).exists() {
                    meta.worktree = wt;
                    changed = true;
                }
            }
        }
        if changed {
            save_task_meta(&name, &meta);
        }
    }
}

pub fn save_task_meta(name: &str, meta: &TaskMeta) {
    let dir = state_dir();
    fs::create_dir_all(&dir).ok();
    if let Ok(json) = serde_json::to_string_pretty(meta) {
        let path = dir.join(format!("{name}.json"));
        atomic_write(&path, &json);
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
        if pushed_branches.is_empty() {
            continue;
        }
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
    atomic_write(&dir.join("order.json"), &json);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_creates_file() {
        let dir = std::env::temp_dir().join("orch-test-atomic");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.json");

        assert!(atomic_write(&path, r#"{"ok": true}"#));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            r#"{"ok": true}"#,
        );
        // No leftover .tmp
        assert!(!path.with_extension("tmp").exists());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn derive_status_no_session() {
        let meta = TaskMeta::default();
        let sessions = HashMap::new();
        let prev = HashMap::new();
        assert_eq!(
            derive_status(&meta, &sessions, &prev),
            TaskStatus::Idle,
        );
    }

    #[test]
    fn derive_status_attached() {
        let meta = TaskMeta {
            session: "task-foo".into(),
            ..Default::default()
        };
        let mut sessions = HashMap::new();
        sessions.insert(
            "task-foo".into(),
            TmuxSession {
                name: "task-foo".into(),
                attached: true,
                has_active_process: true,
                pane_hash: 123,
            },
        );
        assert_eq!(
            derive_status(&meta, &sessions, &HashMap::new()),
            TaskStatus::Attached,
        );
    }

    #[test]
    fn derive_status_needs_input() {
        let meta = TaskMeta {
            session: "task-foo".into(),
            needs_input: true,
            ..Default::default()
        };
        let mut sessions = HashMap::new();
        sessions.insert(
            "task-foo".into(),
            TmuxSession {
                name: "task-foo".into(),
                attached: false,
                has_active_process: true,
                pane_hash: 123,
            },
        );
        assert_eq!(
            derive_status(&meta, &sessions, &HashMap::new()),
            TaskStatus::Input,
        );
    }

    #[test]
    fn derive_status_no_active_process_is_ready() {
        let meta = TaskMeta {
            session: "task-foo".into(),
            ..Default::default()
        };
        let mut sessions = HashMap::new();
        sessions.insert(
            "task-foo".into(),
            TmuxSession {
                name: "task-foo".into(),
                attached: false,
                has_active_process: false,
                pane_hash: 0,
            },
        );
        assert_eq!(
            derive_status(&meta, &sessions, &HashMap::new()),
            TaskStatus::Ready,
        );
    }

    #[test]
    fn derive_status_working_when_hash_changed() {
        let meta = TaskMeta {
            session: "task-foo".into(),
            ..Default::default()
        };
        let mut sessions = HashMap::new();
        sessions.insert(
            "task-foo".into(),
            TmuxSession {
                name: "task-foo".into(),
                attached: false,
                has_active_process: true,
                pane_hash: 999,
            },
        );
        let mut prev = HashMap::new();
        prev.insert("task-foo".to_string(), 111u64);
        assert_eq!(
            derive_status(&meta, &sessions, &prev),
            TaskStatus::Working,
        );
    }

    #[test]
    fn derive_status_ready_when_hash_unchanged() {
        let meta = TaskMeta {
            session: "task-foo".into(),
            ..Default::default()
        };
        let mut sessions = HashMap::new();
        sessions.insert(
            "task-foo".into(),
            TmuxSession {
                name: "task-foo".into(),
                attached: false,
                has_active_process: true,
                pane_hash: 555,
            },
        );
        let mut prev = HashMap::new();
        prev.insert("task-foo".to_string(), 555u64);
        assert_eq!(
            derive_status(&meta, &sessions, &prev),
            TaskStatus::Ready,
        );
    }

    #[test]
    fn find_session_by_suffix() {
        let mut sessions = HashMap::new();
        sessions.insert(
            "1-task-foo".into(),
            TmuxSession {
                name: "1-task-foo".into(),
                attached: false,
                has_active_process: false,
                pane_hash: 0,
            },
        );
        // Direct match fails, suffix match succeeds
        assert!(sessions.get("task-foo").is_none());
        let found = find_session("task-foo", &sessions);
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "1-task-foo");
    }

    #[test]
    fn task_meta_round_trip() {
        let meta = TaskMeta {
            session: "task-test".into(),
            worktree: "~/code/wt".into(),
            prs: vec![100, 200],
            needs_input: true,
        };
        let json = serde_json::to_string(&meta).unwrap();
        let parsed: TaskMeta =
            serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.session, "task-test");
        assert_eq!(parsed.prs, vec![100, 200]);
        assert!(parsed.needs_input);
    }

    #[test]
    fn save_load_bootstraps_state() {
        let dir = std::env::temp_dir().join("orch-test-state");
        fs::create_dir_all(&dir).unwrap();

        // Simulate what cmd_spawn does: load (missing), fill, save
        let name = "test-spawn-task";
        let path = dir.join(format!("{name}.json"));
        // Ensure clean state
        let _ = fs::remove_file(&path);

        // Load returns default when file doesn't exist
        let meta = TaskMeta::default();
        assert!(meta.session.is_empty());

        // Fill and save
        let meta = TaskMeta {
            session: "task-test".into(),
            worktree: "/tmp/wt".into(),
            ..Default::default()
        };
        let json = serde_json::to_string_pretty(&meta).unwrap();
        atomic_write(&path, &json);

        // Load back
        let content = fs::read_to_string(&path).unwrap();
        let loaded: TaskMeta =
            serde_json::from_str(&content).unwrap();
        assert_eq!(loaded.session, "task-test");
        assert_eq!(loaded.worktree, "/tmp/wt");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn is_worker_process_checks() {
        assert!(is_worker_process("claude"));
        assert!(is_worker_process("node"));
        assert!(is_worker_process("codex"));
        assert!(is_worker_process("codex-cli"));
        assert!(!is_worker_process("nvim"));
        assert!(!is_worker_process("zsh"));
    }
}
