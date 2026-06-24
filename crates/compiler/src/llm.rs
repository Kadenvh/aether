//! Claude Messages API client (KTD6).
//!
//! Rust has no official Anthropic SDK, so AETHER talks to `/v1/messages` over
//! raw HTTP. The [`Transport`] trait isolates the network so request-building,
//! tool-use parsing, refusal handling, and retry are all testable without a
//! socket. Structured agent output (the t-DAG in U10, node code in U11) uses
//! **strict tool use** so responses validate into typed Rust.

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

use aether_sdk::{AetherError, Result};

const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

// ---------------------------------------------------------------------------
// Request vocabulary
// ---------------------------------------------------------------------------

/// The model tier driving an agent. Opus for the high-judgment System Architect
/// and Critic agents; Haiku for cheap mechanical Compiler-agent passes (KTD6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    Opus48,
    Haiku45,
}

impl Model {
    pub fn as_str(self) -> &'static str {
        match self {
            Model::Opus48 => "claude-opus-4-8",
            Model::Haiku45 => "claude-haiku-4-5",
        }
    }
}

/// `output_config.effort` — depth/cost control. Default `high` if omitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::Xhigh => "xhigh",
            Effort::Max => "max",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Message { role: "user".into(), content: content.into() }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Message { role: "assistant".into(), content: content.into() }
    }
}

/// A strict tool definition. `input_schema` should set
/// `"additionalProperties": false` and list `"required"` so `strict: true`
/// guarantees the model's `tool_use.input` validates exactly.
#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub strict: bool,
}

/// A single completion request, transport-agnostic.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub model: Model,
    pub max_tokens: u32,
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDef>,
    /// Force a specific tool (`tool_choice: {type:"tool", name}`).
    pub force_tool: Option<String>,
    pub effort: Option<Effort>,
    pub thinking_adaptive: bool,
    pub stream: bool,
}

