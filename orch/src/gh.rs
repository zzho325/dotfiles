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

fn fetch_pr(number: u32) -> Option<PrData> {
    let output = Command::new("gh")
        .args([
            "pr", "view",
            &number.to_string(),
            "--json",
            "number,title,statusCheckRollup,reviews,comments",
        ])
        .stderr(Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).ok()?;

    let title = json["title"].as_str().unwrap_or("").to_string();

    // CI status: check statusCheckRollup
    let ci_pass = json["statusCheckRollup"]
        .as_array()
        .map(|checks| {
            checks.iter().all(|c| {
                c["conclusion"].as_str() == Some("SUCCESS")
                    || c["state"].as_str() == Some("SUCCESS")
            })
        });

    // Approvals: any review with APPROVED state
    let approved = json["reviews"]
        .as_array()
        .is_some_and(|reviews| {
            reviews
                .iter()
                .any(|r| r["state"].as_str() == Some("APPROVED"))
        });

    // Codex reviewed: check comments for codex bot
    let codex_reviewed = json["comments"]
        .as_array()
        .is_some_and(|comments| {
            comments.iter().any(|c| {
                let author = c["author"]["login"].as_str().unwrap_or("");
                author.contains("codex") || author.contains("openai")
            })
        });

    Some(PrData {
        number,
        title,
        ci_pass,
        approved,
        codex_reviewed,
    })
}
