//! `aether run` — wire the live subsystems and execute one intent (U13).
//!
//! Reads `intent.json`, plans the t-DAG with the live System Architect Agent,
//! and runs it through the orchestrator (synthesis → compile → verify → execute
//! → ledger). Network + toolchain are required here; the network-free runtime
//! path is what the e2e test exercises against a seeded cache.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use aether_compiler::agents::ca::CompilerAgent;
use aether_compiler::agents::cra::CriticAgent;
use aether_compiler::agents::saa::SystemArchitect;
use aether_compiler::agents::IpseAgents;
use aether_compiler::intent_parse::{
    default_known_invariants, parse_intent, validate_invariant_refs,
};
use aether_compiler::llm::{LlmClient, ReqwestTransport};
use aether_compiler::RustcDriver;
use aether_ledger::CozoLedger;
use aether_runtime::{BlueprintCache, Sandbox};
use aether_sdk::{Result, Timestamp};
use aether_verifier::default_invariants;

use crate::orchestrator::{LlmSynthesizer, Orchestrator, RunOutcome};

/// Execute an intent end to end, persisting verified state to `ledger_path`.
pub async fn execute(
    intent_path: &Path,
    input_path: Option<&Path>,
    ledger_path: &Path,
    cache_dir: &Path,
    scratch_dir: &Path,
) -> Result<RunOutcome> {
    // 1. Parse + boundary-validate the intent (U9), resolving invariant refs
    //    against the hardcoded registry (KTD3).
    let intent_json = std::fs::read_to_string(intent_path)
        .map_err(|e| aether_sdk::AetherError::Io(format!("reading intent: {e}")))?;
    let intent = parse_intent(&intent_json)?;
    let known = default_known_invariants();
    validate_invariant_refs(&intent, &known)?;

    // 2. Live transport shared across the IPSE agents (KTD6).
    let transport = ReqwestTransport::from_env()?;

    // 3. Plan the t-DAG with the System Architect Agent (U10).
    let saa = SystemArchitect::new(LlmClient::new(transport.clone()));
    let invariant_refs: Vec<String> = intent.invariants.clone();
    let ledger_summary = input_path
        .map(|p| format!("input source: {}", p.display()))
        .unwrap_or_else(|| "no input source".into());
    let tdag = saa.plan(&intent, &invariant_refs, &ledger_summary).await?;

    // 4. Build the live synthesizer (CA + CRA + locked rustc driver) and the
    //    runtime subsystems.
    let agents = IpseAgents::new(
        CompilerAgent::new(LlmClient::new(transport.clone())),
        CriticAgent::new(LlmClient::new(transport)),
    );
    let driver = RustcDriver::discover(scratch_dir)?;
    let synthesizer = LlmSynthesizer::new(agents, driver);

    let cache = BlueprintCache::open(cache_dir)?;
    let sandbox = Sandbox::new()?;
    let mut ledger = CozoLedger::open(ledger_path)?;

    // 5. Orchestrate; the FVL invariant set is hardcoded (U7), never from intent.
    let mut orch = Orchestrator::new(
        synthesizer,
        cache,
        sandbox,
        default_invariants(),
        &mut ledger,
    )?;
    orch.run(&tdag, now()).await
}

/// Wall-clock timestamp in seconds since the Unix epoch.
fn now() -> Timestamp {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Timestamp(secs as i64)
}
