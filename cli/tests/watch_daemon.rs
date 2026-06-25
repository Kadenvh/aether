//! U16 verification: the watch daemon processes a sequence of dropped files
//! into a consistent ledger, one verified transaction per file, on the
//! cache-hit steady-state path (no network).

use std::collections::HashSet;

use async_trait::async_trait;

use aether_ledger::CozoLedger;
use aether_runtime::{BlueprintCache, Sandbox};
use aether_sdk::types::{EdgeKind, EventKind, Ledger, NodeKind, TDag, TDagEdge, TDagNode};
use aether_sdk::{AetherError, Result, Timestamp};
use aether_verifier::default_invariants;

use aether_cli::daemon::process_new;
use aether_cli::orchestrator::{NodeSynthesizer, Orchestrator};

struct NoSynth;

#[async_trait]
impl NodeSynthesizer for NoSynth {
    async fn synthesize(&self, _node: &TDagNode) -> Result<Vec<u8>> {
        Err(AetherError::CompileFailed(
            "synth must not run on cache hit".into(),
        ))
    }
}

fn node(id: &str, kind: NodeKind) -> TDagNode {
    TDagNode {
        id: id.into(),
        kind,
        spec: serde_json::Value::Null,
    }
}

fn tdag() -> TDag {
    TDag {
        nodes: vec![
            node("ingest", NodeKind::Ingest),
            node("persist", NodeKind::Persist),
        ],
        edges: vec![TDagEdge {
            from: "ingest".into(),
            to: "persist".into(),
            kind: EdgeKind::DataFlow,
        }],
    }
}

fn seed(cache: &BlueprintCache, node: &TDagNode, value: i64) {
    let sig = BlueprintCache::tdag_signature(&TDag {
        nodes: vec![node.clone()],
        edges: vec![],
    })
    .unwrap();
    let wat = format!("(module (func (export \"run\") (result i32) i32.const {value}))");
    cache.store_wasm(&sig, wat.as_bytes()).unwrap();
}

fn tmp(tag: &str) -> std::path::PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!("aether-watch-{tag}-{n}"));
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[tokio::test]
async fn processes_dropped_files_into_a_consistent_ledger() {
    let root = tmp("daemon");
    let source = root.join("incoming");
    std::fs::create_dir_all(&source).unwrap();
    let cache = BlueprintCache::open(root.join("cache")).unwrap();
    let dag = tdag();
    seed(&cache, &dag.nodes[0], 1);
    seed(&cache, &dag.nodes[1], 250);

    let ledger_path = root.join("state.db");
    let total = {
        let mut ledger = CozoLedger::open(&ledger_path).unwrap();
        let sandbox = Sandbox::new().unwrap();
        let mut orch =
            Orchestrator::new(NoSynth, cache, sandbox, default_invariants(), &mut ledger).unwrap();
        let mut seen = HashSet::new();
        let base = Timestamp(0);

        // First drop: two files.
        std::fs::write(source.join("day1.csv"), b"a").unwrap();
        std::fs::write(source.join("day2.csv"), b"b").unwrap();
        let n1 = process_new(&mut orch, &dag, &source, &mut seen, base, 0)
            .await
            .unwrap();
        assert_eq!(n1, 2, "two new files processed");

        // Idle tick: nothing new.
        let n_idle = process_new(&mut orch, &dag, &source, &mut seen, base, n1)
            .await
            .unwrap();
        assert_eq!(n_idle, 0);

        // Second drop: one more file.
        std::fs::write(source.join("day3.csv"), b"c").unwrap();
        let n2 = process_new(&mut orch, &dag, &source, &mut seen, base, n1)
            .await
            .unwrap();
        assert_eq!(n2, 1, "one further file processed");
        n1 + n2
    };
    assert_eq!(total, 3);

    // The ledger holds one verified Assert per file, and the chain verifies.
    let ledger = CozoLedger::open(&ledger_path).unwrap();
    ledger
        .verify_chain()
        .expect("chain verifies after the watch session");
    let events = ledger
        .query_as_of(Timestamp(10_000), Timestamp(10_000))
        .unwrap();
    let asserts = events
        .iter()
        .filter(|e| e.kind == EventKind::Assert)
        .count();
    assert_eq!(asserts, 3, "one Assert event per processed file");

    let _ = std::fs::remove_dir_all(&root);
}
