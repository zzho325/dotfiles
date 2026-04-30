//! New persistence model — the v2 store.
//!
//! See `docs/redesign.md` §1 (Data Model) and §6 (Migration Path).
//!
//! Layout:
//!
//! ```text
//! ~/tasks/.orch/
//! ├── runs/                   # untouched by migration
//! ├── store.v2.tmp/           # staging area, deleted on crash recovery
//! ├── store.v2/               # post-cutover authoritative store
//! │   ├── registry.json
//! │   └── tasks/<id>.json
//! └── store.version           # one-line pointer: "v2"; absence = legacy mode
//! ```
//!
//! Slice A scope: data structs, `Store` handle with injectable root,
//! basic load/save. No migration, no read-path integration yet — those
//! are slices B and C.

#![allow(dead_code)] // Wired in slices B-F.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::state::{atomic_write, tasks_dir};

pub type TaskId = u64;

pub const STORE_VERSION: &str = "v2";

// Lifecycle FSM state (persisted intent, distinct from runtime badge).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesiredState {
    #[default]
    New,
    Active,
    Paused,
    Closed,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttentionInfo {
    #[serde(default)]
    pub needs_input: bool,
    #[serde(default)]
    pub last_prompt_from_worker: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeInfo {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub base_ref: String,
    #[serde(default = "default_cleanup_on_close")]
    pub cleanup_on_close: bool,
}

fn default_cleanup_on_close() -> bool {
    true
}

impl Default for WorktreeInfo {
    fn default() -> Self {
        Self {
            path: String::new(),
            base_ref: String::new(),
            cleanup_on_close: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TmuxInfo {
    #[serde(default)]
    pub session_name: String,
    #[serde(default)]
    pub last_known_pane_id: Option<String>,
    #[serde(default)]
    pub pane_titles: Vec<String>,
    #[serde(default)]
    pub active_pane_id: Option<String>,
    /// During a staged 3-step rename: set before any external op,
    /// cleared only after all steps succeed. Presence with
    /// `drift.rename_failed=true` means the user can retry the
    /// rename idempotently with `M`.
    #[serde(default)]
    pub rename_in_flight: Option<RenameInFlight>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenameInFlight {
    pub old_name: String,
    pub new_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMode {
    #[default]
    DirectWorker,
    Orchestrated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerKind {
    #[default]
    ClaudeCode,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentInfo {
    #[serde(default)]
    pub mode: AgentMode,
    #[serde(default)]
    pub worker_kind: WorkerKind,
    #[serde(default)]
    pub orchestrator_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkSource {
    #[default]
    Manual,
    BranchDiscovery,
    MarkdownScan,
    Migration,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrLink {
    pub number: u32,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub source: LinkSource,
    #[serde(default)]
    pub last_verified_at: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LinearLink {
    pub key: String,
    #[serde(default)]
    pub source: LinkSource,
    #[serde(default)]
    pub last_verified_at: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Links {
    #[serde(default)]
    pub prs: Vec<PrLink>,
    #[serde(default)]
    pub linear_issues: Vec<LinearLink>,
    #[serde(default)]
    pub notes_urls: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriftFlags {
    #[serde(default)]
    pub session_missing: bool,
    #[serde(default)]
    pub worker_dead: bool,
    #[serde(default)]
    pub cleanup_pending: bool,
    #[serde(default)]
    pub cleanup_failed: bool,
    #[serde(default)]
    pub worktree_missing: bool,
    #[serde(default)]
    pub rename_failed: bool,
    #[serde(default)]
    pub last_error: Option<String>,
}

impl DriftFlags {
    /// True if any *flag* is set (excluding `last_error`, which is
    /// informational, not a drift state on its own).
    pub fn any(&self) -> bool {
        self.session_missing
            || self.worker_dead
            || self.cleanup_pending
            || self.cleanup_failed
            || self.worktree_missing
            || self.rename_failed
    }
}

/// Per-task record. The source of truth for one task; runtime badges
/// and caches are derived separately and never write back here.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskRecord {
    pub id: TaskId,
    pub slug: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub task_file: PathBuf,
    #[serde(default)]
    pub archived_task_file: Option<PathBuf>,

    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub started_at: Option<u64>,
    #[serde(default)]
    pub paused_at: Option<u64>,
    #[serde(default)]
    pub closed_at: Option<u64>,
    #[serde(default)]
    pub updated_at: u64,

    #[serde(default)]
    pub desired_state: DesiredState,
    #[serde(default)]
    pub attention: AttentionInfo,
    #[serde(default)]
    pub worktree: WorktreeInfo,
    #[serde(default)]
    pub tmux: TmuxInfo,
    #[serde(default)]
    pub agent: AgentInfo,
    #[serde(default)]
    pub links: Links,
    #[serde(default)]
    pub drift: DriftFlags,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Registry {
    pub version: String,
    #[serde(default = "default_next_id")]
    pub next_task_id: TaskId,
    #[serde(default)]
    pub open_order: Vec<TaskId>,
    #[serde(default)]
    pub closed_order: Vec<TaskId>,
}

fn default_next_id() -> TaskId {
    1
}

impl Registry {
    pub fn new() -> Self {
        Self {
            version: STORE_VERSION.to_string(),
            next_task_id: 1,
            open_order: Vec::new(),
            closed_order: Vec::new(),
        }
    }

    /// Allocate the next id and bump `next_task_id`. Caller must save.
    pub fn allocate_id(&mut self) -> TaskId {
        let id = self.next_task_id;
        self.next_task_id += 1;
        id
    }
}

/// Handle to the v2 store rooted at a specific `.orch/` directory.
/// Production callers use `Store::default()`; tests construct with
/// `Store::at(<temp_dir>.join(".orch"))` to stay isolated from the
/// user's real `~/tasks/.orch/`.
pub struct Store {
    pub orch_root: PathBuf,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            orch_root: tasks_dir().join(".orch"),
        }
    }
}

impl Store {
    pub fn at(orch_root: PathBuf) -> Self {
        Self { orch_root }
    }

    pub fn store_root(&self) -> PathBuf {
        self.orch_root.join(format!("store.{STORE_VERSION}"))
    }

    pub fn store_root_tmp(&self) -> PathBuf {
        self.orch_root.join(format!("store.{STORE_VERSION}.tmp"))
    }

    pub fn store_version_path(&self) -> PathBuf {
        self.orch_root.join("store.version")
    }

    pub fn registry_path(&self) -> PathBuf {
        self.store_root().join("registry.json")
    }

    pub fn task_record_path(&self, id: TaskId) -> PathBuf {
        self.store_root().join("tasks").join(format!("{id}.json"))
    }

    /// True iff the v2 store is the authoritative reader. Absence of
    /// the marker (or a different version string) keeps orch on the
    /// legacy `.state/*.json` reader.
    pub fn is_authoritative(&self) -> bool {
        fs::read_to_string(self.store_version_path())
            .map(|s| s.trim() == STORE_VERSION)
            .unwrap_or(false)
    }

    pub fn load_registry(&self) -> Option<Registry> {
        fs::read_to_string(self.registry_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    }

    pub fn save_registry(&self, r: &Registry) -> bool {
        let path = self.registry_path();
        let Some(parent) = path.parent() else {
            return false;
        };
        if fs::create_dir_all(parent).is_err() {
            return false;
        }
        let Ok(json) = serde_json::to_string_pretty(r) else {
            return false;
        };
        atomic_write(&path, &json)
    }

    pub fn load_record(&self, id: TaskId) -> Option<TaskRecord> {
        fs::read_to_string(self.task_record_path(id))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    }

    pub fn save_record(&self, record: &TaskRecord) -> bool {
        let path = self.task_record_path(record.id);
        let Some(parent) = path.parent() else {
            return false;
        };
        if fs::create_dir_all(parent).is_err() {
            return false;
        }
        let Ok(json) = serde_json::to_string_pretty(record) else {
            return false;
        };
        atomic_write(&path, &json)
    }

    /// Find a record by slug — linear scan over `open_order +
    /// closed_order`. Acceptable for personal-tool scale (handful).
    pub fn load_record_by_slug(&self, slug: &str) -> Option<TaskRecord> {
        let registry = self.load_registry()?;
        for id in registry.open_order.iter().chain(registry.closed_order.iter()) {
            if let Some(r) = self.load_record(*id) {
                if r.slug == slug {
                    return Some(r);
                }
            }
        }
        None
    }

    /// Iterate all records in `open_order` order (closed records excluded).
    pub fn load_open_records(&self) -> Vec<TaskRecord> {
        let Some(registry) = self.load_registry() else {
            return Vec::new();
        };
        registry
            .open_order
            .iter()
            .filter_map(|id| self.load_record(*id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolate(name: &str) -> Store {
        let root = std::env::temp_dir()
            .join(format!("orch-store-test-{name}"))
            .join(".orch");
        let _ = fs::remove_dir_all(&root);
        Store::at(root)
    }

    #[test]
    fn desired_state_serializes_snake_case() {
        let json = serde_json::to_string(&DesiredState::Active).unwrap();
        assert_eq!(json, "\"active\"");
        let json = serde_json::to_string(&DesiredState::Paused).unwrap();
        assert_eq!(json, "\"paused\"");
    }

    #[test]
    fn record_round_trip_preserves_all_fields() {
        let record = TaskRecord {
            id: 12,
            slug: "infra-triage".into(),
            title: Some("Investigate Slash Financial".into()),
            task_file: PathBuf::from("/Users/a/tasks/infra-triage.md"),
            archived_task_file: None,
            created_at: 1000,
            started_at: Some(1100),
            paused_at: None,
            closed_at: None,
            updated_at: 2000,
            desired_state: DesiredState::Active,
            attention: AttentionInfo {
                needs_input: true,
                last_prompt_from_worker: Some("ready for review".into()),
            },
            worktree: WorktreeInfo {
                path: "/Users/a/column/task-infra-triage".into(),
                base_ref: "main".into(),
                cleanup_on_close: true,
            },
            tmux: TmuxInfo {
                session_name: "infra-triage".into(),
                last_known_pane_id: Some("%42".into()),
                pane_titles: vec!["worker".into(), "jj-log".into()],
                active_pane_id: Some("%42".into()),
                rename_in_flight: None,
            },
            agent: AgentInfo {
                mode: AgentMode::DirectWorker,
                worker_kind: WorkerKind::ClaudeCode,
                orchestrator_enabled: false,
            },
            links: Links {
                prs: vec![PrLink {
                    number: 25163,
                    repo: "column".into(),
                    source: LinkSource::BranchDiscovery,
                    last_verified_at: Some(1900),
                }],
                linear_issues: vec![LinearLink {
                    key: "ENG-29151".into(),
                    source: LinkSource::MarkdownScan,
                    last_verified_at: Some(1950),
                }],
                notes_urls: vec![],
            },
            drift: DriftFlags::default(),
        };
        let json = serde_json::to_string_pretty(&record).unwrap();
        let parsed: TaskRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, record);
    }

    #[test]
    fn record_with_rename_in_flight_round_trips() {
        let record = TaskRecord {
            id: 13,
            slug: "ach-sanitize".into(),
            tmux: TmuxInfo {
                session_name: "ach-sanitize".into(),
                rename_in_flight: Some(RenameInFlight {
                    old_name: "ach-sanitize".into(),
                    new_name: "ach-clean".into(),
                }),
                ..Default::default()
            },
            drift: DriftFlags {
                rename_failed: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let json = serde_json::to_string(&record).unwrap();
        let parsed: TaskRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.tmux.rename_in_flight, record.tmux.rename_in_flight);
        assert!(parsed.drift.rename_failed);
    }

    #[test]
    fn registry_round_trip() {
        let mut r = Registry::new();
        let id1 = r.allocate_id();
        let id2 = r.allocate_id();
        r.open_order = vec![id1, id2];
        r.closed_order = vec![5];

        let json = serde_json::to_string(&r).unwrap();
        let parsed: Registry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, r);
        assert_eq!(parsed.next_task_id, 3);
        assert_eq!(parsed.open_order, vec![1, 2]);
    }

    #[test]
    fn allocate_id_increments() {
        let mut r = Registry::new();
        assert_eq!(r.allocate_id(), 1);
        assert_eq!(r.allocate_id(), 2);
        assert_eq!(r.allocate_id(), 3);
        assert_eq!(r.next_task_id, 4);
    }

    #[test]
    fn drift_any_detects_set_flags() {
        let mut d = DriftFlags::default();
        assert!(!d.any());
        d.session_missing = true;
        assert!(d.any());

        let mut d = DriftFlags::default();
        d.last_error = Some("spawn rejected".into());
        // last_error is informational, not a drift state on its own
        assert!(!d.any());
    }

    #[test]
    fn store_paths_under_orch_root() {
        let store = isolate("paths");
        assert!(store.store_root().ends_with("store.v2"));
        assert!(store.store_root_tmp().ends_with("store.v2.tmp"));
        assert!(store.store_version_path().ends_with("store.version"));
        assert!(store.registry_path().ends_with("store.v2/registry.json"));
        assert!(store
            .task_record_path(42)
            .ends_with(Path::new("store.v2/tasks/42.json")));
    }

    #[test]
    fn store_is_authoritative_marker() {
        let store = isolate("auth");
        fs::create_dir_all(&store.orch_root).unwrap();

        // No marker -> not authoritative
        assert!(!store.is_authoritative());

        // Marker with v2 -> authoritative
        fs::write(store.store_version_path(), "v2").unwrap();
        assert!(store.is_authoritative());

        // Different version -> not authoritative
        fs::write(store.store_version_path(), "v3").unwrap();
        assert!(!store.is_authoritative());
    }

    #[test]
    fn save_load_record_round_trip() {
        let store = isolate("record-rt");
        let record = TaskRecord {
            id: 7,
            slug: "test-task".into(),
            desired_state: DesiredState::Active,
            created_at: 1000,
            ..Default::default()
        };
        assert!(store.save_record(&record));
        let loaded = store.load_record(7).unwrap();
        assert_eq!(loaded, record);
    }

    #[test]
    fn save_load_registry_round_trip() {
        let store = isolate("registry-rt");
        let mut r = Registry::new();
        r.allocate_id();
        r.allocate_id();
        r.open_order = vec![1, 2];
        assert!(store.save_registry(&r));
        let loaded = store.load_registry().unwrap();
        assert_eq!(loaded, r);
    }

    #[test]
    fn load_record_by_slug_finds_record() {
        let store = isolate("by-slug");
        let mut r = Registry::new();
        let id = r.allocate_id();
        r.open_order = vec![id];
        assert!(store.save_registry(&r));

        let record = TaskRecord {
            id,
            slug: "find-me".into(),
            ..Default::default()
        };
        assert!(store.save_record(&record));

        let found = store.load_record_by_slug("find-me").unwrap();
        assert_eq!(found.id, id);
        assert!(store.load_record_by_slug("not-there").is_none());
    }

    #[test]
    fn load_open_records_excludes_closed() {
        let store = isolate("open-only");
        let mut r = Registry::new();
        let open_id = r.allocate_id();
        let closed_id = r.allocate_id();
        r.open_order = vec![open_id];
        r.closed_order = vec![closed_id];
        assert!(store.save_registry(&r));

        let open_rec = TaskRecord {
            id: open_id,
            slug: "open-task".into(),
            ..Default::default()
        };
        assert!(store.save_record(&open_rec));

        let closed_rec = TaskRecord {
            id: closed_id,
            slug: "closed-task".into(),
            desired_state: DesiredState::Closed,
            ..Default::default()
        };
        assert!(store.save_record(&closed_rec));

        let records = store.load_open_records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].slug, "open-task");
    }
}
