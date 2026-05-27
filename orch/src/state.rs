use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::SystemTime,
};

use crate::cache;
use crate::store::{
    DesiredState, LinkSource, PrLink, Store, TaskRecord,
};

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

pub fn atomic_write(path: &Path, content: &str) -> bool {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content).is_ok()
        && fs::rename(&tmp, path).is_ok()
}

#[derive(Debug, Clone)]
pub struct TmuxSession {
    pub name: String,
    pub attached: bool,
    pub has_active_process: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum CodexStatus {
    #[default]
    None,
    Commented,
    ThumbsUp,
}

#[derive(Debug, Clone, Default)]
pub struct PrData {
    pub number: u32,
    pub title: String,
    pub ci_pass: Option<bool>,
    pub approved: bool,
    pub codex: CodexStatus,
    /// `OPEN | CLOSED | MERGED`. Empty when never fetched.
    pub state: String,
    /// `MERGEABLE | CONFLICTING | UNKNOWN`. None when never fetched.
    pub mergeable: Option<String>,
    pub head_branch: String,
    /// Head commit SHA — used to invalidate the diff cache.
    pub head_sha: String,
    pub additions: u32,
    pub deletions: u32,
    pub changed_files: u32,
    /// GitHub `updatedAt` ISO 8601.
    pub updated_at: String,
    /// PR description / body (markdown).
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Ready,
    Working,
    Input,
    Idle,
    Attached,
    Paused,
    /// `Active` desired_state but the worker process is dead inside an
    /// otherwise-live tmux session. User can `R` resume to re-spawn.
    Error,
}

#[derive(Debug, Clone)]
pub struct Task {
    pub name: String,
    pub record: TaskRecord,
    pub status: TaskStatus,
}

// Loading

pub fn tasks_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join("tasks")
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
/// worker process. Busy state comes from the marker file written by
/// Claude Code's UserPromptSubmit hook (see `orch busy`).
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

// Busy markers — written by `orch busy start` (UserPromptSubmit hook),
// removed by `orch busy stop` (Stop / SessionEnd hook).

pub fn busy_dir() -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    runtime.join("orch").join("busy")
}

/// Expand a leading `~/` against the user's home directory.
pub fn expand_home(p: &str) -> String {
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

/// Compute the runtime task badge from a v2 record + tmux observation.
/// Output is the cache contract the TUI consumes: persisted intent in
/// `desired_state` × live tmux × busy-marker presence.
pub fn derive_status(
    record: &TaskRecord,
    sessions: &HashMap<String, TmuxSession>,
    busy_stale_secs: u64,
) -> TaskStatus {
    let session = find_session(&record.tmux.session_name, sessions);

    let Some(session) = session else {
        return if record.desired_state == DesiredState::Paused {
            TaskStatus::Paused
        } else {
            TaskStatus::Idle
        };
    };

    if session.attached {
        return TaskStatus::Attached;
    }

    if record.attention.needs_input {
        return TaskStatus::Input;
    }

    if !session.has_active_process {
        return TaskStatus::Error;
    }

    if is_worktree_busy(&record.worktree.path, busy_stale_secs) {
        TaskStatus::Working
    } else {
        TaskStatus::Ready
    }
}

fn find_session<'a>(
    session_name: &str,
    sessions: &'a HashMap<String, TmuxSession>,
) -> Option<&'a TmuxSession> {
    if session_name.is_empty() {
        return None;
    }
    if let Some(s) = sessions.get(session_name) {
        return Some(s);
    }
    sessions
        .values()
        .find(|s| strip_numeric_prefix(&s.name) == session_name)
}

/// Open task slugs in registry order. The v2 store is the single source
/// of truth; an empty registry yields an empty list.
pub fn ordered_open_slugs() -> Vec<String> {
    let store = Store::default();
    let Some(registry) = store.load_registry() else {
        return Vec::new();
    };
    registry
        .open_order
        .iter()
        .filter_map(|id| store.load_record(*id))
        .filter(|r| r.desired_state != DesiredState::Closed)
        .map(|r| r.slug)
        .collect()
}

pub fn load_tasks(
    sessions: &HashMap<String, TmuxSession>,
    busy_stale_secs: u64,
) -> Vec<Task> {
    let store = Store::default();
    let Some(registry) = store.load_registry() else {
        return Vec::new();
    };
    registry
        .open_order
        .iter()
        .filter_map(|id| store.load_record(*id))
        .filter(|r| r.desired_state != DesiredState::Closed)
        .map(|record| {
            let status = derive_status(&record, sessions, busy_stale_secs);
            Task {
                name: record.slug.clone(),
                record,
                status,
            }
        })
        .collect()
}

