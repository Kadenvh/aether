//! U10 verification: the System Architect Agent turns an intent into a valid,
//! acyclic t-DAG. Driven by a recorded `/v1/messages` tool_use fixture — no
//! network, no live model.

use async_trait::async_trait;
use serde_json::{json, Value};

use aether_compiler::agents::saa::SystemArchitect;
use aether_compiler::llm::{LlmClient, Transport, TransportError};
use aether_sdk::types::{Intent, NodeKind};

/// A transport that always returns one recorded response, ignoring the request.
struct FixtureTransport {
    response: Value,
}

#[async_trait]
impl Transport for FixtureTransport {
    async fn post_messages(&self, _body: &Value) -> Result<Value, TransportError> {
        Ok(self.response.clone())
    }
}

/// Recorded SAA response for the blueprint EDID intent: a 4-node linear pipe
/// ingest -> transform -> flag -> persist.
fn edid_tdag_response() -> Value {
    json!({
        "model": "claude-opus-4-8",
        "stop_reason": "tool_use",
        "content": [{
            "type": "tool_use",
            "name": "emit_tdag",
            "input": {
                "nodes": [
                    {"id": "ingest_bills_csv", "kind": "ingest"},
                    {"id": "convert_eur_to_usd", "kind": "transform"},
                    {"id": "flag_variance_anomalies", "kind": "flag"},
                    {"id": "persist_to_ledger", "kind": "persist"}
                ],
                "edges": [
                    {"from": "ingest_bills_csv", "to": "convert_eur_to_usd", "kind": "data_flow"},
                    {"from": "convert_eur_to_usd", "to": "flag_variance_anomalies", "kind": "data_flow"},
                    {"from": "flag_variance_anomalies", "to": "persist_to_ledger", "kind": "data_flow"}
                ]
            }
        }]
    })
}

fn edid_intent() -> Intent {
    Intent {
        objective: "Import partner utility bills, convert Euros to USD, flag lines \
                    with anomalous variance > 20% vs historical average, save to ledger"
            .into(),
        invariants: vec!["usd_amount >= 0.0".into()],
        input: None,
        output: None,
    }
}

#[tokio::test]
async fn plans_valid_acyclic_tdag_for_edid_intent() {
    let transport = FixtureTransport {
        response: edid_tdag_response(),
    };
    let saa = SystemArchitect::new(LlmClient::new(transport));

    let tdag = saa
        .plan(
            &edid_intent(),
            &["usd_amount >= 0.0".to_string()],
            "no prior state",
        )
        .await
        .expect("SAA should produce a valid t-DAG");

    assert_eq!(tdag.nodes.len(), 4);
    assert_eq!(tdag.edges.len(), 3);
    assert_eq!(tdag.nodes[0].kind, NodeKind::Ingest);
    assert_eq!(tdag.nodes[3].kind, NodeKind::Persist);
}

#[tokio::test]
async fn rejects_cyclic_tdag_from_model() {
    // A model that emits a cycle must be rejected by construction-time validation.
    let mut response = edid_tdag_response();
    response["content"][0]["input"]["edges"]
        .as_array_mut()
        .unwrap()
        .push(json!({"from": "persist_to_ledger", "to": "ingest_bills_csv", "kind": "data_flow"}));

    let saa = SystemArchitect::new(LlmClient::new(FixtureTransport { response }));
    let result = saa
        .plan(
            &edid_intent(),
            &["usd_amount >= 0.0".to_string()],
            "no prior state",
        )
        .await;
    assert!(result.is_err(), "a cyclic t-DAG must not validate");
}

#[tokio::test]
async fn surfaces_model_refusal() {
    let response = json!({"model": "claude-opus-4-8", "stop_reason": "refusal", "content": []});
    let saa = SystemArchitect::new(LlmClient::new(FixtureTransport { response }));
    let result = saa
        .plan(
            &edid_intent(),
            &["usd_amount >= 0.0".to_string()],
            "no prior state",
        )
        .await;
    assert!(
        result.is_err(),
        "a refusal must surface as an error, not a panic"
    );
}
