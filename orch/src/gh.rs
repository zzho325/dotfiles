use std::{
    collections::HashMap,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use crate::state::PrData;

const POLL_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
struct CacheEntry {
    data: PrData,
    fetched: Instant,
}

pub struct PrCache {
    entries: Arc<Mutex<HashMap<u32, CacheEntry>>>,
}

impl PrCache {
    pub fn new() -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn get(&self, pr: u32) -> Option<PrData> {
        let entries = self.entries.lock().ok()?;
        entries.get(&pr).map(|e| e.data.clone())
    }

    /// Trigger a background refresh for a set of PRs.
    pub fn refresh(&self, prs: Vec<u32>) {
        let entries = self.entries.clone();
        thread::spawn(move || {
            for pr in prs {
                // Skip if recently fetched
                {
                    let cache = entries.lock().unwrap();
                    if let Some(entry) = cache.get(&pr) {
                        if entry.fetched.elapsed() < POLL_INTERVAL {
                            continue;
                        }
                    }
                }
                if let Some(data) = fetch_pr(pr) {
                    let mut cache = entries.lock().unwrap();
                    cache.insert(pr, CacheEntry {
                        data,
                        fetched: Instant::now(),
                    });
                }
            }
        });
    }
}

fn is_codex_bot(login: &str) -> bool {
    login.contains("codex") || login.contains("chatgpt")
}

fn check_codex_thumbs(number: u32) -> Option<bool> {
    let output = Command::new("gh")
        .args([
            "api",
            &format!("repos/{{owner}}/{{repo}}/issues/{number}/reactions"),
        ])
        .current_dir(repo_cwd())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let reactions: serde_json::Value =
        serde_json::from_slice(&output.stdout).ok()?;
    let arr = reactions.as_array()?;
    let has_thumb = arr.iter().any(|r| {
        let login = r["user"]["login"].as_str().unwrap_or("");
        let content = r["content"].as_str().unwrap_or("");
        is_codex_bot(login) && content == "+1"
    });
    Some(has_thumb)
}

fn repo_cwd() -> String {
    std::env::var("ORCH_REPO")
        .map(|r| format!("{r}/main"))
        .unwrap_or_else(|_| ".".to_string())
}

fn fetch_pr(number: u32) -> Option<PrData> {
    let output = Command::new("gh")
        .args([
            "pr", "view",
            &number.to_string(),
            "--json",
            "number,title,body,state,mergeable,headRefName,headRefOid,\
             additions,deletions,changedFiles,updatedAt,\
             statusCheckRollup,reviews,comments",
        ])
        .current_dir(repo_cwd())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).ok()?;

    let title = json["title"].as_str().unwrap_or("").to_string();

    // CI status: tristate from statusCheckRollup.
    //   Some(true)  = all checks done and passing
    //   Some(false) = at least one check finished and failed
    //   None        = at least one check still running (no failures yet)
    //
    // CheckRun (GitHub Actions): `status` ∈ {QUEUED, IN_PROGRESS, COMPLETED, ...};
    //   `conclusion` is empty until status=COMPLETED.
    // StatusContext (legacy): `state` ∈ {SUCCESS, PENDING, FAILURE, ERROR}.
    let ci_pass = json["statusCheckRollup"].as_array().and_then(|checks| {
        if checks.is_empty() {
            return None;
        }
        let mut any_pending = false;
        let mut any_fail = false;
        for c in checks {
            let conclusion = c["conclusion"].as_str().filter(|s| !s.is_empty());
            let status = c["status"].as_str();
            let state = c["state"].as_str();
            let success = matches!(conclusion, Some("SUCCESS" | "SKIPPED" | "NEUTRAL"))
                || state == Some("SUCCESS");
            if success {
                continue;
            }
            let pending = matches!(
                status,
                Some("QUEUED" | "IN_PROGRESS" | "WAITING" | "REQUESTED" | "PENDING")
            ) || state == Some("PENDING")
                || matches!(conclusion, Some("ACTION_REQUIRED" | "STALE"));
            if pending {
                any_pending = true;
            } else {
                any_fail = true;
            }
        }
        if any_fail {
            Some(false)
        } else if any_pending {
            None
        } else {
            Some(true)
        }
    });

    // Approvals: any review with APPROVED state
    let approved = json["reviews"]
        .as_array()
        .is_some_and(|reviews| {
            reviews
                .iter()
                .any(|r| r["state"].as_str() == Some("APPROVED"))
        });

    // Codex status: check reviews for codex bot
    let mut codex = crate::state::CodexStatus::None;

    if let Some(reviews) = json["reviews"].as_array() {
        for r in reviews {
            let author = r["author"]["login"].as_str().unwrap_or("");
            if !is_codex_bot(author) {
                continue;
            }
            if r["state"].as_str() == Some("COMMENTED") {
                codex = crate::state::CodexStatus::Commented;
            }
        }
    }

    // Check PR reactions for codex 👍 via issues API
    if codex == crate::state::CodexStatus::None {
        if let Some(has_thumb) = check_codex_thumbs(number) {
            if has_thumb {
                codex = crate::state::CodexStatus::ThumbsUp;
            }
        }
    }

    // Extended metadata for the preview + drill view.
    let state = json["state"].as_str().unwrap_or("").to_string();
    let mergeable = json["mergeable"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from);
    let head_branch = json["headRefName"].as_str().unwrap_or("").to_string();
    let head_sha = json["headRefOid"].as_str().unwrap_or("").to_string();
    let additions = json["additions"].as_u64().unwrap_or(0) as u32;
    let deletions = json["deletions"].as_u64().unwrap_or(0) as u32;
    let changed_files = json["changedFiles"].as_u64().unwrap_or(0) as u32;
    let updated_at = json["updatedAt"].as_str().unwrap_or("").to_string();
    let body = json["body"].as_str().unwrap_or("").to_string();

    Some(PrData {
        number,
        title,
        ci_pass,
        approved,
        codex,
        state,
        mergeable,
        head_branch,
        head_sha,
        additions,
        deletions,
        changed_files,
        updated_at,
        body,
    })
}

