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
    pub identifier: String,
    pub title: String,
    /// Workflow state name (e.g. "In Progress", "Done").
    pub state: String,
    /// State category: `backlog | unstarted | started | completed | canceled | triage`.
    pub state_kind: String,
    pub assignee: String,
    pub fetched_at: u64,
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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrCache {
    pub generated_at: u64,
    pub prs: HashMap<u32, CachedPr>,
}

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
        };
        let data = pr.to_pr_data();
        assert_eq!(data.codex, state::CodexStatus::ThumbsUp);
        let back = CachedPr::from_pr_data(&data);
        assert_eq!(back.codex, "ThumbsUp");
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
