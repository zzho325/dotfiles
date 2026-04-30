use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::SystemTime,
};

use serde::{Deserialize, Serialize};

/// Default age (seconds) past which a busy marker is considered stale.
/// Tunable via `ORCH_BUSY_STALE_SECS` env var. 30 minutes is comfortably
/// larger than the longest Claude turn we'd expect in practice.
pub const DEFAULT_BUSY_STALE_SECS: u64 = 1800;

pub fn busy_stale_secs() -> u64 {
    std::env::var("ORCH_BUSY_STALE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_BUSY_STALE_SECS)
}

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
    /// User parked this task; do not auto-spawn.
    #[serde(default)]
    pub paused: bool,
}

// Tmux session snapshot

#[derive(Debug, Clone)]
pub struct TmuxSession {
    pub name: String,
    pub attached: bool,
    /// Whether claude/node is running in any pane
    pub has_active_process: bool,
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
    Paused,
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
    // v2 store dispatch: when `store.version=v2` exists, read from the
    // new TaskRecord layer and flatten to TaskMeta. The legacy reader
    // is the fallback for transitional cases (record missing for a slug
    // that still has a .state/<name>.json on disk).
    let store = crate::store::Store::default();
    if store.is_authoritative() {
        if let Some(record) = store.load_record_by_slug(name) {
            return TaskMeta::from_record(&record);
        }
    }
    let path = state_dir().join(format!("{name}.json"));
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

impl TaskMeta {
    /// Flatten a v2 `TaskRecord` into the legacy `TaskMeta` shape that
    /// existing read sites expect. Lossy by design — fields not in
    /// TaskMeta (drift flags, agent mode, link provenance) are dropped.
    /// Only used during the slice B/C transition; once all consumers
    /// migrate to TaskRecord directly, this conversion goes away.
    pub fn from_record(r: &crate::store::TaskRecord) -> Self {
        Self {
            session: r.tmux.session_name.clone(),
            worktree: r.worktree.path.clone(),
            prs: r.links.prs.iter().map(|p| p.number).collect(),
            needs_input: r.attention.needs_input,
            paused: r.desired_state == crate::store::DesiredState::Paused,
        }
    }
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