/// Fetch the unified diff for a PR via `gh pr diff <num>` and parse it
/// into a `CachedPrDiff`. Honors the size budget — diffs above the raw
/// byte budget come back with `truncated: true` and empty `files`.
///
/// `head_sha` is passed in (from the PR metadata fetch) so the cache can
/// be invalidated when the branch is force-pushed; we don't read it from
/// the diff itself.
pub fn fetch_pr_diff(
    number: u32,
    head_sha: &str,
) -> crate::cache::CachedPrDiff {
    let now = crate::cache::now_epoch();
    let output = Command::new("gh")
        .args(["pr", "diff", &number.to_string()])
        .current_dir(repo_cwd())
        .stderr(Stdio::piped())
        .output();

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            return crate::cache::CachedPrDiff {
                number,
                fetched_at: now,
                head_sha: head_sha.to_string(),
                raw_size: 0,
                truncated: false,
                error: Some(format!("spawn gh: {e}")),
                files: Vec::new(),
            };
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg = stderr
            .lines()
            .next()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "gh pr diff failed".to_string());
        return crate::cache::CachedPrDiff {
            number,
            fetched_at: now,
            head_sha: head_sha.to_string(),
            raw_size: 0,
            truncated: false,
            error: Some(msg),
            files: Vec::new(),
        };
    }

    let raw_size = output.stdout.len() as u64;

    if raw_size > crate::cache::PR_DIFF_RAW_BUDGET {
        return crate::cache::CachedPrDiff {
            number,
            fetched_at: now,
            head_sha: head_sha.to_string(),
            raw_size,
            truncated: true,
            error: None,
            files: Vec::new(),
        };
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let files = parse_unified_diff(&raw);
    crate::cache::CachedPrDiff {
        number,
        fetched_at: now,
        head_sha: head_sha.to_string(),
        raw_size,
        truncated: false,
        error: None,
        files,
    }
}

