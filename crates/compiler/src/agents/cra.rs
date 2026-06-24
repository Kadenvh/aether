//! Critic-Reviewer Agent (CRA) — U11 (R5, KTD7).
//!
//! Turns *structured* rustc diagnostics into a minimal, localized patch plus a
//! short natural-language lesson (Reflexion self-reflection). Uses Opus at high
//! effort — repair is the high-judgment step. The lesson is fed back into the
//! next Compiler-Agent attempt and recorded in the correction log (R10).

use serde::Deserialize;
use serde_json::json;

use aether_sdk::Result;

use crate::llm::{CompletionRequest, Effort, LlmClient, Message, Model, ToolDef, Transport};
use crate::synth::repair::Repair;
use crate::synth::Diagnostic;

const CRA_SYSTEM_PROMPT: &str = include_str!("prompts/cra_system.txt");
const TOOL_NAME: &str = "emit_repair";
const MAX_TOKENS: u32 = 8192;

#[derive(Debug, Clone, Deserialize)]
struct RepairOut {
    rust_source: String,
    lesson: String,
}

/// The Critic-Reviewer Agent over a given [`Transport`].
pub struct CriticAgent<T: Transport> {
    client: LlmClient<T>,
}

impl<T: Transport> CriticAgent<T> {
    pub fn new(client: LlmClient<T>) -> Self {
        CriticAgent { client }
    }

    /// Produce a localized fix for `prev_source` given the compiler diagnostics.
    pub async fn repair(&self, prev_source: &str, diagnostics: &[Diagnostic]) -> Result<Repair> {
        let mut req = CompletionRequest::new(Model::Opus48, MAX_TOKENS);
        req.system = Some(CRA_SYSTEM_PROMPT.to_string());
        req.messages
            .push(Message::user(build_prompt(prev_source, diagnostics)));
        req.tools.push(repair_tool());
        req.effort = Some(Effort::High);
        req.thinking_adaptive = true;
        let out: RepairOut = self.client.complete_tool(req, TOOL_NAME).await?;
        Ok(Repair {
            rust_source: out.rust_source,
            lesson: out.lesson,
        })
    }
}

fn build_prompt(prev_source: &str, diagnostics: &[Diagnostic]) -> String {
    let diags = diagnostics
        .iter()
        .map(|d| {
            format!(
                "  [{}] {}: {}",
                d.level,
                d.code.as_deref().unwrap_or("?"),
                d.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "CURRENT SOURCE:\n```rust\n{prev_source}\n```\n\n\
         COMPILER DIAGNOSTICS (structured):\n{diags}\n\n\
         Return the corrected full source and a one-line lesson via `emit_repair`.",
    )
}

fn repair_tool() -> ToolDef {
    ToolDef {
        name: TOOL_NAME.into(),
        description: "Emit the corrected Rust source and a concise lesson learned.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "rust_source": {"type": "string"},
                "lesson": {"type": "string"}
            },
            "required": ["rust_source", "lesson"],
            "additionalProperties": false
        }),
        strict: true,
    }
}