    let active_sessions = load_pane_info();

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, ' ');
            let name = parts.next()?.to_string();
            let attached = parts.next()? != "0";
            let has_active_process =
                active_sessions.contains(&name);
            Some((
                name.clone(),
                TmuxSession {
                    name,
                    attached,
                    has_active_process,
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

/// Strip the leading `N-` numeric prefix from a tmux session name
/// (e.g. "3-task-foo" → "task-foo", "task-foo" → "task-foo").
pub fn strip_numeric_prefix(name: &str) -> &str {
    name.trim_start_matches(|c: char| c.is_ascii_digit() || c == '-')
}

/// Single tmux list-panes call to detect which sessions have an active
/// worker process. No more pane-content hashing — busy state comes from
/// the marker file written by Claude Code's UserPromptSubmit hook.
fn load_pane_info() -> HashSet<String> {
    let output = Command::new("tmux")
        .args([
            "list-panes", "-a", "-F",
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
        let (session, cmd) = match line.split_once(' ') {
            Some(parts) => parts,
            None => continue,
        };
        if is_worker_process(cmd) {
            active.insert(session.to_string());
        }
    }
    active
}

// Busy-marker reading. Markers are written by Claude Code hooks
// (UserPromptSubmit) and removed by Stop/SessionEnd hooks. See
// docs/busy-detection-plan.md.

fn busy_dir() -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    runtime.join("orch").join("busy")
}

/// Expand `~` in a path against the user's home directory.
fn expand_home(p: &str) -> String {
    let home = dirs::home_dir().unwrap_or_default();
    p.replace("~", &home.to_string_lossy())
}

/// True if `marker_cwd` is the same as or nested inside `worktree`.
fn marker_in_worktree(marker_cwd: &str, worktree: &str) -> bool {
    let a = expand_home(marker_cwd);
    let b = expand_home(worktree);
    let b = b.trim_end_matches('/');
    if b.is_empty() {
        return false;
    }
    a == b || a.starts_with(&format!("{b}/"))
}

/// True if any fresh busy marker reports a `cwd` inside `worktree`.
/// "Fresh" = mtime within `stale_secs`. Returns false if `worktree` is
/// empty (no recorded path), the busy dir doesn't exist, or no markers
/// match.
pub fn is_worktree_busy(worktree: &str, stale_secs: u64) -> bool {
    if worktree.is_empty() {
        return false;
    }
    let Ok(entries) = fs::read_dir(busy_dir()) else {
        return false;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue; };
        let Ok(modified) = meta.modified() else { continue; };
        let age = now
            .duration_since(modified)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if age >= stale_secs {
            continue;
        }
        let Ok(content) = fs::read_to_string(entry.path()) else { continue; };
        let Ok(parsed) =
            serde_json::from_str::<serde_json::Value>(&content) else { continue; };
        let Some(cwd) = parsed["cwd"].as_str() else { continue; };
        if marker_in_worktree(cwd, worktree) {
            return true;
        }
    }
    false
}

/// Best-effort sweep of busy markers older than `stale_secs`. Called on
/// daemon startup and periodically from the status loop to clean up
/// markers leaked by crashed Claude sessions.
pub fn sweep_stale_markers(stale_secs: u64) {
    let Ok(entries) = fs::read_dir(busy_dir()) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue; };
        let Ok(modified) = meta.modified() else { continue; };
        let age = now
            .duration_since(modified)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if age >= stale_secs {
            let _ = fs::remove_file(entry.path());
        }
    }
}

pub fn derive_status(
    meta: &TaskMeta,
    sessions: &HashMap<String, TmuxSession>,
    busy_stale_secs: u64,
) -> TaskStatus {
    let session = find_session(&meta.session, sessions);

    let Some(session) = session else {
        return if meta.paused {
            TaskStatus::Paused
        } else {
            TaskStatus::Idle
        };
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

    // Working iff a fresh busy marker reports a cwd inside this task's
    // worktree. Markers are written by Claude Code's UserPromptSubmit
    // hook and removed on Stop. See docs/busy-detection-plan.md.
    if is_worktree_busy(&meta.worktree, busy_stale_secs) {
        TaskStatus::Working
    } else {
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
    sessions
        .values()
        .find(|s| strip_numeric_prefix(&s.name) == session_name)
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
    busy_stale_secs: u64,
) -> Vec<Task> {
    let ordered = ordered_task_names(order);

    ordered
        .into_iter()
        .map(|name| {
            let meta = load_task_meta(&name);
            let status = derive_status(&meta, sessions, busy_stale_secs);
            Task {
                name,
                meta,
                status,
                prs: Vec::new(), // Filled by gh polling
            }
        })
        .collect()
}

/// Flip `paused=true` for any task whose recorded tmux session is
/// no longer alive (e.g. after a laptop restart). Tasks that never
/// had a session recorded stay as-is (genuinely unspawned → Idle).
pub fn auto_pause_orphaned(sessions: &HashMap<String, TmuxSession>) {
    let dir = tasks_dir();
    for name in load_task_names(&dir) {
        let meta = load_task_meta(&name);
        if meta.session.is_empty() || meta.paused {
            continue;
        }
        if find_session(&meta.session, sessions).is_none() {
            let mut latest = load_task_meta(&name);
            latest.paused = true;
            save_task_meta(&name, &latest);
        }
    }
}

/// Ensure every task .md file has a corresponding state file.
/// Populates worktree if $ORCH_REPO/task-{name} exists on disk.
/// Does NOT populate session — empty session signals "unspawned"
/// to the orchestrator. Session is set by `orch spawn`.
pub fn ensure_state_files() {
    let dir = tasks_dir();
    let repo = std::env::var("ORCH_REPO").ok();
    for name in load_task_names(&dir) {
        let mut meta = load_task_meta(&name);
        let mut changed = false;
        if meta.worktree.is_empty() {
            if let Some(ref r) = repo {
                let wt = format!("{r}/task-{name}");
                if Path::new(&wt).exists() {
                    meta.worktree = wt;
                    changed = true;
                }
            }
        }
        // Create the state file if it doesn't exist yet, even
        // with no populated fields — so the orchestrator knows
        // the task has been seen.
        if !state_dir().join(format!("{name}.json")).exists() {
            changed = true;
        }
        if changed {
            save_task_meta(&name, &meta);
        }
    }
}

pub fn save_task_meta(name: &str, meta: &TaskMeta) {
    // Legacy write — always, regardless of whether v2 is authoritative.
    // The legacy store stays readable as a one-release fallback.
    let dir = state_dir();
    fs::create_dir_all(&dir).ok();
    if let Ok(json) = serde_json::to_string_pretty(meta) {
        let path = dir.join(format!("{name}.json"));
        atomic_write(&path, &json);
    }

    // v2 mirror write when authoritative. Updates the TaskMeta-derived
    // subset of fields on the existing TaskRecord; everything else
    // (drift flags, agent mode, link provenance) is preserved.
    let store = crate::store::Store::default();
    if store.is_authoritative() {
        if let Some(mut record) = store.load_record_by_slug(name) {
            apply_task_meta_to_record(meta, &mut record);
            store.save_record(&record);
        }
        // No record-by-slug: skip silently. Either the slug isn't yet
        // migrated (transitional) or someone is calling save for a
        // task that never had an md file. The legacy write above
        // already captured the state; v2 will sync next time the slug
        // is seen during reconciliation.
    }
}

/// Update a `TaskRecord` from a (legacy) `TaskMeta`. Fields without a
/// TaskMeta counterpart are left untouched. PRs are replaced wholesale
/// with `source=Migration` since TaskMeta doesn't carry provenance.
fn apply_task_meta_to_record(
    meta: &TaskMeta,
    record: &mut crate::store::TaskRecord,
) {
    use crate::store::{DesiredState, LinkSource, PrLink};
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    record.tmux.session_name = meta.session.clone();
    record.worktree.path = meta.worktree.clone();
    record.attention.needs_input = meta.needs_input;

    // Recompute desired_state from legacy fields. A `Closed` record
    // shouldn't be flipped back to Active by a stale TaskMeta save; we
    // only mutate when the current state is reachable from this map.
    let new_desired = if meta.paused {
        DesiredState::Paused
    } else if !meta.session.is_empty() || !meta.worktree.is_empty() {
        DesiredState::Active
    } else {
        DesiredState::New
    };
    if record.desired_state != DesiredState::Closed {
        record.desired_state = new_desired;
    }
    if new_desired == DesiredState::Paused && record.paused_at.is_none() {
        record.paused_at = Some(now);
    }
    if new_desired == DesiredState::Active && record.started_at.is_none() {
        record.started_at = Some(now);
    }

    // PRs: replace the migration-sourced subset, preserve manual links.
    let manual: Vec<_> = record
        .links
        .prs
        .iter()
        .filter(|p| p.source == LinkSource::Manual)
        .cloned()
        .collect();
    let mut new_prs: Vec<PrLink> = meta
        .prs
        .iter()
        .map(|&number| PrLink {
            number,
            source: LinkSource::Migration,
            ..Default::default()
        })
        .collect();
    for m in manual {
        if !new_prs.iter().any(|p| p.number == m.number) {
            new_prs.push(m);
        }
    }
    record.links.prs = new_prs;
    record.updated_at = now;
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
        let meta = load_task_meta(&name);
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
            update_prs(&name, task_prs);
        }
    }
}

/// Narrow update: re-read latest meta and replace only the `prs`
/// field. Prevents clobbering concurrent writes to other fields
/// (e.g. `paused` set by `orch pause` between reconcile's load and save).
fn update_prs(name: &str, prs: Vec<u32>) {
    let mut latest = load_task_meta(name);
    latest.prs = prs;
    save_task_meta(name, &latest);
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
        assert_eq!(
            derive_status(&meta, &sessions, DEFAULT_BUSY_STALE_SECS),
            TaskStatus::Idle,
        );
    }

