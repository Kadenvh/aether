//! System Architect Agent (SAA) — U10 (R5, KTD3).
//!
//! Decomposes a validated [`Intent`] into a t-DAG via Claude. The model returns
//! structured output through **strict tool use**, so its answer deserializes
//! directly into the U9 [`TDag`] types; the result is then validated for
//! acyclicity by [`ExecutionGraph::build`] before it can be used.
//!
//! The agent designs *structure only*. It receives the resolved invariant set
//! as read-only context and never authors invariants (KTD3) — the frozen system
//! prompt enforces this, and nothing in the tool schema lets it add invariants.

use serde_json::json;

use aether_sdk::types::{Intent, TDag};
use aether_sdk::Result;

use crate::llm::{CompletionRequest, Effort, LlmClient, Message, Model, ToolDef, Transport};
use crate::tdag::ExecutionGraph;

/// Frozen system prompt — kept byte-stable so the prompt-cache prefix hits.
const SAA_SYSTEM_PROMPT: &str = include_str!("prompts/saa_system.txt");
const TOOL_NAME: &str = "emit_tdag";
const MAX_TOKENS: u32 = 8192;

/// The System Architect Agent over a given [`Transport`].
pub struct SystemArchitect<T: Transport> {
    client: LlmClient<T>,
}

impl<T: Transport> SystemArchitect<T> {
    pub fn new(client: LlmClient<T>) -> Self {
        SystemArchitect { client }
    }

    /// Plan a t-DAG for `intent`.
    ///
    /// `known_invariants` are the already-resolved invariant references (context
    /// only — the agent cannot extend them); `ledger_summary` is a read-only
    /// synopsis of relevant current state. Returns a structurally valid, acyclic
    /// [`TDag`] or an error if the model refuses, returns malformed structure, or
    /// proposes a cyclic graph.
    pub async fn plan(
        &self,
        intent: &Intent,
        known_invariants: &[String],
        ledger_summary: &str,
    ) -> Result<TDag> {
        let mut req = CompletionRequest::new(Model::Opus48, MAX_TOKENS);
        req.system = Some(SAA_SYSTEM_PROMPT.to_string());
        req.messages.push(Message::user(build_user_prompt(
            intent,
            known_invariants,
            ledger_summary,
        )));
        req.tools.push(tdag_tool());
        req.effort = Some(Effort::High);
        req.thinking_adaptive = true;

        let tdag: TDag = self.client.complete_tool(req, TOOL_NAME).await?;
        // Construction-time acyclicity + dangling-edge / duplicate-id checks (U9).
        ExecutionGraph::build(&tdag)?;
        Ok(tdag)
    }
}

/// Assemble the per-request user message. Variable content lives here so the
/// system prompt above stays cache-stable.
fn build_user_prompt(intent: &Intent, known_invariants: &[String], ledger_summary: &str) -> String {
    let invariants = if known_invariants.is_empty() {
        "(none)".to_string()
    } else {
        known_invariants
            .iter()
            .map(|i| format!("  - {i}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "OBJECTIVE:\n{objective}\n\n\
         FIXED SAFETY INVARIANTS (read-only context — do not modify or add):\n{invariants}\n\n\
         CURRENT LEDGER STATE (summary):\n{ledger}\n\n\
         Emit the t-DAG that achieves the objective.",
        objective = intent.objective,
        invariants = invariants,
        ledger = ledger_summary,
    )
}

/// Strict tool whose `input` deserializes into [`TDag`]. Node `spec` is omitted
/// (it defaults) so the schema stays closed-world for strict validation; the
/// Compiler Agent derives per-node detail later (U11/U15).
fn tdag_tool() -> ToolDef {
    ToolDef {
        name: TOOL_NAME.into(),
        description: "Emit the temporal DAG of execution nodes and their \
                      data/temporal dependencies that fulfills the objective."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "nodes": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "string"},
                            "kind": {
                                "type": "string",
                                "enum": ["ingest", "transform", "flag", "persist", "api_sync"]
                            }
                        },
                        "required": ["id", "kind"],
                        "additionalProperties": false
                    }
                },
                "edges": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "from": {"type": "string"},
                            "to": {"type": "string"},
                            "kind": {"type": "string", "enum": ["data_flow", "temporal"]}
                        },
                        "required": ["from", "to", "kind"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["nodes", "edges"],
            "additionalProperties": false
        }),
        strict: true,
    }
}
