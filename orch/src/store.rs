//! v2 persistence model — the authoritative store.
//!
//! Layout:
//!
//! ```text
//! ~/tasks/.orch/
//! ├── runs/                   # untouched by migration
//! ├── store.v2.tmp/           # staging area, deleted on crash recovery
//! ├── store.v2/               # authoritative store
//! │   ├── registry.json
//! │   └── tasks/<id>.json
//! └── store.version           # one-line pointer: "v2"
//! ```

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::state::{
    atomic_write, load_task_names, session_matches, tasks_dir,
};

pub type TaskId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreVersion {
    V2,
}

impl StoreVersion {
    pub const CURRENT: Self = Self::V2;

    pub fn marker(self) -> &'static str {
        match self {
            Self::V2 => "v2",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "v2" => Some(Self::V2),
            _ => None,
        }
    }
}

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
            version: StoreVersion::CURRENT.marker().to_string(),
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
        self.orch_root
            .join(format!("store.{}", StoreVersion::CURRENT.marker()))
    }

    pub fn store_root_tmp(&self) -> PathBuf {
        self.orch_root
            .join(format!("store.{}.tmp", StoreVersion::CURRENT.marker()))
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
            .ok()
            .and_then(|s| StoreVersion::parse(&s))
            .is_some()
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

    /// Load the record for `slug`, run `f` to mutate it, and save it
    /// back. Returns `true` if the record existed and was saved.
    pub fn update_record_by_slug<F>(&self, slug: &str, mut f: F) -> bool
    where
        F: FnMut(&mut TaskRecord),
    {
        let Some(mut record) = self.load_record_by_slug(slug) else {
            return false;
        };
        f(&mut record);
        self.save_record(&record)
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

    // Migration. See `docs/redesign.md` §6.

    /// Migrate from legacy `.state/*.json` + `order.json` to the v2
    /// store. Idempotent: a partial run leaves `store.v2.tmp/` and no
    /// marker; the next call discards the tmp and starts over.
    ///
    /// Steps:
    /// 1. Discard any leftover `store.v2.tmp/` from a prior crash.
    /// 2. Build records from `tasks_dir` (legacy `.state/*.json` +
    ///    `order.json` + task .md files).
    /// 3. Write into `store.v2.tmp/`.
    /// 4. fsync the tmp dir.
    /// 5. Atomic rename `store.v2.tmp` -> `store.v2`.
    /// 6. fsync `.orch/`.
    /// 7. Write `store.version=v2`, fsync.
    ///
    /// Returns the count of migrated records on success.
    pub fn migrate_from_legacy(&self, tasks_dir: &Path) -> Result<usize, String> {
        // Skip if already migrated.
        if self.is_authoritative() {
            return Ok(0);
        }

        // Step 1: discard leftover tmp.
        let tmp_root = self.store_root_tmp();
        let _ = fs::remove_dir_all(&tmp_root);
        // Also discard any half-finished store.v2 from a prior crash —
        // without the marker it isn't authoritative anyway.
        let _ = fs::remove_dir_all(self.store_root());

        // Step 2: read legacy state.
        let names = load_task_names(tasks_dir);
        let order = legacy_order(tasks_dir);
        let live_sessions = legacy_live_sessions();

        // Step 3: build registry + records, write into tmp.
        let mut registry = Registry::new();
        let assignment_order = ordered_names(&order, &names);
        let tmp_tasks = tmp_root.join("tasks");
        fs::create_dir_all(&tmp_tasks)
            .map_err(|e| format!("create tmp/tasks: {e}"))?;

        let now = crate::cache::now_epoch();
        for slug in &assignment_order {
            let id = registry.allocate_id();
            registry.open_order.push(id);
            let record = build_record_from_legacy(
                id,
                slug,
                tasks_dir,
                &live_sessions,
                now,
            );
            let path = tmp_tasks.join(format!("{id}.json"));
            let json = serde_json::to_string_pretty(&record)
                .map_err(|e| format!("serialize record {id}: {e}"))?;
            atomic_write(&path, &json);
            fsync_path(&path).map_err(|e| format!("fsync record {id}: {e}"))?;
        }

        // Write registry into tmp.
        let registry_path = tmp_root.join("registry.json");
        let json = serde_json::to_string_pretty(&registry)
            .map_err(|e| format!("serialize registry: {e}"))?;
        atomic_write(&registry_path, &json);
        fsync_path(&registry_path)
            .map_err(|e| format!("fsync registry: {e}"))?;

        // Step 4: fsync the tasks/ subdir + the tmp root dir.
        fsync_path(&tmp_tasks)
            .map_err(|e| format!("fsync tmp tasks dir: {e}"))?;
        fsync_path(&tmp_root)
            .map_err(|e| format!("fsync tmp dir: {e}"))?;

        // Step 5: atomic rename.
        fs::rename(&tmp_root, self.store_root())
            .map_err(|e| format!("rename tmp -> store: {e}"))?;

        // Step 6: fsync .orch/.
        fsync_path(&self.orch_root)
            .map_err(|e| format!("fsync .orch/: {e}"))?;

        // Step 7: write marker, fsync.
        let version_path = self.store_version_path();
        atomic_write(&version_path, StoreVersion::CURRENT.marker());
        fsync_path(&version_path)
            .map_err(|e| format!("fsync store.version: {e}"))?;
        fsync_path(&self.orch_root)
            .map_err(|e| format!("fsync .orch/ post-marker: {e}"))?;

        Ok(registry.open_order.len())
    }
}

fn fsync_path(path: &Path) -> std::io::Result<()> {
    fs::File::open(path)?.sync_all()
}

/// Read the legacy `order.json` if present.
fn legacy_order(tasks_dir: &Path) -> Vec<String> {
    let path = tasks_dir.join(".state").join("order.json");
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Live tmux session names (e.g. `task-foo`, `3-task-foo`).
fn legacy_live_sessions() -> Vec<String> {
    let output = std::process::Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok();
    let Some(output) = output.filter(|o| o.status.success()) else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(String::from)
        .collect()
}

/// ID-assignment order: first task names listed in `order.json`, then
/// any remaining task names alphabetically.
fn ordered_names(order: &[String], all: &[String]) -> Vec<String> {
    let mut result: Vec<String> = order
        .iter()
        .filter(|n| all.contains(n))
        .cloned()
        .collect();
    for name in all {
        if !result.contains(name) {
            result.push(name.clone());
        }
    }
    result
}

/// Build a TaskRecord from legacy `.state/<slug>.json` plus tmux
/// observation. Field mapping per `redesign.md` §6.
fn build_record_from_legacy(
    id: TaskId,
    slug: &str,
    tasks_dir: &Path,
    live_sessions: &[String],
    now: u64,
) -> TaskRecord {
    let legacy_path = tasks_dir.join(".state").join(format!("{slug}.json"));
    let legacy: LegacyMeta = fs::read_to_string(&legacy_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let task_file = tasks_dir.join(format!("{slug}.md"));

    // desired_state: paused>active>new based on legacy fields + live tmux.
    let has_session_or_wt = !legacy.session.is_empty() || !legacy.worktree.is_empty();
    let desired_state = if legacy.paused {
        DesiredState::Paused
    } else if has_session_or_wt {
        DesiredState::Active
    } else {
        DesiredState::New
    };

    // Drift if persisted session is not in live tmux (recoverable on resume).
    let session_missing = !legacy.session.is_empty()
        && desired_state == DesiredState::Active
        && !live_sessions.iter().any(|s| session_matches(s, &legacy.session));

    TaskRecord {
        id,
        slug: slug.to_string(),
        title: None,
        task_file,
        archived_task_file: None,
        created_at: now,
        started_at: if has_session_or_wt { Some(now) } else { None },
        paused_at: if legacy.paused { Some(now) } else { None },
        closed_at: None,
        updated_at: now,
        desired_state,
        attention: AttentionInfo {
            needs_input: legacy.needs_input,
            last_prompt_from_worker: None,
        },
        worktree: WorktreeInfo {
            path: legacy.worktree,
            base_ref: String::new(),
            cleanup_on_close: true,
        },
        tmux: TmuxInfo {
            session_name: legacy.session,
            ..Default::default()
        },
        agent: AgentInfo::default(),
        links: Links {
            prs: legacy
                .prs
                .into_iter()
                .map(|number| PrLink {
                    number,
                    source: LinkSource::Migration,
                    ..Default::default()
                })
                .collect(),
            linear_issues: Vec::new(),
            notes_urls: Vec::new(),
        },
        drift: DriftFlags {
            session_missing,
            ..Default::default()
        },
    }
}

/// Local copy of the legacy meta layout so migration doesn't depend on
/// the live shape of `state::TaskMeta` (which evolves in later slices).
#[derive(Debug, Clone, Default, Deserialize)]
struct LegacyMeta {
    #[serde(default)]
    session: String,
    #[serde(default)]
    worktree: String,
    #[serde(default)]
    prs: Vec<u32>,
    #[serde(default)]
    needs_input: bool,
    #[serde(default)]
    paused: bool,
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

    /// Build a fixture `tasks_dir` with legacy `.state/<slug>.json`,
    /// `order.json`, and task `.md` files. Returns the dir.
    fn build_legacy_fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("orch-migrate-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".state")).unwrap();

        // 3 tasks, 2 with state files (started/paused), 1 fresh
        for slug in &["foo", "bar", "baz"] {
            fs::write(dir.join(format!("{slug}.md")), "# stub").unwrap();
        }

        fs::write(
            dir.join(".state").join("foo.json"),
            r#"{"session":"task-foo","worktree":"/tmp/wt-foo","prs":[100,200],"needs_input":false,"paused":false}"#,
        ).unwrap();
        fs::write(
            dir.join(".state").join("bar.json"),
            r#"{"session":"task-bar","worktree":"/tmp/wt-bar","prs":[],"needs_input":true,"paused":true}"#,
        ).unwrap();
        // baz has no .state/baz.json — it's a fresh task

        // Order: bar, foo (baz appended at the end alphabetically)
        fs::write(
            dir.join(".state").join("order.json"),
            r#"["bar","foo"]"#,
        ).unwrap();

        dir
    }

    #[test]
    fn migrate_writes_marker_and_records() {
        let store = isolate("migrate-basic");
        let tasks = build_legacy_fixture("migrate-basic-tasks");

        let count = store.migrate_from_legacy(&tasks).unwrap();
        assert_eq!(count, 3);

        // Marker present + authoritative
        assert!(store.is_authoritative());

        // Tmp dir is gone after successful rename
        assert!(!store.store_root_tmp().exists());

        // Registry has the right ids and order: bar=1, foo=2, baz=3
        let registry = store.load_registry().unwrap();
        assert_eq!(registry.next_task_id, 4);
        assert_eq!(registry.open_order, vec![1, 2, 3]);
        assert!(registry.closed_order.is_empty());

        let bar = store.load_record(1).unwrap();
        assert_eq!(bar.slug, "bar");
        assert_eq!(bar.desired_state, DesiredState::Paused);
        assert!(bar.attention.needs_input);
        assert_eq!(bar.tmux.session_name, "task-bar");
        assert_eq!(bar.worktree.path, "/tmp/wt-bar");

        let foo = store.load_record(2).unwrap();
        assert_eq!(foo.slug, "foo");
        assert_eq!(foo.desired_state, DesiredState::Active);
        assert_eq!(foo.links.prs.len(), 2);
        assert_eq!(foo.links.prs[0].number, 100);
        assert_eq!(foo.links.prs[0].source, LinkSource::Migration);

        let baz = store.load_record(3).unwrap();
        assert_eq!(baz.slug, "baz");
        assert_eq!(baz.desired_state, DesiredState::New);
        assert_eq!(baz.tmux.session_name, "");

        let _ = fs::remove_dir_all(&tasks);
    }

    #[test]
    fn migrate_is_idempotent() {
        let store = isolate("migrate-idempotent");
        let tasks = build_legacy_fixture("migrate-idempotent-tasks");

        let first = store.migrate_from_legacy(&tasks).unwrap();
        assert_eq!(first, 3);
        assert!(store.is_authoritative());

        // Second call short-circuits because marker is present.
        let second = store.migrate_from_legacy(&tasks).unwrap();
        assert_eq!(second, 0);

        let _ = fs::remove_dir_all(&tasks);
    }

    #[test]
    fn migrate_recovers_from_partial_tmp() {
        let store = isolate("migrate-partial");
        let tasks = build_legacy_fixture("migrate-partial-tasks");

        // Simulate a crashed prior migration: leftover tmp + partial store.
        fs::create_dir_all(store.store_root_tmp().join("tasks")).unwrap();
        fs::write(
            store.store_root_tmp().join("registry.json"),
            r#"{"version":"v2","next_task_id":99}"#,
        ).unwrap();

        // No marker yet, so this run should discard the tmp and start fresh.
        let count = store.migrate_from_legacy(&tasks).unwrap();
        assert_eq!(count, 3);

        // Fresh ids — leftover registry didn't influence this run.
        let registry = store.load_registry().unwrap();
        assert_eq!(registry.next_task_id, 4);
        assert!(!store.store_root_tmp().exists());

        let _ = fs::remove_dir_all(&tasks);
    }

    #[test]
    fn migrate_drops_session_missing_drift_on_legacy_session_not_live() {
        let store = isolate("migrate-drift");
        let tasks = build_legacy_fixture("migrate-drift-tasks");

        // foo has session=task-foo in legacy; tmux is empty (no live).
        store.migrate_from_legacy(&tasks).unwrap();
        let foo = store.load_record_by_slug("foo").unwrap();
        assert_eq!(foo.desired_state, DesiredState::Active);
        // Active + persisted session not live -> session_missing drift.
        assert!(foo.drift.session_missing);

        let _ = fs::remove_dir_all(&tasks);
    }

    #[test]
    fn ordered_names_legacy_first_then_alpha() {
        let order = vec!["bar".into(), "foo".into()];
        let all = vec!["baz".into(), "bar".into(), "foo".into(), "qux".into()];
        let result = ordered_names(&order, &all);
        assert_eq!(result, vec!["bar", "foo", "baz", "qux"]);
    }

    #[test]
    fn ordered_names_skips_stale_order_entries() {
        // order.json refers to "deleted" which has no .md anymore
        let order = vec!["deleted".into(), "foo".into()];
        let all = vec!["foo".into(), "bar".into()];
        let result = ordered_names(&order, &all);
        assert_eq!(result, vec!["foo", "bar"]);
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
