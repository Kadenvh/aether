//! U13 verification: the full runtime thesis, end to end, without a network.
//!
//! We seed the Blueprint Cache with real core-wasm nodes (so synthesis is
//! skipped — the LLM is never called) and drive the orchestrator through
//! cache → AOT → TAR (sandboxed execution) → FVL (meta-schema + Z3) → USL
//! (append-only hash-chained ledger). This proves a verified ephemeral pipeline
//! produces an immutable, chain-verified ledger entry — and that an
//! invariant-violating result is rejected fail-closed and recorded (R10).

use async_trait::async_trait;

use aether_ledger::CozoLedger;
use aether_runtime::{BlueprintCache, Sandbox};
use aether_sdk::types::{EdgeKind, EventKind, Ledger, NodeKind, TDag, TDagEdge, TDagNode};
use aether_sdk::{AetherError, Result, Timestamp};
use aether_verifier::default_invariants;

use aether_cli::orchestrator::{NodeSynthesizer, Orchestrator};

/// A synthesizer that must never run — proves the cache-hit path.
struct NoSynth;

#[async_trait]
impl NodeSynthesizer for NoSynth {
    async fn synthesize(&self, node: &TDagNode) -> Result<Vec<u8>> {
        Err(AetherError::CompileFailed(format!(
            "synth should not run for cached node '{}'",
            node.id
        )))
    }
}

fn node(id: &str, kind: NodeKind) -> TDagNode {
    TDagNode {
        id: id.into(),
        kind,
        spec: serde_json::Value::Null,
    }
}

fn edge(from: &str, to: &str) -> TDagEdge {
    TDagEdge {
        from: from.into(),
        to: to.into(),
        kind: EdgeKind::DataFlow,
    }
}

/// A core-wasm module exporting `run() -> i32` returning `value`.
fn wat_returning(value: i64) -> String {
    format!("(module (func (export \"run\") (result i32) i32.const {value}))")
}

/// Seed the cache so a node resolves to a wasm returning its scripted value.
fn seed(cache: &BlueprintCache, node: &TDagNode, value: i64) {
    let sig = BlueprintCache::tdag_signature(&TDag {
        nodes: vec![node.clone()],
        edges: vec![],
    })
    .unwrap();
    cache
        .store_wasm(&sig, wat_returning(value).as_bytes())
        .unwrap();
}

fn tmp(tag: &str) -> std::path::PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!("aether-e2e-{tag}-{n}"));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn edid_tdag() -> TDag {
    TDag {
        nodes: vec![
            node("ingest_bills", NodeKind::Ingest),
            node("convert_eur_usd", NodeKind::Transform),
            node("persist_ledger", NodeKind::Persist),
        ],
        edges: vec![
            edge("ingest_bills", "convert_eur_usd"),
            edge("convert_eur_usd", "persist_ledger"),
        ],
    }
}

#[tokio::test]
async fn verified_pipeline_appends_to_immutable_ledger() {
    let dir = tmp("ok");
    let cache = BlueprintCache::open(dir.join("cache")).unwrap();
    let tdag = edid_tdag();

    // Terminal node yields +12345 cents — satisfies usd_amount >= 0.
    seed(&cache, &tdag.nodes[0], 1);
    seed(&cache, &tdag.nodes[1], 999);
    seed(&cache, &tdag.nodes[2], 12_345);

    let ledger_path = dir.join("state.db");
    let outcome = {
        let mut ledger = CozoLedger::open(&ledger_path).unwrap();
        let sandbox = Sandbox::new().unwrap();
        let mut orch =
            Orchestrator::new(NoSynth, cache, sandbox, default_invariants(), &mut ledger).unwrap();
        orch.run(&tdag, Timestamp(1000))
            .await
            .expect("verified run should succeed")
    };

    assert_eq!(outcome.nodes_executed, 3);
    assert_eq!(outcome.result_cents, 12_345);

    // Reopen: the verified mutation persisted as an Assert, chain verifies.
    let ledger = CozoLedger::open(&ledger_path).unwrap();
    ledger.verify_chain().expect("ledger chain verifies");
    let events = ledger
        .query_as_of(Timestamp(2000), Timestamp(2000))
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, EventKind::Assert);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn invariant_violation_is_rejected_and_recorded() {
    let dir = tmp("reject");
    let cache = BlueprintCache::open(dir.join("cache")).unwrap();
    let tdag = edid_tdag();

    // Terminal node yields a NEGATIVE result — violates usd_amount >= 0.
    seed(&cache, &tdag.nodes[0], 1);
    seed(&cache, &tdag.nodes[1], 1);
    seed(&cache, &tdag.nodes[2], -500);

    let ledger_path = dir.join("state.db");
    let result = {
        let mut ledger = CozoLedger::open(&ledger_path).unwrap();
        let sandbox = Sandbox::new().unwrap();
        let mut orch =
            Orchestrator::new(NoSynth, cache, sandbox, default_invariants(), &mut ledger).unwrap();
        orch.run(&tdag, Timestamp(1000)).await
    };
    assert!(
        result.is_err(),
        "a negative result must be rejected by the FVL gate"
    );

    // The rejection is recorded in the chain (R10), and the chain still verifies.
    let ledger = CozoLedger::open(&ledger_path).unwrap();
    ledger
        .verify_chain()
        .expect("chain verifies even with a rejection event");
    let events = ledger
        .query_as_of(Timestamp(2000), Timestamp(2000))
        .unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.kind == EventKind::VerificationRejection),
        "expected a VerificationRejection event"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