/// Parse `gh pr diff` (unified diff) output into per-file CachedPrDiffFile
/// records. Per-hunk lines are truncated at PR_DIFF_LINES_PER_HUNK.
fn parse_unified_diff(raw: &str) -> Vec<crate::cache::CachedPrDiffFile> {
    use crate::cache::{CachedPrDiffFile, CachedPrDiffHunk, PR_DIFF_LINES_PER_HUNK};

    let mut files: Vec<CachedPrDiffFile> = Vec::new();
    let mut cur_file: Option<CachedPrDiffFile> = None;
    let mut cur_hunk: Option<CachedPrDiffHunk> = None;
    let mut hunk_truncated_marker_added = false;

    fn finalize_hunk(
        cur_file: &mut Option<CachedPrDiffFile>,
        cur_hunk: &mut Option<CachedPrDiffHunk>,
    ) {
        if let (Some(f), Some(h)) = (cur_file.as_mut(), cur_hunk.take()) {
            f.hunks.push(h);
        }
    }
    fn finalize_file(
        files: &mut Vec<CachedPrDiffFile>,
        cur_file: &mut Option<CachedPrDiffFile>,
        cur_hunk: &mut Option<CachedPrDiffHunk>,
    ) {
        finalize_hunk(cur_file, cur_hunk);
        if let Some(f) = cur_file.take() {
            files.push(f);
        }
    }

    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            finalize_file(&mut files, &mut cur_file, &mut cur_hunk);
            // "a/foo b/foo" — paths can contain spaces (rare; stick to
            // the simple split for now). Default status is "modified";
            // overridden by subsequent header lines.
            let path = rest
                .split_whitespace()
                .nth(1)
                .map(|s| s.strip_prefix("b/").unwrap_or(s).to_string())
                .unwrap_or_default();
            cur_file = Some(CachedPrDiffFile {
                path,
                old_path: None,
                additions: 0,
                deletions: 0,
                status: "modified".into(),
                hunks: Vec::new(),
            });
            hunk_truncated_marker_added = false;
            continue;
        }
        if line.starts_with("index ") || line.starts_with("similarity index") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("rename from ") {
            if let Some(f) = cur_file.as_mut() {
                f.status = "renamed".into();
                f.old_path = Some(rest.to_string());
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("rename to ") {
            if let Some(f) = cur_file.as_mut() {
                f.path = rest.to_string();
            }
            continue;
        }
        if line.starts_with("new file mode") {
            if let Some(f) = cur_file.as_mut() {
                f.status = "added".into();
            }
            continue;
        }
        if line.starts_with("deleted file mode") {
            if let Some(f) = cur_file.as_mut() {
                f.status = "deleted".into();
            }
            continue;
        }
        if line.starts_with("Binary files ") {
            if let Some(f) = cur_file.as_mut() {
                f.status = "binary".into();
            }
            continue;
        }
        if line.starts_with("--- ") || line.starts_with("+++ ") {
            // We use the `diff --git b/path` line as the canonical path.
            // Skip these markers.
            continue;
        }
        if line.starts_with("@@") {
            finalize_hunk(&mut cur_file, &mut cur_hunk);
            cur_hunk = Some(CachedPrDiffHunk {
                header: line.to_string(),
                lines: Vec::new(),
            });
            hunk_truncated_marker_added = false;
            continue;
        }
        if let Some(h) = cur_hunk.as_mut() {
            if h.lines.len() >= PR_DIFF_LINES_PER_HUNK {
                if !hunk_truncated_marker_added {
                    h.lines.push(format!(
                        " (… hunk truncated, exceeded {PR_DIFF_LINES_PER_HUNK} lines)"
                    ));
                    hunk_truncated_marker_added = true;
                }
            } else {
                h.lines.push(line.to_string());
                if let Some(f) = cur_file.as_mut() {
                    if line.starts_with('+') {
                        f.additions += 1;
                    } else if line.starts_with('-') {
                        f.deletions += 1;
                    }
                }
            }
        }
    }

    finalize_file(&mut files, &mut cur_file, &mut cur_hunk);
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_unified_diff_basic_two_files() {
        let raw = "\
diff --git a/foo.go b/foo.go
index 1234567..abcdef0 100644
--- a/foo.go
+++ b/foo.go
@@ -1,3 +1,4 @@
 line1
-line2
+line2-modified
+line3-new
 line4
diff --git a/bar.go b/bar.go
new file mode 100644
index 0000000..abcd
--- /dev/null
+++ b/bar.go
@@ -0,0 +1,2 @@
+hello
+world
";
        let files = parse_unified_diff(raw);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "foo.go");
        assert_eq!(files[0].status, "modified");
        assert_eq!(files[0].additions, 2);
        assert_eq!(files[0].deletions, 1);
        assert_eq!(files[0].hunks.len(), 1);
        assert_eq!(files[1].path, "bar.go");
        assert_eq!(files[1].status, "added");
        assert_eq!(files[1].additions, 2);
    }

    #[test]
    fn parse_unified_diff_rename_with_path_change() {
        let raw = "\
diff --git a/old.go b/new.go
similarity index 90%
rename from old.go
rename to new.go
--- a/old.go
+++ b/new.go
@@ -1 +1 @@
-old
+new
";
        let files = parse_unified_diff(raw);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "new.go");
        assert_eq!(files[0].old_path.as_deref(), Some("old.go"));
        assert_eq!(files[0].status, "renamed");
    }

    #[test]
    fn parse_unified_diff_binary_file() {
        let raw = "\
diff --git a/img.png b/img.png
index 1111111..2222222 100644
Binary files a/img.png and b/img.png differ
";
        let files = parse_unified_diff(raw);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, "binary");
        assert_eq!(files[0].hunks.len(), 0);
    }
}
