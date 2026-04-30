//! Linear API integration (Phase 4b).
//!
//! Single workspace per `LINEAR_API_KEY` env var. Issue lookups use
//! the human-readable identifier (e.g. `ENG-29535`). Linear's
//! GraphQL `issue(id:)` accepts both UUIDs and identifiers, so we
//! pass the latter directly.
//!
//! No caching here — `cache.rs` owns persistence; this module just
//! wraps the HTTP/GraphQL surface.

use std::time::Duration;

use serde::Deserialize;

const ENDPOINT: &str = "https://api.linear.app/graphql";
const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LinearIssue {
    pub identifier: String,
    pub title: String,
    #[serde(default)]
    pub state: Option<IssueState>,
    #[serde(default)]
    pub assignee: Option<IssueAssignee>,
    #[serde(default, rename = "updatedAt")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct IssueState {
    #[serde(default)]
    pub name: String,
    /// "backlog" | "unstarted" | "started" | "completed" | "canceled" | "triage"
    #[serde(default, rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct IssueAssignee {
    #[serde(default, rename = "displayName")]
    pub display_name: String,
}

pub fn api_key_from_env() -> Option<String> {
    std::env::var("LINEAR_API_KEY")
        .ok()
        .filter(|s| !s.is_empty())
}

/// Fetch a single issue by identifier (e.g. "ENG-29535"). Returns
/// `Ok(None)` if the issue doesn't exist (Linear returns `null`),
/// `Err` on auth/transport/parse failure.
pub fn fetch_issue(api_key: &str, identifier: &str) -> Result<Option<LinearIssue>, String> {
    let query = r#"query($id: String!) {
        issue(id: $id) {
            identifier
            title
            state { name type }
            assignee { displayName }
            updatedAt
        }
    }"#;

    let body = serde_json::json!({
        "query": query,
        "variables": { "id": identifier }
    });

    let resp = ureq::post(ENDPOINT)
        .set("Authorization", api_key)
        .set("Content-Type", "application/json")
        .timeout(TIMEOUT)
        .send_json(body)
        .map_err(|e| format!("http: {e}"))?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("parse: {e}"))?;

    if let Some(errors) = json.get("errors") {
        return Err(format!("graphql: {errors}"));
    }

    let issue_val = match json.get("data").and_then(|d| d.get("issue")) {
        Some(v) if !v.is_null() => v.clone(),
        _ => return Ok(None),
    };

    let issue: LinearIssue = serde_json::from_value(issue_val)
        .map_err(|e| format!("decode: {e}"))?;
    Ok(Some(issue))
}

/// Batch-fetch multiple issues in a single GraphQL request via
/// aliased `issue(id:)` selections. Returns a map from identifier
/// to issue (missing issues are absent from the map). Errors apply
/// to the whole batch — partial success is not reported separately.
pub fn fetch_many(
    api_key: &str,
    identifiers: &[String],
) -> Result<std::collections::HashMap<String, LinearIssue>, String> {
    use std::collections::HashMap;
    if identifiers.is_empty() {
        return Ok(HashMap::new());
    }

    // Build aliased selection set: `i0: issue(id: "ENG-1") { ... }`.
    let mut selections = String::new();
    for (i, id) in identifiers.iter().enumerate() {
        // Keys must be valid GraphQL aliases — strip non-alphanumerics
        // and prefix to keep them valid.
        selections.push_str(&format!(
            r#"i{i}: issue(id: "{}") {{ identifier title state {{ name type }} assignee {{ displayName }} updatedAt }} "#,
            escape_graphql(id),
        ));
    }
    let query = format!("query {{ {selections} }}");
    let body = serde_json::json!({ "query": query });

    let resp = ureq::post(ENDPOINT)
        .set("Authorization", api_key)
        .set("Content-Type", "application/json")
        .timeout(TIMEOUT)
        .send_json(body)
        .map_err(|e| format!("http: {e}"))?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("parse: {e}"))?;

    if let Some(errors) = json.get("errors") {
        // Linear returns errors-and-data when individual issues are
        // missing; only treat top-level error as fatal when there's
        // no data at all.
        if json.get("data").map(|d| d.is_null()).unwrap_or(true) {
            return Err(format!("graphql: {errors}"));
        }
    }

    let data = json.get("data").cloned().unwrap_or(serde_json::Value::Null);
    let mut out = HashMap::new();
    for i in 0..identifiers.len() {
        let key = format!("i{i}");
        let val = match data.get(&key) {
            Some(v) if !v.is_null() => v.clone(),
            _ => continue,
        };
        if let Ok(issue) = serde_json::from_value::<LinearIssue>(val) {
            out.insert(identifiers[i].clone(), issue);
        }
    }
    Ok(out)
}

fn escape_graphql(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_graphql_handles_quotes() {
        assert_eq!(escape_graphql("ENG-1"), "ENG-1");
        assert_eq!(escape_graphql(r#"a"b"#), r#"a\"b"#);
        assert_eq!(escape_graphql(r"a\b"), r"a\\b");
    }

    #[test]
    fn api_key_from_env_filters_empty() {
        unsafe { std::env::set_var("LINEAR_API_KEY", ""); }
        assert!(api_key_from_env().is_none());
        unsafe { std::env::set_var("LINEAR_API_KEY", "lin_api_xyz"); }
        assert_eq!(api_key_from_env(), Some("lin_api_xyz".into()));
        unsafe { std::env::remove_var("LINEAR_API_KEY"); }
        assert!(api_key_from_env().is_none());
    }
}
