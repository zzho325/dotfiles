use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::state;

fn cache_dir() -> PathBuf {
    state::tasks_dir().join(".orch/cache")
}

pub fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedTask {
    pub session: String,
    pub actual_session: String,
    pub status: String,
    pub has_active_process: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatusCache {
    pub generated_at: u64,
    pub tasks: HashMap<String, CachedTask>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedLinear {
    // Existing.
    pub identifier: String,
    pub title: String,
    /// Workflow state name (e.g. "In Progress", "Done").
    pub state: String,
    /// State category: `backlog | unstarted | started | completed | canceled | triage`.
    pub state_kind: String,
    pub assignee: String,
    pub fetched_at: u64,

    // Identity + display (added in deep-view slice).
    #[serde(default)]
    pub description: String,
    /// 0=none, 1=urgent, 2=high, 3=medium, 4=low (Linear's scale).
    #[serde(default)]
    pub priority: u8,
    #[serde(default)]
    pub priority_label: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub branch_name: String,
    #[serde(default)]
    pub updated_at: String,

    // Hierarchy.
    #[serde(default)]
    pub parent_key: Option<String>,
    #[serde(default)]
    pub parent_title: Option<String>,
    #[serde(default)]
    pub children: Vec<CachedChild>,

    // Project / cycle.
    #[serde(default)]
    pub project: Option<CachedProject>,
    #[serde(default)]
    pub cycle_name: Option<String>,
    #[serde(default)]
    pub cycle_ends_at: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedChild {
    pub identifier: String,
    pub title: String,
    pub state: String,
    pub state_kind: String,
    pub assignee: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedProject {
    pub id: String,
    pub name: String,
    pub slug_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LinearCache {
    pub generated_at: u64,
    /// Keyed by issue identifier (e.g. "ENG-29535").
    pub issues: HashMap<String, CachedLinear>,
    /// Keys that were attempted recently but returned not-found from
    /// Linear (deleted issue, typo, non-Linear key like REQ-01). The
    /// TUI renders these as "(not on Linear)" rather than perpetually
    /// "loading…". Repopulated on each refresh.
    #[serde(default)]
    pub not_found: Vec<String>,
    /// True when the most recent refresh failed (e.g. no API key,
    /// network error). The cache content is retained — TUI can render
    /// last-known data with a "stale/disconnected" badge.
    #[serde(default)]
    pub disconnected: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedPr {
    pub number: u32,
    pub title: String,
    pub ci_pass: Option<bool>,
    pub approved: bool,
    pub codex: String,
    pub fetched_at: u64,

    // Extended metadata (PR redesign — preview + drill).
    /// `OPEN | CLOSED | MERGED`. Empty when never fetched.
    #[serde(default)]
    pub state: String,
    /// `MERGEABLE | CONFLICTING | UNKNOWN`. None when never fetched.
    #[serde(default)]
    pub mergeable: Option<String>,
    /// Head branch name (e.g. `feat/foo`).
    #[serde(default)]
    pub head_branch: String,
    /// Head commit SHA — used to invalidate the diff cache when the branch
    /// is force-pushed or new commits land.
    #[serde(default)]
    pub head_sha: String,
    #[serde(default)]
    pub additions: u32,
    #[serde(default)]
    pub deletions: u32,
    #[serde(default)]
    pub changed_files: u32,
    /// GitHub `updatedAt` ISO 8601. Drives the "3d ago" age display.
    #[serde(default)]
    pub updated_at: String,
    /// PR description / body (markdown). Rendered in the preview pane.
    #[serde(default)]
    pub body: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrCache {
    pub generated_at: u64,
    pub prs: HashMap<u32, CachedPr>,
}

// Diff cache — separate from PrCache so the metadata read-path stays cheap
// (status/preview don't need parsed hunks). Lazy-fetched on first Enter.

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedPrDiff {
    pub number: u32,
    pub fetched_at: u64,
    /// Head SHA of the diff at fetch time. If the live PR's head_sha
    /// differs, the diff is stale and should be refreshed on next Enter.
    #[serde(default)]
    pub head_sha: String,
    /// Raw `gh pr diff` byte size. Above PR_DIFF_RAW_BUDGET → `truncated`.
    #[serde(default)]
    pub raw_size: u64,
    /// True when `raw_size > PR_DIFF_RAW_BUDGET`. Files/hunks are dropped.
    #[serde(default)]
    pub truncated: bool,
    /// Set when `gh pr diff` failed (auth, deleted branch, network).
    /// `files` is empty in this case; the detail view renders the
    /// message instead of "loading…".
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub files: Vec<CachedPrDiffFile>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedPrDiffFile {
    pub path: String,
    /// Pre-rename path; same as `path` when not a rename.
    #[serde(default)]
    pub old_path: Option<String>,
    #[serde(default)]
    pub additions: u32,
    #[serde(default)]
    pub deletions: u32,
    /// `added | modified | deleted | renamed | binary`.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub hunks: Vec<CachedPrDiffHunk>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedPrDiffHunk {
    /// Raw `@@ -a,b +c,d @@ context` header.
    pub header: String,
    /// Raw lines, each prefixed by ` `, `+`, or `-`.
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrDiffCache {
    pub generated_at: u64,
    pub diffs: HashMap<u32, CachedPrDiff>,
}

/// 1.5 MB raw `gh pr diff` byte budget. Above this, the cache stores
/// `truncated: true` and the detail view shows a "diff too large" banner.
pub const PR_DIFF_RAW_BUDGET: u64 = 1_500_000;
/// Per-hunk truncation. Hunks larger than this drop their tail with a
/// `(… N more lines)` marker — same pattern lazygit uses on huge files.
pub const PR_DIFF_LINES_PER_HUNK: usize = 2_000;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Lease {
    pub pid: u32,
    pub heartbeat_at: u64,
}

pub fn read_status() -> StatusCache {
    let path = cache_dir().join("status.json");
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn read_prs() -> PrCache {
    let path = cache_dir().join("prs.json");
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn read_pr_diffs() -> PrDiffCache {
    let path = cache_dir().join("pr_diffs.json");
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn write_pr_diffs(cache: &PrDiffCache) {
    let dir = cache_dir();
    fs::create_dir_all(&dir).ok();
    if let Ok(json) = serde_json::to_string_pretty(cache) {
        state::atomic_write(&dir.join("pr_diffs.json"), &json);
    }
}

pub fn read_linear() -> LinearCache {
    let path = cache_dir().join("linear.json");
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn write_linear(cache: &LinearCache) {
    let dir = cache_dir();
    fs::create_dir_all(&dir).ok();
    if let Ok(json) = serde_json::to_string_pretty(cache) {
        state::atomic_write(&dir.join("linear.json"), &json);
    }
}

impl CachedLinear {
    pub fn from_issue(issue: &crate::linear::LinearIssue) -> Self {
        Self {
            identifier: issue.identifier.clone(),
            title: issue.title.clone(),
            state: issue
                .state
                .as_ref()
                .map(|s| s.name.clone())
                .unwrap_or_default(),
            state_kind: issue
                .state
                .as_ref()
                .map(|s| s.kind.clone())
                .unwrap_or_default(),
            assignee: issue
                .assignee
                .as_ref()
                .map(|a| a.display_name.clone())
                .unwrap_or_default(),
            fetched_at: now_epoch(),
            description: issue.description.clone().unwrap_or_default(),
            priority: issue.priority.unwrap_or(0),
            priority_label: issue.priority_label.clone().unwrap_or_default(),
            url: issue.url.clone().unwrap_or_default(),
            branch_name: issue.branch_name.clone().unwrap_or_default(),
            updated_at: issue.updated_at.clone().unwrap_or_default(),
            parent_key: issue.parent.as_ref().map(|p| p.identifier.clone()),
            parent_title: issue.parent.as_ref().map(|p| p.title.clone()),
            children: issue
                .children
                .as_ref()
                .map(|c| {
                    c.nodes
                        .iter()
                        .map(|child| CachedChild {
                            identifier: child.identifier.clone(),
                            title: child.title.clone(),
                            state: child
                                .state
                                .as_ref()
                                .map(|s| s.name.clone())
                                .unwrap_or_default(),
                            state_kind: child
                                .state
                                .as_ref()
                                .map(|s| s.kind.clone())
                                .unwrap_or_default(),
                            assignee: child
                                .assignee
                                .as_ref()
                                .map(|a| a.display_name.clone())
                                .unwrap_or_default(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            project: issue.project.as_ref().map(|p| CachedProject {
                id: p.id.clone(),
                name: p.name.clone(),
                slug_id: p.slug_id.clone().unwrap_or_default(),
            }),
            cycle_name: issue
                .cycle
                .as_ref()
                .map(|c| c.name.clone().unwrap_or_default()),
            cycle_ends_at: issue
                .cycle
                .as_ref()
                .and_then(|c| c.ends_at.clone()),
        }
    }
}

pub fn read_lease() -> Lease {
    let path = cache_dir().join("lease.json");
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn is_daemon_alive() -> bool {
    let lease = read_lease();
    let age = now_epoch().saturating_sub(lease.heartbeat_at);
    age < 10
}

pub fn write_status(cache: &StatusCache) {
    let dir = cache_dir();
    fs::create_dir_all(&dir).ok();
    if let Ok(json) = serde_json::to_string_pretty(cache) {
        state::atomic_write(&dir.join("status.json"), &json);
    }
}

pub fn write_prs(cache: &PrCache) {
    let dir = cache_dir();
    fs::create_dir_all(&dir).ok();
    if let Ok(json) = serde_json::to_string_pretty(cache) {
        state::atomic_write(&dir.join("prs.json"), &json);
    }
}

pub fn write_lease() {
    let dir = cache_dir();
    fs::create_dir_all(&dir).ok();
    let lease = Lease {
        pid: std::process::id(),
        heartbeat_at: now_epoch(),
    };
    if let Ok(json) = serde_json::to_string(&lease) {
        state::atomic_write(&dir.join("lease.json"), &json);
    }
}

impl CachedPr {
    pub fn to_pr_data(&self) -> state::PrData {
        let codex = match self.codex.as_str() {
            "ThumbsUp" => state::CodexStatus::ThumbsUp,
            "Commented" => state::CodexStatus::Commented,
            _ => state::CodexStatus::None,
        };
        state::PrData {
            number: self.number,
            title: self.title.clone(),
            ci_pass: self.ci_pass,
            approved: self.approved,
            codex,
            state: self.state.clone(),
            mergeable: self.mergeable.clone(),
            head_branch: self.head_branch.clone(),
            head_sha: self.head_sha.clone(),
            additions: self.additions,
            deletions: self.deletions,
            changed_files: self.changed_files,
            updated_at: self.updated_at.clone(),
            body: self.body.clone(),
        }
    }

    pub fn from_pr_data(pr: &state::PrData) -> Self {
        let codex = match pr.codex {
            state::CodexStatus::ThumbsUp => "ThumbsUp",
            state::CodexStatus::Commented => "Commented",
            state::CodexStatus::None => "None",
        };
        CachedPr {
            number: pr.number,
            title: pr.title.clone(),
            ci_pass: pr.ci_pass,
            approved: pr.approved,
            codex: codex.to_string(),
            fetched_at: now_epoch(),
            state: pr.state.clone(),
            mergeable: pr.mergeable.clone(),
            head_branch: pr.head_branch.clone(),
            head_sha: pr.head_sha.clone(),
            additions: pr.additions,
            deletions: pr.deletions,
            changed_files: pr.changed_files,
            updated_at: pr.updated_at.clone(),
            body: pr.body.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_cache_round_trip() {
        let mut tasks = HashMap::new();
        tasks.insert(
            "foo".into(),
            CachedTask {
                session: "task-foo".into(),
                actual_session: "3-task-foo".into(),
                status: "working".into(),
                has_active_process: true,
            },
        );
        let cache = StatusCache {
            generated_at: 100,
            tasks,
        };
        let json = serde_json::to_string(&cache).unwrap();
        let parsed: StatusCache =
            serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.tasks["foo"].status, "working");
        assert_eq!(parsed.tasks["foo"].actual_session, "3-task-foo");
    }

    #[test]
    fn pr_cache_round_trip() {
        let pr = CachedPr {
            number: 123,
            title: "test".into(),
            ci_pass: Some(true),
            approved: true,
            codex: "ThumbsUp".into(),
            fetched_at: 100,
            state: "OPEN".into(),
            mergeable: Some("MERGEABLE".into()),
            head_branch: "feat/foo".into(),
            head_sha: "abc123".into(),
            additions: 287,
            deletions: 42,
            changed_files: 6,
            updated_at: "2026-04-30T12:00:00Z".into(),
            body: "Some description body".into(),
        };
        let data = pr.to_pr_data();
        assert_eq!(data.codex, state::CodexStatus::ThumbsUp);
        assert_eq!(data.state, "OPEN");
        assert_eq!(data.head_sha, "abc123");
        assert_eq!(data.additions, 287);
        let back = CachedPr::from_pr_data(&data);
        assert_eq!(back.codex, "ThumbsUp");
        assert_eq!(back.head_sha, "abc123");
        assert_eq!(back.changed_files, 6);
    }

    #[test]
    fn pr_diff_cache_round_trip() {
        let cache = PrDiffCache {
            generated_at: 100,
            diffs: HashMap::from([(
                4821,
                CachedPrDiff {
                    number: 4821,
                    fetched_at: 200,
                    head_sha: "deadbeef".into(),
                    raw_size: 1024,
                    truncated: false,
                    error: None,
                    files: vec![CachedPrDiffFile {
                        path: "src/foo.rs".into(),
                        old_path: None,
                        additions: 10,
                        deletions: 2,
                        status: "modified".into(),
                        hunks: vec![CachedPrDiffHunk {
                            header: "@@ -1,3 +1,11 @@".into(),
                            lines: vec![
                                " context".into(),
                                "-old".into(),
                                "+new".into(),
                            ],
                        }],
                    }],
                },
            )]),
        };
        let json = serde_json::to_string_pretty(&cache).unwrap();
        let parsed: PrDiffCache = serde_json::from_str(&json).unwrap();
        let d = &parsed.diffs[&4821];
        assert_eq!(d.head_sha, "deadbeef");
        assert_eq!(d.files.len(), 1);
        assert_eq!(d.files[0].hunks[0].lines.len(), 3);
    }

    #[test]
    fn pr_diff_budget_constants_sane() {
        // Sanity bounds — a stamp on the codex-stamped numbers.
        assert_eq!(PR_DIFF_RAW_BUDGET, 1_500_000);
        assert_eq!(PR_DIFF_LINES_PER_HUNK, 2_000);
    }

    #[test]
    fn lease_stale_check() {
        let lease = Lease {
            pid: 1,
            heartbeat_at: 0,
        };
        let json = serde_json::to_string(&lease).unwrap();
        let parsed: Lease =
            serde_json::from_str(&json).unwrap();
        // heartbeat_at=0 is ancient → stale
        let age = now_epoch().saturating_sub(parsed.heartbeat_at);
        assert!(age > 10);
    }
}