impl CompletionRequest {
    pub fn new(model: Model, max_tokens: u32) -> Self {
        CompletionRequest {
            model,
            max_tokens,
            system: None,
            messages: Vec::new(),
            tools: Vec::new(),
            force_tool: None,
            effort: None,
            thinking_adaptive: false,
            stream: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Response vocabulary
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ToolUse {
    pub name: String,
    pub input: Value,
}

/// The interpreted result of a completion.
#[derive(Debug, Clone)]
pub struct Completion {
    pub model: String,
    pub stop_reason: String,
    pub text: Option<String>,
    pub tool_uses: Vec<ToolUse>,
}

impl Completion {
    /// True when the safety classifiers (or the model) declined the request.
    pub fn is_refusal(&self) -> bool {
        self.stop_reason == "refusal"
    }
}

// ---------------------------------------------------------------------------
// Transport — one network attempt. Retry/backoff lives in LlmClient so it is
// exercised by tests with a mock transport (no socket).
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct TransportError {
    pub status: Option<u16>,
    pub retryable: bool,
    pub message: String,
}

#[async_trait]
pub trait Transport: Send + Sync {
    async fn post_messages(&self, body: &Value) -> std::result::Result<Value, TransportError>;
}

#[derive(Debug, Clone, Copy)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub base_delay_ms: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        RetryConfig { max_retries: 2, base_delay_ms: 500 }
    }
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

pub struct LlmClient<T: Transport> {
    transport: T,
    retry: RetryConfig,
}

impl<T: Transport> LlmClient<T> {
    pub fn new(transport: T) -> Self {
        LlmClient { transport, retry: RetryConfig::default() }
    }

    pub fn with_retry(transport: T, retry: RetryConfig) -> Self {
        LlmClient { transport, retry }
    }

    /// Build the exact `/v1/messages` JSON body. Pure and deterministic so the
    /// prompt prefix stays byte-stable for prompt-cache hits.
    pub fn build_request(&self, req: &CompletionRequest) -> Value {
        let mut body = json!({
            "model": req.model.as_str(),
            "max_tokens": req.max_tokens,
            "messages": req.messages.iter().map(|m| json!({"role": m.role, "content": m.content})).collect::<Vec<_>>(),
        });
        let map = body.as_object_mut().expect("object");
        if let Some(system) = &req.system {
            map.insert("system".into(), json!(system));
        }
        if req.thinking_adaptive {
            map.insert("thinking".into(), json!({"type": "adaptive"}));
        }
        if let Some(effort) = req.effort {
            map.insert("output_config".into(), json!({"effort": effort.as_str()}));
        }
        if !req.tools.is_empty() {
            let tools: Vec<Value> = req
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.input_schema,
                        "strict": t.strict,
                    })
                })
                .collect();
            map.insert("tools".into(), json!(tools));
        }
        if let Some(name) = &req.force_tool {
            map.insert("tool_choice".into(), json!({"type": "tool", "name": name}));
        }
        if req.stream {
            map.insert("stream".into(), json!(true));
        }
        body
    }

    /// Send a request, retrying retryable transport failures with exponential
    /// backoff, then interpret the response.
    pub async fn complete(&self, req: &CompletionRequest) -> Result<Completion> {
        let body = self.build_request(req);
        let mut attempt = 0u32;
        loop {
            match self.transport.post_messages(&body).await {
                Ok(value) => return Self::parse_response(value),
                Err(err) if err.retryable && attempt < self.retry.max_retries => {
                    let delay = self.retry.base_delay_ms.saturating_mul(1u64 << attempt);
                    if delay > 0 {
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    }
                    attempt += 1;
                }
                Err(err) => {
                    return Err(AetherError::Llm(format!(
                        "transport error (status {:?}, {} attempt(s)): {}",
                        err.status,
                        attempt + 1,
                        err.message
                    )));
                }
            }
        }
    }

    /// Force the named strict tool and deserialize its `input` into `O`. Errors
    /// on refusal, on a missing tool call, or on a schema mismatch.
    pub async fn complete_tool<O: DeserializeOwned>(
        &self,
        mut req: CompletionRequest,
        tool_name: &str,
    ) -> Result<O> {
        req.force_tool = Some(tool_name.to_string());
        let completion = self.complete(&req).await?;
        if completion.is_refusal() {
            return Err(AetherError::LlmRefusal(format!(
                "model declined the request for tool '{tool_name}'"
            )));
        }
        let tool_use = completion
            .tool_uses
            .into_iter()
            .find(|t| t.name == tool_name)
            .ok_or_else(|| {
                AetherError::Llm(format!("response contained no '{tool_name}' tool_use block"))
            })?;
        serde_json::from_value(tool_use.input)
            .map_err(|e| AetherError::Llm(format!("tool '{tool_name}' output failed schema: {e}")))
    }

    /// Interpret a raw `/v1/messages` JSON response. Checks `stop_reason` before
    /// touching `content` so a refusal (empty content) never panics.
    fn parse_response(value: Value) -> Result<Completion> {
        let model = value
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let stop_reason = value
            .get("stop_reason")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        let mut text: Option<String> = None;
        let mut tool_uses = Vec::new();
        if let Some(blocks) = value.get("content").and_then(Value::as_array) {
            for block in blocks {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(t) = block.get("text").and_then(Value::as_str) {
                            text.get_or_insert_with(String::new).push_str(t);
                        }
                    }
                    Some("tool_use") => {
                        let name = block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        let input = block.get("input").cloned().unwrap_or(Value::Null);
                        tool_uses.push(ToolUse { name, input });
                    }
                    _ => {}
                }
            }
        }

        Ok(Completion { model, stop_reason, text, tool_uses })
    }
}

// ---------------------------------------------------------------------------
// Real network transport
// ---------------------------------------------------------------------------

