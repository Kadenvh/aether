//! End-to-end `aether run` orchestration (U13) — the thesis in one path.
//!
//! Executes a validated, topologically-ordered t-DAG:
//! for each node, obtain its `.wasm` (Blueprint-Cache hit, else synthesize +
//! compile via the locked driver), AOT-compile + instantiate it in the
//! gas/epoch-bounded TAR sandbox, and collect its output. The aggregate result
//! becomes a proposed [`Mutation`], which must pass the FVL gate — meta-schema
//! (U8) **and** the Z3 invariant proof (U7) — before it is appended to the
//! immutable USL (U3). Compile failures and verification rejections are
//! themselves recorded as correction events in the same hash chain (R10).
//!
//! Synthesis is behind [`NodeSynthesizer`] so the runtime path
//! (cache → AOT → TAR → FVL → USL) is testable without a network: the live
//! [`LlmSynthesizer`] drives CA/CRA + rustc, while tests seed the cache.

use async_trait::async_trait;

use aether_compiler::agents::IpseAgents;
use aether_compiler::llm::Transport;
use aether_compiler::synth::{RepairConfig, RepairLoop};
use aether_compiler::RustcDriver;
use aether_runtime::{BlueprintCache, ExecLimits, Sandbox};
use aether_sdk::types::{EventKind, Ledger, LedgerEvent, Mutation, NodeKind, TDag, TDagNode};
use aether_sdk::{AetherError, Result, Timestamp};
use aether_verifier::{Invariant, MetaSchema, MutationDelta, Z3InvariantEngine};

/// The exported entry function every synthesized node provides.
const NODE_ENTRY: &str = "run";
/// The state variable the pipeline's net result is attributed to (cents).
const RESULT_VAR: &str = "usd_amount";

/// Produces the `.wasm` bytes implementing a single t-DAG node.
#[async_trait]
pub trait NodeSynthesizer: Send + Sync {
    async fn synthesize(&self, node: &TDagNode) -> Result<Vec<u8>>;
}

/// The live synthesizer: Compiler/Critic agents drive the bounded repair loop
/// against the locked rustc→wasm driver (U10/U11/U12).
pub struct LlmSynthesizer<T: Transport> {
    agents: IpseAgents<T>,
    driver: RustcDriver,
    repair: RepairLoop,
}

impl<T: Transport> LlmSynthesizer<T> {
    pub fn new(agents: IpseAgents<T>, driver: RustcDriver) -> Self {
        LlmSynthesizer {
            agents,
            driver,
            repair: RepairLoop::new(RepairConfig::default()),
        }
    }
}

#[async_trait]
impl<T: Transport> NodeSynthesizer for LlmSynthesizer<T> {
    async fn synthesize(&self, node: &TDagNode) -> Result<Vec<u8>> {
        let spec = node_spec(node);
        let outcome = self.repair.run(&spec, &self.agents, &self.driver).await?;
        Ok(outcome.wasm)
    }
}

/// A compact natural-language spec handed to the Compiler Agent for a node.
fn node_spec(node: &TDagNode) -> String {
    format!(
        "Node id: {id}\nKind: {kind:?}\nSpec: {spec}\n\
         Implement an exported `fn {entry}() -> i32` returning this node's integer-cents result.",
        id = node.id,
        kind = node.kind,
        spec = node.spec,
        entry = NODE_ENTRY,
    )
}

/// The result of a successful end-to-end run.
#[derive(Debug)]
pub struct RunOutcome {
    /// Net integer-cents result the pipeline produced.
    pub result_cents: i64,
    /// The ledger event id appended for the verified mutation.
    pub event_id: String,
    /// Number of t-DAG nodes executed.
    pub nodes_executed: usize,
}

/// Wires the subsystems and runs an intent's t-DAG end to end.
pub struct Orchestrator<'a, S: NodeSynthesizer, L: Ledger> {
    synthesizer: S,
    cache: BlueprintCache,
    sandbox: Sandbox,
    meta_schema: MetaSchema,
    z3: Z3InvariantEngine,
    invariants: Vec<Invariant>,
    ledger: &'a mut L,
}

impl<'a, S: NodeSynthesizer, L: Ledger> Orchestrator<'a, S, L> {
    pub fn new(
        synthesizer: S,
        cache: BlueprintCache,
        sandbox: Sandbox,
        invariants: Vec<Invariant>,
        ledger: &'a mut L,
    ) -> Result<Self> {
        Ok(Orchestrator {
            synthesizer,
            cache,
            sandbox,
            meta_schema: MetaSchema::new()?,
            z3: Z3InvariantEngine::new(),
            invariants,
            ledger,
        })
    }

