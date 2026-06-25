use std::time::{Duration, Instant};

pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8080/api";
pub const DEFAULT_AGENT: &str = "codex";
pub const DEFAULT_CLAUDE_AGENT: &str = "claude-code";
pub const DEFAULT_MODEL: &str = "gpt-5.5";
pub const DEFAULT_CLAUDE_MODEL: &str = "claude-opus-4-8";
pub const DEFAULT_EFFORT: &str = "xhigh";

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub agent: String,
    pub model: String,
    pub effort_level: String,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

impl AgentConfig {
    pub fn from_env() -> Self {
        Self {
            agent: env_or("ORCH_REMOTE_AGENT_AGENT", DEFAULT_AGENT),
            model: env_or("ORCH_REMOTE_AGENT_MODEL", DEFAULT_MODEL),
            effort_level: env_or("ORCH_REMOTE_AGENT_EFFORT", DEFAULT_EFFORT),
        }
    }

    pub fn codex(model: &str, effort: &str) -> Self {
        Self {
            agent: DEFAULT_AGENT.to_string(),
            model: model.to_string(),
            effort_level: effort.to_string(),
        }
    }

    pub fn claude_code(model: &str, effort: &str) -> Self {
        Self {
            agent: DEFAULT_CLAUDE_AGENT.to_string(),
            model: model.to_string(),
            effort_level: effort.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RemoteAgentClient {
    base_url: String,
    http: ureq::Agent,
}

#[derive(Debug, Clone)]
pub struct AskOptions {
    pub config: AgentConfig,
    pub timeout: Duration,
}

impl Default for AskOptions {
    fn default() -> Self {
        Self {
            config: AgentConfig::default(),
            timeout: Duration::from_secs(45 * 60),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AskResult {
    pub session_id: String,
    pub answer: String,
    pub transcript_jsonl: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ListSessionsResponse {
    #[serde(default)]
    pub sessions: Vec<SessionResponse>,
    #[serde(default)]
    pub next_cursor: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionResponse {
    pub session_id: String,
    #[serde(default)]
    pub source: String,
    pub status: String,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub last_active_at: u64,
    #[serde(default)]
    pub options: SessionOptions,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SessionOptions {
    #[serde(default)]
    pub agent_config: Option<AgentConfigResponse>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AgentConfigResponse {
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub effort_level: String,
}

#[derive(Debug, serde::Deserialize)]
struct TranscriptEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}

impl RemoteAgentClient {
    pub fn from_env() -> Self {
        let http = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(180))
            .build();
        Self {
            base_url: env_or("ORCH_REMOTE_AGENT_API_URL", DEFAULT_BASE_URL)
                .trim_end_matches('/')
                .to_string(),
            http,
        }
    }

    pub fn create_session(&self, config: &AgentConfig) -> Result<SessionResponse, String> {
        let body = serde_json::json!({
            "agent_config": {
                "agent": config.agent,
                "model": config.model,
                "effort_level": config.effort_level,
            },
        });
        let resp = self
            .http
            .post(&format!("{}/sessions", self.base_url))
            .send_json(body)
            .map_err(error_message)?;
        decode_session_response(resp)
    }

    pub fn list_sessions(
        &self,
        limit: u32,
        source: Option<&str>,
    ) -> Result<ListSessionsResponse, String> {
        let limit = limit.clamp(1, 100).to_string();
        let mut request = self
            .http
            .get(&format!("{}/sessions", self.base_url))
            .query("limit", &limit);
        if let Some(source) = source.filter(|s| !s.trim().is_empty()) {
            request = request.query("source", source);
        }
        request
            .call()
            .map_err(error_message)?
            .into_json()
            .map_err(|e| e.to_string())
    }

    pub fn send_message(&self, session_id: &str, message: &str) -> Result<(), String> {
        let body = serde_json::json!({ "message": message });
        self.http
            .post(&format!("{}/sessions/{session_id}/messages", self.base_url))
            .send_json(body)
            .map_err(error_message)?;
        Ok(())
    }

    pub fn get_session(&self, session_id: &str) -> Result<SessionResponse, String> {
        let resp = self
            .http
            .get(&format!("{}/sessions/{session_id}", self.base_url))
            .call()
            .map_err(error_message)?;
        decode_session_response(resp)
    }

    pub fn transcript_jsonl(&self, session_id: &str) -> Result<String, String> {
        self.http
            .get(&format!(
                "{}/sessions/{session_id}/transcript.jsonl",
                self.base_url,
            ))
            .call()
            .map_err(error_message)?
            .into_string()
            .map_err(|e| e.to_string())
    }

    pub fn wait_for_idle_after_message(
        &self,
        session_id: &str,
        timeout: Duration,
    ) -> Result<(), String> {
        let started = Instant::now();
        let mut saw_active = false;
        loop {
            if started.elapsed() > timeout {
                return Err(format!(
                    "timed out waiting for remote session {session_id} to become idle",
                ));
            }

            let session = self.get_session(session_id)?;
            match session.status.as_str() {
                "active" => saw_active = true,
                "idle" => {
                    if let Ok(transcript) = self.transcript_jsonl(session_id) {
                        if last_agent_message(&transcript).is_some() {
                            return Ok(());
                        }
                    }
                    if saw_active {
                        std::thread::sleep(Duration::from_secs(1));
                    }
                }
                "initializing" => {}
                "closed" => return Err(format!("remote session {session_id} is closed")),
                other => {
                    return Err(format!(
                        "remote session {session_id} has unexpected status {other}",
                    ));
                }
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }

    pub fn ask_existing_session(
        &self,
        session_id: &str,
        prompt: &str,
        timeout: Duration,
    ) -> Result<AskResult, String> {
        self.send_message(session_id, prompt)?;
        self.wait_for_idle_after_message(session_id, timeout)?;
        let transcript_jsonl = self.transcript_jsonl(session_id)?;
        let answer = last_agent_message(&transcript_jsonl)
            .ok_or_else(|| "remote transcript had no agent response".to_string())?;
        Ok(AskResult {
            session_id: session_id.to_string(),
            answer,
            transcript_jsonl,
        })
    }
}

pub fn last_agent_message(transcript_jsonl: &str) -> Option<String> {
    let mut current = String::new();
    let mut last = None;

    let events = serde_json::Deserializer::from_str(transcript_jsonl)
        .into_iter::<TranscriptEvent>()
        .filter_map(Result::ok);
    for event in events {
        match event.kind.as_str() {
            "agent_message_chunk" => current.push_str(&event.text),
            "turn_complete" => {
                if current.trim().is_empty() && !event.text.trim().is_empty() {
                    current.push_str(&event.text);
                }
                let message = current.trim();
                if !message.is_empty() {
                    last = Some(message.to_string());
                }
                current.clear();
            }
            _ => {}
        }
    }

    let message = current.trim();
    if !message.is_empty() {
        last = Some(message.to_string());
    }
    last
}

fn env_or(key: &str, fallback: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn decode_session_response(resp: ureq::Response) -> Result<SessionResponse, String> {
    let value: serde_json::Value = resp.into_json().map_err(|e| e.to_string())?;
    if let Some(session) = value.get("session") {
        return serde_json::from_value(session.clone()).map_err(|e| e.to_string());
    }
    serde_json::from_value(value).map_err(|e| e.to_string())
}

fn error_message(err: ureq::Error) -> String {
    match err {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            if body.trim().is_empty() {
                format!("remote API returned HTTP {code}")
            } else {
                format!("remote API returned HTTP {code}: {}", body.trim())
            }
        }
        ureq::Error::Transport(e) => e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_agent_message_combines_chunks_for_last_turn() {
        let jsonl = r#"
{"type":"user_message_chunk","text":"hi"}
{"type":"agent_message_chunk","text":"first"}
{"type":"agent_message_chunk","text":" turn"}
{"type":"turn_complete","stop_reason":"end_turn"}
{"type":"agent_message_chunk","text":"second"}
{"type":"agent_message_chunk","text":" turn"}
{"type":"turn_complete","stop_reason":"end_turn"}
"#;
        assert_eq!(last_agent_message(jsonl).as_deref(), Some("second turn"),);
    }

    #[test]
    fn last_agent_message_uses_unclosed_current_turn() {
        let jsonl = r#"
{"type":"agent_message_chunk","text":"partial"}
"#;
        assert_eq!(last_agent_message(jsonl).as_deref(), Some("partial"));
    }

    #[test]
    fn last_agent_message_accepts_turn_complete_text() {
        let jsonl = r#"
{"type":"turn_complete","text":"final response","stop_reason":"end_turn"}
"#;
        assert_eq!(last_agent_message(jsonl).as_deref(), Some("final response"),);
    }

    #[test]
    fn last_agent_message_accepts_pretty_consecutive_json_objects() {
        let json = r#"
{
  "text": "hello ",
  "type": "agent_message_chunk"
}
{
  "text": "world",
  "type": "agent_message_chunk"
}
{
  "stop_reason": "end_turn",
  "type": "turn_complete"
}
"#;
        assert_eq!(last_agent_message(json).as_deref(), Some("hello world"));
    }
}