/// Resolve the API key from a lookup function. Pure, so the env-handling rule
/// (no hardcoded fallback) is testable without mutating process env.
pub fn resolve_api_key<F: Fn(&str) -> Option<String>>(lookup: F) -> Result<String> {
    match lookup("ANTHROPIC_API_KEY") {
        Some(key) if !key.trim().is_empty() => Ok(key),
        _ => Err(AetherError::Llm(
            "ANTHROPIC_API_KEY is not set; refusing to call the API without a key".into(),
        )),
    }
}

pub struct ReqwestTransport {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl ReqwestTransport {
    pub fn new(api_key: impl Into<String>) -> Self {
        ReqwestTransport {
            client: reqwest::Client::new(),
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
        }
    }

    /// Construct from the `ANTHROPIC_API_KEY` env var; errors clearly if absent.
    pub fn from_env() -> Result<Self> {
        let key = resolve_api_key(|k| std::env::var(k).ok())?;
        Ok(Self::new(key))
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
}

#[async_trait]
impl Transport for ReqwestTransport {
    async fn post_messages(&self, body: &Value) -> std::result::Result<Value, TransportError> {
        let url = format!("{}/v1/messages", self.base_url);
        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(body)
            .send()
            .await
            .map_err(|e| TransportError {
                status: None,
                retryable: true,
                message: format!("request failed: {e}"),
            })?;

        let status = resp.status();
        if status.is_success() {
            resp.json::<Value>().await.map_err(|e| TransportError {
                status: Some(status.as_u16()),
                retryable: false,
                message: format!("invalid JSON response: {e}"),
            })
        } else {
            let code = status.as_u16();
            let retryable = code == 429 || code >= 500;
            let detail = resp.text().await.unwrap_or_default();
            Err(TransportError { status: Some(code), retryable, message: detail })
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    /// Returns a scripted result and counts how many times it was called.
    struct MockTransport {
        responses: std::sync::Mutex<Vec<std::result::Result<Value, TransportError>>>,
        calls: Arc<AtomicU32>,
    }

    impl MockTransport {
        fn new(
            responses: Vec<std::result::Result<Value, TransportError>>,
        ) -> (Self, Arc<AtomicU32>) {
            let calls = Arc::new(AtomicU32::new(0));
            (
                MockTransport { responses: std::sync::Mutex::new(responses), calls: calls.clone() },
                calls,
            )
        }
    }

    #[async_trait]
    impl Transport for MockTransport {
        async fn post_messages(&self, _body: &Value) -> std::result::Result<Value, TransportError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut responses = self.responses.lock().unwrap();
            if responses.len() == 1 {
                // Sticky last response (for "always retryable" tests).
                match &responses[0] {
                    Ok(v) => Ok(v.clone()),
                    Err(e) => Err(TransportError {
                        status: e.status,
                        retryable: e.retryable,
                        message: e.message.clone(),
                    }),
                }
            } else {
                responses.remove(0)
            }
        }
    }

    fn no_delay() -> RetryConfig {
        RetryConfig { max_retries: 2, base_delay_ms: 0 }
    }

    fn tdag_tool() -> ToolDef {
        ToolDef {
            name: "emit_tdag".into(),
            description: "Emit the temporal DAG".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "node_count": {"type": "integer"} },
                "required": ["node_count"],
                "additionalProperties": false
            }),
            strict: true,
        }
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct TDagOut {
        node_count: u32,
    }

    #[tokio::test]
    async fn build_request_includes_strict_tool_thinking_and_effort() {
        let (mock, _) = MockTransport::new(vec![Ok(json!({}))]);
        let client = LlmClient::new(mock);
        let mut req = CompletionRequest::new(Model::Opus48, 4096);
        req.system = Some("system".into());
        req.messages.push(Message::user("hi"));
        req.tools.push(tdag_tool());
        req.force_tool = Some("emit_tdag".into());
        req.effort = Some(Effort::High);
        req.thinking_adaptive = true;

        let body = client.build_request(&req);
        assert_eq!(body["model"], "claude-opus-4-8");
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["output_config"]["effort"], "high");
        assert_eq!(body["tools"][0]["strict"], true);
        assert_eq!(body["tools"][0]["input_schema"]["additionalProperties"], false);
        assert_eq!(body["tool_choice"]["type"], "tool");
        assert_eq!(body["tool_choice"]["name"], "emit_tdag");
    }