    #[test]
    fn update_prs_preserves_other_fields() {
        // Simulates reconcile_prs overwriting while CLI concurrently
        // sets paused=true. update_prs must re-read before writing.
        let dir = state_dir();
        fs::create_dir_all(&dir).unwrap();
        let name = "_test_update_prs";
        let path = dir.join(format!("{name}.json"));
        let _ = fs::remove_file(&path);

        // Initial write
        save_task_meta(
            name,
            &TaskMeta {
                session: "task-test".into(),
                prs: vec![1, 2],
                ..Default::default()
            },
        );

        // Simulate CLI writing paused=true after reconcile loaded
        // its copy but before it saves
        let mut latest = load_task_meta(name);
        latest.paused = true;
        save_task_meta(name, &latest);

        // Now reconcile's update_prs should preserve paused
        update_prs(name, vec![1, 2, 3]);

        let result = load_task_meta(name);
        assert_eq!(result.prs, vec![1, 2, 3]);
        assert!(result.paused, "paused flag was clobbered");
        assert_eq!(result.session, "task-test");

        fs::remove_file(&path).ok();
    }

    #[test]
    fn derive_status_paused_when_no_session() {
        let meta = TaskMeta {
            paused: true,
            ..Default::default()
        };
        let sessions = HashMap::new();
        assert_eq!(
            derive_status(&meta, &sessions, DEFAULT_BUSY_STALE_SECS),
            TaskStatus::Paused,
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
            },
        );
        assert_eq!(
            derive_status(&meta, &sessions, DEFAULT_BUSY_STALE_SECS),
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
            },
        );
        assert_eq!(
            derive_status(&meta, &sessions, DEFAULT_BUSY_STALE_SECS),
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
            },
        );
        assert_eq!(
            derive_status(&meta, &sessions, DEFAULT_BUSY_STALE_SECS),
            TaskStatus::Ready,
        );
    }

    #[test]
    fn derive_status_ready_without_busy_marker() {
        // No marker file present anywhere -> Ready even with active process
        let meta = TaskMeta {
            session: "task-foo".into(),
            worktree: "/tmp/orch-test-no-marker".into(),
            ..Default::default()
        };
        let mut sessions = HashMap::new();
        sessions.insert(
            "task-foo".into(),
            TmuxSession {
                name: "task-foo".into(),
                attached: false,
                has_active_process: true,
            },
        );
        assert_eq!(
            derive_status(&meta, &sessions, DEFAULT_BUSY_STALE_SECS),
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
            },
        );
        // Direct match fails, suffix match succeeds
        assert!(sessions.get("task-foo").is_none());
        let found = find_session("task-foo", &sessions);
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "1-task-foo");
    }

    #[test]
    fn marker_in_worktree_exact_and_nested() {
        assert!(marker_in_worktree("/tmp/wt", "/tmp/wt"));
        assert!(marker_in_worktree("/tmp/wt/sub", "/tmp/wt"));
        assert!(!marker_in_worktree("/tmp/other", "/tmp/wt"));
        assert!(!marker_in_worktree("/tmp/wtx", "/tmp/wt"));
    }

    #[test]
    fn marker_in_worktree_handles_tilde() {
        let home = dirs::home_dir().unwrap();
        let home_str = home.to_string_lossy().to_string();
        let abs_wt = format!("{home_str}/code/wt-foo");
        assert!(marker_in_worktree(&abs_wt, "~/code/wt-foo"));
        assert!(marker_in_worktree("~/code/wt-foo", &abs_wt));
    }

    #[test]
    fn marker_in_worktree_empty_worktree() {
        assert!(!marker_in_worktree("/tmp/anything", ""));
    }

    #[test]
    fn is_worktree_busy_fresh_marker() {
        // Use a unique busy dir so this test doesn't interfere with the
        // user's running orch.
        let test_runtime = std::env::temp_dir().join("orch-test-busy-fresh");
        let _ = fs::remove_dir_all(&test_runtime);
        let busy = test_runtime.join("orch").join("busy");
        fs::create_dir_all(&busy).unwrap();

        let wt = "/tmp/test-wt";
        let marker = busy.join("test-sid-1");
        fs::write(
            &marker,
            format!(r#"{{"cwd": "{wt}", "started_at": "x", "pid": 1}}"#),
        )
        .unwrap();

        // Point XDG_RUNTIME_DIR at the test dir for this thread
        // SAFETY: tests are single-threaded and we restore after
        unsafe {
            std::env::set_var("XDG_RUNTIME_DIR", &test_runtime);
        }
        assert!(is_worktree_busy(wt, 60));
        assert!(!is_worktree_busy("/tmp/other", 60));

        // Stale: 0 stale_secs threshold means everything is stale
        assert!(!is_worktree_busy(wt, 0));

        unsafe {
            std::env::remove_var("XDG_RUNTIME_DIR");
        }
        let _ = fs::remove_dir_all(&test_runtime);
    }

    #[test]
    fn task_meta_from_record_flattens_correctly() {
        use crate::store;
        let record = store::TaskRecord {
            id: 5,
            slug: "agentserver".into(),
            desired_state: store::DesiredState::Paused,
            tmux: store::TmuxInfo {
                session_name: "agentserver".into(),
                ..Default::default()
            },
            worktree: store::WorktreeInfo {
                path: "/tmp/wt".into(),
                ..Default::default()
            },
            attention: store::AttentionInfo {
                needs_input: true,
                ..Default::default()
            },
            links: store::Links {
                prs: vec![
                    store::PrLink { number: 100, ..Default::default() },
                    store::PrLink { number: 200, ..Default::default() },
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        let meta = TaskMeta::from_record(&record);
        assert_eq!(meta.session, "agentserver");
        assert_eq!(meta.worktree, "/tmp/wt");
        assert_eq!(meta.prs, vec![100, 200]);
        assert!(meta.needs_input);
        assert!(meta.paused);
    }

    #[test]
    fn apply_task_meta_to_record_recomputes_desired_state() {
        use crate::store;
        let mut record = store::TaskRecord {
            id: 1,
            slug: "x".into(),
            desired_state: store::DesiredState::New,
            ..Default::default()
        };

        // session/worktree set -> Active
        let meta = TaskMeta {
            session: "task-x".into(),
            worktree: "/tmp/wt".into(),
            ..Default::default()
        };
        apply_task_meta_to_record(&meta, &mut record);
        assert_eq!(record.desired_state, store::DesiredState::Active);
        assert_eq!(record.tmux.session_name, "task-x");
        assert!(record.started_at.is_some());

        // paused=true -> Paused
        let meta = TaskMeta {
            session: "task-x".into(),
            worktree: "/tmp/wt".into(),
            paused: true,
            ..Default::default()
        };
        apply_task_meta_to_record(&meta, &mut record);
        assert_eq!(record.desired_state, store::DesiredState::Paused);
        assert!(record.paused_at.is_some());
    }

    #[test]
    fn apply_task_meta_to_record_does_not_unclose() {
        use crate::store;
        let mut record = store::TaskRecord {
            id: 1,
            slug: "x".into(),
            desired_state: store::DesiredState::Closed,
            ..Default::default()
        };
        // Stale TaskMeta with active session shouldn't undo Closed.
        let meta = TaskMeta {
            session: "task-x".into(),
            worktree: "/tmp/wt".into(),
            ..Default::default()
        };
        apply_task_meta_to_record(&meta, &mut record);
        assert_eq!(record.desired_state, store::DesiredState::Closed);
    }

    #[test]
    fn apply_task_meta_to_record_preserves_manual_pr_links() {
        use crate::store;
        let mut record = store::TaskRecord {
            id: 1,
            slug: "x".into(),
            links: store::Links {
                prs: vec![
                    store::PrLink {
                        number: 100,
                        source: store::LinkSource::Manual,
                        ..Default::default()
                    },
                    store::PrLink {
                        number: 200,
                        source: store::LinkSource::Migration,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        let meta = TaskMeta {
            prs: vec![300, 400],
            ..Default::default()
        };
        apply_task_meta_to_record(&meta, &mut record);
        // Migration-sourced 200 dropped; manual 100 retained; new 300, 400 added.
        let nums: Vec<u32> = record.links.prs.iter().map(|p| p.number).collect();
        assert!(nums.contains(&300));
        assert!(nums.contains(&400));
        assert!(nums.contains(&100));
        assert!(!nums.contains(&200));
    }

    #[test]
    fn task_meta_from_record_paused_only_when_desired_paused() {
        use crate::store;
        let record = store::TaskRecord {
            desired_state: store::DesiredState::Active,
            ..Default::default()
        };
        assert!(!TaskMeta::from_record(&record).paused);

        let record = store::TaskRecord {
            desired_state: store::DesiredState::Closed,
            ..Default::default()
        };
        // Closed tasks aren't "paused" in TaskMeta semantics
        assert!(!TaskMeta::from_record(&record).paused);
    }

    #[test]
    fn busy_stale_secs_reads_env() {
        unsafe {
            std::env::set_var("ORCH_BUSY_STALE_SECS", "120");
        }
        assert_eq!(busy_stale_secs(), 120);
        unsafe {
            std::env::remove_var("ORCH_BUSY_STALE_SECS");
        }
        assert_eq!(busy_stale_secs(), DEFAULT_BUSY_STALE_SECS);
    }

    #[test]
    fn task_meta_round_trip() {
        let meta = TaskMeta {
            session: "task-test".into(),
            worktree: "~/code/wt".into(),
            prs: vec![100, 200],
            needs_input: true,
            paused: false,
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
