//! `aether watch` — monitor a source directory and run the pipeline per file (U16).
//!
//! Plans the intent's t-DAG once, then polls the source for new files; each new
//! file triggers a full orchestrated run. Steady-state runs are Blueprint-Cache
//! hits (synthesis bypassed). On Ctrl-C the daemon drains one final tick (so
//! files already dropped are processed) before exiting — graceful shutdown.
//!
//! A dependency-free poll loop is used rather than an OS file-watch crate:
//! single-host V1 scope, and it keeps the dependency surface minimal.

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

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

use crate::daemon::process_new;
use crate::orchestrator::{LlmSynthesizer, Orchestrator};

/// Watch `source` and process new files until interrupted. Returns the total
/// number of files processed across the session.
pub async fn watch(
    intent_path: &Path,
    source: &Path,
    ledger_path: &Path,
    cache_dir: &Path,
    scratch_dir: &Path,
    poll: Duration,
) -> Result<usize> {
    // Plan the t-DAG once (the pipeline shape is fixed for the intent).
    let intent_json = std::fs::read_to_string(intent_path)
        .map_err(|e| aether_sdk::AetherError::Io(format!("reading intent: {e}")))?;
    let intent = parse_intent(&intent_json)?;
    validate_invariant_refs(&intent, &default_known_invariants())?;

    let transport = ReqwestTransport::from_env()?;
    let saa = SystemArchitect::new(LlmClient::new(transport.clone()));
    let tdag = saa
        .plan(&intent, &intent.invariants, "watch daemon: per-file runs")
        .await?;

    let agents = IpseAgents::new(
        CompilerAgent::new(LlmClient::new(transport.clone())),
        CriticAgent::new(LlmClient::new(transport)),
    );
    let synthesizer = LlmSynthesizer::new(agents, RustcDriver::discover(scratch_dir)?);
    let cache = BlueprintCache::open(cache_dir)?;
    let sandbox = Sandbox::new()?;
    let mut ledger = CozoLedger::open(ledger_path)?;
    let mut orch = Orchestrator::new(
        synthesizer,
        cache,
        sandbox,
        default_invariants(),
        &mut ledger,
    )?;

    let mut seen = HashSet::new();
    let mut processed = 0usize;
    let base = Timestamp(0);

    eprintln!(
        "aether watch: monitoring {} (Ctrl-C to drain and exit)",
        source.display()
    );
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                // Graceful shutdown: drain one final tick, then stop.
                processed +=
                    process_new(&mut orch, &tdag, source, &mut seen, base, processed).await?;
                eprintln!("aether watch: drained; {processed} file(s) processed total");
                return Ok(processed);
            }
            _ = tokio::time::sleep(poll) => {
                let n = process_new(&mut orch, &tdag, source, &mut seen, base, processed).await?;
                if n > 0 {
                    processed += n;
                    eprintln!("aether watch: processed {n} new file(s) ({processed} total)");
                }
            }
        }
    }
}