    #[tokio::test]
    async fn complete_tool_parses_tool_use_into_typed_struct() {
        let response = json!({
            "model": "claude-opus-4-8",
            "stop_reason": "tool_use",
            "content": [
                {"type": "tool_use", "name": "emit_tdag", "input": {"node_count": 4}}
            ]
        });
        let (mock, _) = MockTransport::new(vec![Ok(response)]);
        let client = LlmClient::new(mock);
        let mut req = CompletionRequest::new(Model::Opus48, 4096);
        req.tools.push(tdag_tool());
        let out: TDagOut = client.complete_tool(req, "emit_tdag").await.unwrap();
        assert_eq!(out, TDagOut { node_count: 4 });
    }

    #[tokio::test]
    async fn complete_tool_rejects_schema_violation() {
        // tool_use input is missing the required `node_count` field.
        let response = json!({
            "model": "claude-opus-4-8",
            "stop_reason": "tool_use",
            "content": [{"type": "tool_use", "name": "emit_tdag", "input": {"wrong": 1}}]
        });
        let (mock, _) = MockTransport::new(vec![Ok(response)]);
        let client = LlmClient::new(mock);
        let mut req = CompletionRequest::new(Model::Opus48, 4096);
        req.tools.push(tdag_tool());
        let result: Result<TDagOut> = client.complete_tool(req, "emit_tdag").await;
        assert!(matches!(result, Err(AetherError::Llm(_))));
    }

    #[tokio::test]
    async fn refusal_surfaces_typed_error_not_panic() {
        // stop_reason refusal with EMPTY content — must not index-panic.
        let response = json!({
            "model": "claude-opus-4-8",
            "stop_reason": "refusal",
            "content": []
        });
        let (mock, _) = MockTransport::new(vec![Ok(response)]);
        let client = LlmClient::new(mock);
        let mut req = CompletionRequest::new(Model::Opus48, 4096);
        req.tools.push(tdag_tool());
        let result: Result<TDagOut> = client.complete_tool(req, "emit_tdag").await;
        assert!(matches!(result, Err(AetherError::LlmRefusal(_))));
    }

    #[tokio::test]
    async fn retries_then_gives_up_after_cap() {
        let err = || {
            Err(TransportError {
                status: Some(429),
                retryable: true,
                message: "rate limited".into(),
            })
        };
        let (mock, calls) = MockTransport::new(vec![err()]); // sticky: always 429
        let client = LlmClient::with_retry(mock, no_delay());
        let req = CompletionRequest::new(Model::Opus48, 256);
        let result = client.complete(&req).await;
        assert!(matches!(result, Err(AetherError::Llm(_))));
        // initial attempt + 2 retries = 3 calls
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retries_then_succeeds() {
        let ok = json!({"model": "claude-opus-4-8", "stop_reason": "end_turn", "content": [{"type": "text", "text": "ok"}]});
        let responses = vec![
            Err(TransportError {
                status: Some(503),
                retryable: true,
                message: "overloaded".into(),
            }),
            Ok(ok),
        ];
        let (mock, calls) = MockTransport::new(responses);
        let client = LlmClient::with_retry(mock, no_delay());
        let req = CompletionRequest::new(Model::Opus48, 256);
        let completion = client.complete(&req).await.unwrap();
        assert_eq!(completion.text.as_deref(), Some("ok"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn resolve_api_key_requires_present_nonempty_key() {
        assert!(resolve_api_key(|_| None).is_err());
        assert!(resolve_api_key(|_| Some("  ".into())).is_err());
        assert_eq!(resolve_api_key(|_| Some("sk-abc".into())).unwrap(), "sk-abc");
    }
}