/// Ensure every `tasks/<name>.md` has an open v2 record. Populates
/// `record.worktree.path` if `$ORCH_REPO/task-<name>` exists on disk.
/// Reopens a closed record when its task file exists again.
/// Does NOT populate `tmux.session_name` — empty session signals
/// "unspawned" to the orchestrator.
pub fn ensure_state_files() {
    let store = Store::default();
    let Some(mut registry) = store.load_registry() else {
        return;
    };

    let dir = tasks_dir();
    let repo = std::env::var("ORCH_REPO").ok();
    let now = cache::now_epoch();

    let known_slugs: HashSet<String> = registry
        .open_order
        .iter()
        .chain(registry.closed_order.iter())
        .filter_map(|id| store.load_record(*id))
        .map(|r| r.slug)
        .collect();

    let mut registry_changed = false;
    for name in load_task_names(&dir) {
        if known_slugs.contains(&name) {
            // Existing record — reopen if the task file exists again, and
            // backfill a missing/stale worktree from $ORCH_REPO.
            if let Some(mut record) = store.load_record_by_slug(&name) {
                if record.desired_state == DesiredState::Closed {
                    record.desired_state = DesiredState::New;
                    record.closed_at = None;
                    record.archived_task_file = None;
                    registry.closed_order.retain(|id| *id != record.id);
                    if !registry.open_order.contains(&record.id) {
                        registry.open_order.push(record.id);
                    }
                    registry_changed = true;
                }
                record.task_file = dir.join(format!("{name}.md"));
                if worktree_missing(&record.worktree.path) {
                    if let Some(wt) = repo_worktree_for(&repo, &name) {
                        record.worktree.path = wt;
                    }
                }
                record.updated_at = now;
                store.save_record(&record);
            }
        } else {
            // New .md added since last daemon scan — allocate a record.
            let id = registry.allocate_id();
            registry.open_order.push(id);
            registry_changed = true;
            let mut record = TaskRecord {
                id,
                slug: name.clone(),
                task_file: dir.join(format!("{name}.md")),
                created_at: now,
                updated_at: now,
                ..Default::default()
            };
            if let Some(wt) = repo_worktree_for(&repo, &name) {
                record.worktree.path = wt;
            }
            store.save_record(&record);
        }
    }
    if registry_changed {
        store.save_registry(&registry);
    }
}

fn worktree_missing(worktree: &str) -> bool {
    worktree.is_empty() || !Path::new(&expand_home(worktree)).exists()
}

#[cfg(test)]
fn ensure_state_files_at(store: &Store, dir: &Path, repo: Option<String>, now: u64) {
    let Some(mut registry) = store.load_registry() else {
        return;
    };

    let known_slugs: HashSet<String> = registry
        .open_order
        .iter()
        .chain(registry.closed_order.iter())
        .filter_map(|id| store.load_record(*id))
        .map(|r| r.slug)
        .collect();

    let mut registry_changed = false;
    for name in load_task_names(dir) {
        if known_slugs.contains(&name) {
            if let Some(mut record) = store.load_record_by_slug(&name) {
                if record.desired_state == DesiredState::Closed {
                    record.desired_state = DesiredState::New;
                    record.closed_at = None;
                    record.archived_task_file = None;
                    registry.closed_order.retain(|id| *id != record.id);
                    if !registry.open_order.contains(&record.id) {
                        registry.open_order.push(record.id);
                    }
                    registry_changed = true;
                }
                record.task_file = dir.join(format!("{name}.md"));
                if worktree_missing(&record.worktree.path) {
                    if let Some(wt) = repo_worktree_for(&repo, &name) {
                        record.worktree.path = wt;
                    }
                }
                record.updated_at = now;
                store.save_record(&record);
            }
        } else {
            let id = registry.allocate_id();
            registry.open_order.push(id);
            registry_changed = true;
            let mut record = TaskRecord {
                id,
                slug: name.clone(),
                task_file: dir.join(format!("{name}.md")),
                created_at: now,
                updated_at: now,
                ..Default::default()
            };
            if let Some(wt) = repo_worktree_for(&repo, &name) {
                record.worktree.path = wt;
            }
            store.save_record(&record);
        }
    }
    if registry_changed {
        store.save_registry(&registry);
    }
}

fn repo_worktree_for(repo: &Option<String>, name: &str) -> Option<String> {
    let repo = repo.as_ref()?;
    let wt = format!("{repo}/task-{name}");
    Path::new(&wt).exists().then_some(wt)
}

