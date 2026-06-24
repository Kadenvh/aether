//! Compiler Agent (CA) — U11 (R5).
//!
//! Synthesizes the Rust source and WIT interface for a single t-DAG node from
//! the node spec, plus any lessons accumulated by the Critic-Reviewer on prior
//! attempts. Uses Haiku by default (KTD6: cheap mechanical generation) with
//! strict tool use so the output deserializes directly.

use serde::Deserialize;
use serde_json::json;

use aether_sdk::Result;

use crate::llm::{CompletionRequest, Effort, LlmClient, Message, Model, ToolDef, Transport};

const CA_SYSTEM_PROMPT: &str = include_str!("prompts/ca_system.txt");
const TOOL_NAME: &str = "emit_node";
const MAX_TOKENS: u32 = 8192;

/// A synthesized node: its Rust implementation and its WIT interface.
#[derive(Debug, Clone, Deserialize)]
pub struct GeneratedNode {
    pub rust_source: String,
    pub wit: String,
}

/// The Compiler Agent over a given [`Transport`].
pub struct CompilerAgent<T: Transport> {
    client: LlmClient<T>,
    model: Model,
}

impl<T: Transport> CompilerAgent<T> {
    /// Build with the default (cheap) model tier.
    pub fn new(client: LlmClient<T>) -> Self {
        CompilerAgent {
            client,
            model: Model::Haiku45,
        }
    }

    /// Override the model tier (e.g. escalate to Opus for a hard node).
    pub fn with_model(client: LlmClient<T>, model: Model) -> Self {
        CompilerAgent { client, model }
    }

    /// Generate the node's Rust + WIT. `lessons` are prepended so the agent
    /// avoids mistakes the Critic-Reviewer already diagnosed.
    pub async fn generate(&self, node_spec: &str, lessons: &[String]) -> Result<GeneratedNode> {
        let mut req = CompletionRequest::new(self.model, MAX_TOKENS);
        req.system = Some(CA_SYSTEM_PROMPT.to_string());
        req.messages
            .push(Message::user(build_prompt(node_spec, lessons)));
        req.tools.push(node_tool());
        req.effort = Some(Effort::Medium);
        req.thinking_adaptive = true;
        self.client.complete_tool(req, TOOL_NAME).await
    }
}

fn build_prompt(node_spec: &str, lessons: &[String]) -> String {
    let lessons_block = if lessons.is_empty() {
        "(none yet)".to_string()
    } else {
        lessons
            .iter()
            .map(|l| format!("  - {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "NODE SPEC:\n{node_spec}\n\n\
         LESSONS FROM PRIOR FAILED ATTEMPTS (do not repeat these mistakes):\n{lessons_block}\n\n\
         Emit the Rust implementation and its WIT interface via `emit_node`.",
    )
}

fn node_tool() -> ToolDef {
    ToolDef {
        name: TOOL_NAME.into(),
        description: "Emit the Rust source and WIT interface implementing this node.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "rust_source": {"type": "string"},
                "wit": {"type": "string"}
            },
            "required": ["rust_source", "wit"],
            "additionalProperties": false
        }),
        strict: true,
    }
}
