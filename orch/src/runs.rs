//! Per-run output storage for orchestrator invocations.
//!
//! Each run gets a directory under `~/tasks/.orch/runs/<run_id>/` with:
//! - `trigger.txt` — the input that triggered the run
//! - `output.log` — combined stdout+stderr
//! - `meta.json` — status, timing, exit code

use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunMeta {
    pub id: String,
    pub started_at: u64,
    #[serde(default)]
    pub finished_at: Option<u64>,
    #[serde(default)]
    pub exit_code: Option<i32>,
    pub trigger_kind: String,
    pub trigger_summary: String,
}

fn orch_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join("tasks/.orch")
}

fn runs_dir() -> PathBuf {
    orch_dir().join("runs")
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Classify a trigger message into a kind and short summary.
fn classify_trigger(message: &str) -> (String, String) {
    let first_line = message.lines().next().unwrap_or(message);
    let kind = if first_line.contains("[new-task]") {
        "new-task"
    } else if first_line.contains("[message]") {
        "message"
    } else if first_line.contains("[scan]") {
        "scan"
    } else {
        "other"
    };
    let summary: String = first_line.chars().take(80).collect();
    (kind.to_string(), summary)
}

/// Create a new run directory and return (run_id, output_log_path).
pub fn create_run(message: &str) -> Option<(String, PathBuf)> {
    let ts = now_epoch();
    let (kind, summary) = classify_trigger(message);
    let id = format!("{ts}-{kind}");

    let dir = runs_dir().join(&id);
    fs::create_dir_all(&dir).ok()?;

    // Write trigger
    fs::write(dir.join("trigger.txt"), message).ok()?;

    // Write initial meta
    let meta = RunMeta {
        id: id.clone(),
        started_at: ts,
        finished_at: None,
        exit_code: None,
        trigger_kind: kind,
        trigger_summary: summary,
    };
    let json = serde_json::to_string_pretty(&meta).ok()?;
    fs::write(dir.join("meta.json"), json).ok()?;

    Some((id, dir.join("output.log")))
}

/// Update meta.json with final status after the run completes.
pub fn finish_run(run_id: &str, exit_code: i32) {
    let meta_path = runs_dir().join(run_id).join("meta.json");
    let Ok(content) = fs::read_to_string(&meta_path) else {
        return;
    };
    let Ok(mut meta) = serde_json::from_str::<RunMeta>(&content) else {
        return;
    };
    meta.finished_at = Some(now_epoch());
    meta.exit_code = Some(exit_code);
    if let Ok(json) = serde_json::to_string_pretty(&meta) {
        crate::state::atomic_write(&meta_path, &json);
    }
}

/// List recent runs, newest first.
pub fn list_runs(limit: usize) -> Vec<RunMeta> {
    let dir = runs_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut runs: Vec<RunMeta> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let meta_path = e.path().join("meta.json");
            let content = fs::read_to_string(&meta_path).ok()?;
            serde_json::from_str(&content).ok()
        })
        .collect();
    runs.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    runs.truncate(limit);
    runs
}

/// Read the output log for a run.
pub fn read_output(run_id: &str) -> String {
    let path = runs_dir().join(run_id).join("output.log");
    fs::read_to_string(&path).unwrap_or_default()
}

/// Get the output log file length (for change detection).
pub fn output_len(run_id: &str) -> u64 {
    let path = runs_dir().join(run_id).join("output.log");
    fs::metadata(&path).map(|m| m.len()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_trigger_kinds() {
        let (kind, _) = classify_trigger("[scan]");
        assert_eq!(kind, "scan");

        let (kind, _) = classify_trigger("[message] hello");
        assert_eq!(kind, "message");

        let (kind, _) = classify_trigger("[new-task] foo.md");
        assert_eq!(kind, "new-task");

        let (kind, _) = classify_trigger("something else");
        assert_eq!(kind, "other");
    }

    #[test]
    fn classify_trigger_truncates_summary() {
        let long = "x".repeat(200);
        let (_, summary) = classify_trigger(&long);
        assert_eq!(summary.len(), 80);
    }

    #[test]
    fn run_meta_serde_round_trip() {
        let meta = RunMeta {
            id: "123-scan".into(),
            started_at: 1000,
            finished_at: Some(2000),
            exit_code: Some(0),
            trigger_kind: "scan".into(),
            trigger_summary: "test".into(),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let parsed: RunMeta =
            serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "123-scan");
        assert_eq!(parsed.finished_at, Some(2000));
        assert_eq!(parsed.exit_code, Some(0));
    }

    #[test]
    fn run_meta_defaults_optional_fields() {
        let json = r#"{"id":"x","started_at":0,"trigger_kind":"scan","trigger_summary":"t"}"#;
        let meta: RunMeta = serde_json::from_str(json).unwrap();
        assert_eq!(meta.finished_at, None);
        assert_eq!(meta.exit_code, None);
    }
}

/// Prune old runs (>7 days or >200 total).
pub fn prune_old_runs() {
    let dir = runs_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };

    let cutoff = now_epoch().saturating_sub(7 * 24 * 3600);
    let mut dirs: Vec<(PathBuf, u64)> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let meta_path = e.path().join("meta.json");
            let content = fs::read_to_string(&meta_path).ok()?;
            let meta: RunMeta = serde_json::from_str(&content).ok()?;
            Some((e.path(), meta.started_at))
        })
        .collect();

    dirs.sort_by(|a, b| b.1.cmp(&a.1));

    for (i, (path, ts)) in dirs.iter().enumerate() {
        if *ts < cutoff || i >= 200 {
            let _ = fs::remove_dir_all(path);
        }
    }
}