/// Reconcile PR state: discover open PRs by branch via `jj op log` and
/// update `record.links.prs`. Manual-sourced PR links are preserved.
pub fn reconcile_prs() {
    let repo_dir = match std::env::var("ORCH_REPO") {
        Ok(r) => format!("{r}/main"),
        Err(_) => return,
    };

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

    let store = Store::default();
    for record in store.load_open_records() {
        let worktree = expand_home(&record.worktree.path);
        if worktree.is_empty() {
            continue;
        }
        let wt_path = Path::new(&worktree);
        if !wt_path.join(".jj").exists() {
            continue;
        }

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
                    if let Some(rest) = line.strip_prefix("push bookmark ") {
                        if let Some(name) = rest.split(" to git remote").next() {
                            branches.insert(name.trim().to_string());
                        }
                    }
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

        if pushed_branches.is_empty() {
            continue;
        }

        let mut discovered: Vec<u32> = Vec::new();
        for branch in &pushed_branches {
            if let Some(nums) = branch_to_prs.get(branch) {
                discovered.extend(nums);
            }
        }
        discovered.sort();
        discovered.dedup();

        let current: Vec<u32> =
            record.links.prs.iter().map(|p| p.number).collect();
        if current == discovered {
            continue;
        }
        update_record_prs(&store, record.slug.clone(), &discovered);
    }
}

/// Rewrite `record.links.prs` to reflect the discovered numbers,
/// preserving Manual-sourced links.
fn update_record_prs(store: &Store, slug: String, discovered: &[u32]) {
    store.update_record_by_slug(&slug, |record| {
        let manual: Vec<PrLink> = record
            .links
            .prs
            .iter()
            .filter(|p| p.source == LinkSource::Manual)
            .cloned()
            .collect();
        let mut new_prs: Vec<PrLink> = discovered
            .iter()
            .map(|&n| PrLink {
                number: n,
                source: LinkSource::BranchDiscovery,
                ..Default::default()
            })
            .collect();
        for m in manual {
            if !new_prs.iter().any(|p| p.number == m.number) {
                new_prs.push(m);
            }
        }
        record.links.prs = new_prs;
        record.updated_at = cache::now_epoch();
    });
}