    /// Execute the t-DAG and persist the verified mutation. `now` is the wall
    /// timestamp (injected so runs are deterministic / testable).
    pub async fn run(&mut self, tdag: &TDag, now: Timestamp) -> Result<RunOutcome> {
        let order = topo_order(tdag)?;
        let by_id = |id: &str| tdag.nodes.iter().find(|n| n.id == id);

        // Execute every node in dependency order; the terminal node's output is
        // the pipeline's net result.
        let limits = ExecLimits::default();
        let mut last_output: i64 = 0;
        for node_id in &order {
            let node = by_id(node_id)
                .ok_or_else(|| AetherError::IntentInvalid(format!("missing node '{node_id}'")))?;
            last_output = self.execute_node(node, &limits, now).await?;
        }

        // Propose the verified mutation from the net result.
        let mutation = Mutation {
            subject: format!(
                "pipeline:{}",
                order.last().map(String::as_str).unwrap_or("empty")
            ),
            predicate: format!("{RESULT_VAR}_cents"),
            object: last_output.to_string(),
        };

        // FVL hot-path gate: meta-schema (U8) then Z3 invariant proof (U7).
        if let Err(e) = self.gate(&mutation, last_output) {
            self.append(
                now,
                EventKind::VerificationRejection,
                &mutation_json(&mutation),
            )?;
            return Err(e);
        }

        let event_id = self.append(now, EventKind::Assert, &mutation_json(&mutation))?;
        Ok(RunOutcome {
            result_cents: last_output,
            event_id,
            nodes_executed: order.len(),
        })
    }

    /// Obtain a node's wasm (cache or synthesis), AOT-compile, and run it in TAR.
    async fn execute_node(
        &mut self,
        node: &TDagNode,
        limits: &ExecLimits,
        now: Timestamp,
    ) -> Result<i64> {
        let signature = BlueprintCache::tdag_signature(&TDag {
            nodes: vec![node.clone()],
            edges: vec![],
        })?;

        let wasm = match self.cache.wasm_for_signature(&signature) {
            Some(bytes) => bytes,
            None => match self.synthesizer.synthesize(node).await {
                Ok(bytes) => {
                    self.cache.store_wasm(&signature, &bytes)?;
                    bytes
                }
                Err(e) => {
                    // R10: record the synthesis/compile failure in the chain.
                    self.append(
                        now,
                        EventKind::CompileFailure,
                        &serde_json::json!({ "node": node.id, "error": e.to_string() }),
                    )?;
                    return Err(e);
                }
            },
        };

        let module = self.cache.module_for_wasm(self.sandbox.engine(), &wasm)?;
        let out = self
            .sandbox
            .run_module_i32(&module, NODE_ENTRY, limits)
            .await?;
        Ok(i64::from(out))
    }

    /// The FVL gate: structural validation then a fail-closed invariant proof.
    fn gate(&self, mutation: &Mutation, result_cents: i64) -> Result<()> {
        self.meta_schema
            .validate_mutation(&mutation_json(mutation))?;
        // Model the run as setting the result var from a known-valid zero state.
        let deltas = [MutationDelta::new(RESULT_VAR, result_cents)];
        self.z3.prove_preserved(&self.invariants, &deltas)
    }

    /// Append an event to the USL, returning its id.
    fn append(
        &mut self,
        now: Timestamp,
        kind: EventKind,
        payload: &serde_json::Value,
    ) -> Result<String> {
        let id = format!("evt-{}-{:?}", now.0, kind);
        let event = LedgerEvent {
            id: id.clone(),
            kind,
            payload: payload.clone(),
            tx_time: now,
            valid_from: now,
            valid_to: None,
            prev_hash: String::new(),
            curr_hash: String::new(),
        };
        self.ledger.append_event(event)?;
        Ok(id)
    }
}

fn mutation_json(m: &Mutation) -> serde_json::Value {
    serde_json::json!({ "subject": m.subject, "predicate": m.predicate, "object": m.object })
}

/// Deterministic topological order of node ids (validates acyclicity via U9).
fn topo_order(tdag: &TDag) -> Result<Vec<String>> {
    use aether_compiler::tdag::ExecutionGraph;
    let graph = ExecutionGraph::build(tdag)?;
    Ok(graph.topo_order().to_vec())
}

/// Whether a node kind writes to the ledger (terminal persist).
pub fn is_persist(kind: NodeKind) -> bool {
    matches!(kind, NodeKind::Persist)
}