/// Try `git worktree remove --force`. Close is already an explicit cleanup
/// action, so dirty task worktrees should not linger as stale entries. If git
/// says "not a working tree" (already disowned), the directory is a pure orphan
/// with no git state, so `rm -rf` it. Other errors propagate.
pub fn remove_worktree(path: &Path) -> Result<(), String> {
    let repo = std::env::var("ORCH_REPO")
        .map_err(|_| "ORCH_REPO not set".to_string())?;
    let main = format!("{repo}/main");
    let path_str = path.to_str().ok_or_else(|| "non-utf8 path".to_string())?;
    let output = Command::new("git")
        .args(["worktree", "remove", "--force", path_str])
        .current_dir(&main)
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let msg = stderr.trim().to_string();

    if msg.contains("is not a working tree")
        && !path.join(".git").exists()
        && !path.join(".jj").exists()
    {
        fs::remove_dir_all(path).map_err(|e| e.to_string())?;
        return Ok(());
    }

    Err(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{
        self, AttentionInfo, DesiredState, TaskRecord, TmuxInfo, WorktreeInfo,
    };

    fn record_with(session: &str, worktree: &str) -> TaskRecord {
        TaskRecord {
            id: 1,
            slug: "x".into(),
            tmux: TmuxInfo {
                session_name: session.into(),
                ..Default::default()
            },
            worktree: WorktreeInfo {
                path: worktree.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

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
        assert!(!path.with_extension("tmp").exists());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn derive_status_no_session_idle() {
        let record = TaskRecord::default();
        let sessions = HashMap::new();
        assert_eq!(
            derive_status(&record, &sessions, DEFAULT_BUSY_STALE_SECS),
            TaskStatus::Idle,
        );
    }

    #[test]
    fn derive_status_paused_when_no_session() {
        let record = TaskRecord {
            desired_state: DesiredState::Paused,
            ..Default::default()
        };
        assert_eq!(
            derive_status(&record, &HashMap::new(), DEFAULT_BUSY_STALE_SECS),
            TaskStatus::Paused,
        );
    }

    #[test]
    fn derive_status_attached() {
        let record = record_with("task-foo", "");
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
            derive_status(&record, &sessions, DEFAULT_BUSY_STALE_SECS),
            TaskStatus::Attached,
        );
    }

    #[test]
    fn derive_status_needs_input() {
        let record = TaskRecord {
            tmux: TmuxInfo {
                session_name: "task-foo".into(),
                ..Default::default()
            },
            attention: AttentionInfo {
                needs_input: true,
                ..Default::default()
            },
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
            derive_status(&record, &sessions, DEFAULT_BUSY_STALE_SECS),
            TaskStatus::Input,
        );
    }

    #[test]
    fn derive_status_no_active_process_is_error() {
        let record = record_with("task-foo", "");
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
            derive_status(&record, &sessions, DEFAULT_BUSY_STALE_SECS),
            TaskStatus::Error,
        );
    }

    #[test]
    fn derive_status_ready_without_busy_marker() {
        let record = record_with("task-foo", "/tmp/orch-test-no-marker");
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
            derive_status(&record, &sessions, DEFAULT_BUSY_STALE_SECS),
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

        unsafe {
            std::env::set_var("XDG_RUNTIME_DIR", &test_runtime);
        }
        assert!(is_worktree_busy(wt, 60));
        assert!(!is_worktree_busy("/tmp/other", 60));
        assert!(!is_worktree_busy(wt, 0));

        unsafe {
            std::env::remove_var("XDG_RUNTIME_DIR");
        }
        let _ = fs::remove_dir_all(&test_runtime);
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
    fn is_worker_process_checks() {
        assert!(is_worker_process("claude"));
        assert!(is_worker_process("node"));
        assert!(is_worker_process("codex"));
        assert!(is_worker_process("codex-cli"));
        assert!(!is_worker_process("nvim"));
        assert!(!is_worker_process("zsh"));
    }

    #[test]
    fn update_record_prs_preserves_manual_links() {
        // Set up an isolated store with one open record.
        let root = std::env::temp_dir()
            .join("orch-test-update-record-prs")
            .join(".orch");
        let _ = fs::remove_dir_all(&root);
        let store = Store::at(root.clone());
        let mut registry = store::Registry::new();
        let id = registry.allocate_id();
        registry.open_order = vec![id];
        store.save_registry(&registry);

        let initial = TaskRecord {
            id,
            slug: "demo".into(),
            links: store::Links {
                prs: vec![
                    PrLink {
                        number: 100,
                        source: LinkSource::Manual,
                        ..Default::default()
                    },
                    PrLink {
                        number: 200,
                        source: LinkSource::BranchDiscovery,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        store.save_record(&initial);

        // Reconciliation finds 300 + 400; 200 (BranchDiscovery) drops,
        // 100 (Manual) is preserved.
        update_record_prs(&store, "demo".into(), &[300, 400]);

        let updated = store.load_record_by_slug("demo").unwrap();
        let nums: Vec<u32> = updated.links.prs.iter().map(|p| p.number).collect();
        assert!(nums.contains(&100), "manual 100 retained");
        assert!(nums.contains(&300), "discovered 300 added");
        assert!(nums.contains(&400), "discovered 400 added");
        assert!(!nums.contains(&200), "discovery-sourced 200 dropped");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ensure_state_reopens_closed_task_and_links_worktree() {
        let root = std::env::temp_dir()
            .join("orch-test-reopen-task")
            .join(".orch");
        let tasks = std::env::temp_dir().join("orch-test-reopen-task-tasks");
        let repo = std::env::temp_dir().join("orch-test-reopen-task-repo");
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&tasks);
        let _ = fs::remove_dir_all(&repo);
        fs::create_dir_all(&tasks).unwrap();
        fs::create_dir_all(repo.join("task-foo")).unwrap();
        fs::write(tasks.join("foo.md"), "# Foo\n").unwrap();

        let store = Store::at(root.clone());
        let mut registry = store::Registry::new();
        let id = registry.allocate_id();
        registry.closed_order = vec![id];
        store.save_registry(&registry);
        store.save_record(&TaskRecord {
            id,
            slug: "foo".into(),
            desired_state: DesiredState::Closed,
            closed_at: Some(10),
            archived_task_file: Some(PathBuf::from("/tmp/old.md")),
            worktree: store::WorktreeInfo {
                path: "/tmp/missing-orch-worktree".into(),
                ..Default::default()
            },
            ..Default::default()
        });

        ensure_state_files_at(
            &store,
            &tasks,
            Some(repo.to_string_lossy().to_string()),
            20,
        );

        let registry = store.load_registry().unwrap();
        assert_eq!(registry.open_order, vec![id]);
        assert!(registry.closed_order.is_empty());

        let record = store.load_record_by_slug("foo").unwrap();
        assert_eq!(record.desired_state, DesiredState::New);
        assert_eq!(record.closed_at, None);
        assert_eq!(record.archived_task_file, None);
        assert_eq!(
            record.worktree.path,
            repo.join("task-foo").to_string_lossy().to_string()
        );

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&tasks);
        let _ = fs::remove_dir_all(&repo);
    }
}
