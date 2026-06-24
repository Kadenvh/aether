//! The Reflexion-shaped compile-critic repair loop (U11, KTD7, R10).
//!
//! The Compiler Agent (CA) synthesizes a node's Rust; the rustc→WASM driver
//! (U12, behind [`NodeCompiler`]) compiles it; on failure the Critic-Reviewer
//! agent (CRA, behind [`CodeAgent::repair`]) turns the *structured* diagnostics
//! into a minimal patch plus a natural-language lesson, which is prepended to
//! the next attempt and recorded as a correction (R10).
//!
//! Both the compiler and the agents are traits so the convergence logic is
//! tested against scripted stubs — no real rustc, no live model (the unit's
//! verification). Convergence guards (KTD7):
//! - **hard iteration cap** (3–5),
//! - **stagnation detection** — abort + escalate to HITLC if the diagnostic set
//!   is identical to the previous attempt's, or the error count stops shrinking.

use async_trait::async_trait;

use aether_sdk::{AetherError, Result};

use super::diagnostics::{diagnostic_signature, error_count, Diagnostic};

/// The result of one compile attempt.
pub enum CompileOutcome {
    /// Compilation succeeded; carries the produced `.wasm` bytes.
    Success(Vec<u8>),
    /// Compilation failed with these structured diagnostics.
    Errors(Vec<Diagnostic>),
}

/// Compiles synthesized Rust to WASM. Implemented by U12's locked rustc driver;
/// stubbed in tests.
#[async_trait]
pub trait NodeCompiler: Send + Sync {
    async fn compile(&self, rust_source: &str) -> Result<CompileOutcome>;
}

/// A minimal, localized fix produced by the Critic-Reviewer agent.
pub struct Repair {
    pub rust_source: String,
    pub lesson: String,
}

/// The synthesis agents behind the loop. `generate` is the Compiler Agent's
/// initial pass; `repair` is the Critic-Reviewer's diagnostic-driven patch.
#[async_trait]
pub trait CodeAgent: Send + Sync {
    async fn generate(&self, node_spec: &str, lessons: &[String]) -> Result<String>;
    async fn repair(&self, prev_source: &str, diagnostics: &[Diagnostic]) -> Result<Repair>;
}

/// One failed-attempt record for the correction log (R10). U13 seals these into
/// `CompileFailure` ledger events so they participate in the hash chain.
#[derive(Debug, Clone)]
pub struct CorrectionRecord {
    pub attempt: u32,
    pub diagnostics: Vec<String>,
    pub lesson: String,
}

/// Convergence configuration (KTD7).
#[derive(Debug, Clone, Copy)]
pub struct RepairConfig {
    /// Hard cap on compile attempts (3–5).
    pub max_iterations: u32,
}

impl Default for RepairConfig {
    fn default() -> Self {
        RepairConfig { max_iterations: 4 }
    }
}

/// A successful synthesis: the compiled module plus the trail that produced it.
#[derive(Debug, Clone)]
pub struct RepairOutcome {
    pub wasm: Vec<u8>,
    pub attempts: u32,
    pub lessons: Vec<String>,
    pub corrections: Vec<CorrectionRecord>,
}

/// Drives synthesis → compile → critique → repair to a bounded fixpoint.
pub struct RepairLoop {
    config: RepairConfig,
}

impl RepairLoop {
    pub fn new(config: RepairConfig) -> Self {
        RepairLoop { config }
    }

    /// Run the loop for one node. Returns the compiled module on success, or a
    /// [`AetherError::CompileFailed`] (carrying the escalation reason) when the
    /// loop hits the iteration cap or detects stagnation.
    pub async fn run(
        &self,
        node_spec: &str,
        agent: &dyn CodeAgent,
        compiler: &dyn NodeCompiler,
    ) -> Result<RepairOutcome> {
        let mut lessons: Vec<String> = Vec::new();
        let mut corrections: Vec<CorrectionRecord> = Vec::new();
        let mut source = agent.generate(node_spec, &lessons).await?;
        let mut prev_sig: Option<Vec<String>> = None;
        let mut prev_errors: Option<usize> = None;

        for attempt in 1..=self.config.max_iterations {
            match compiler.compile(&source).await? {
                CompileOutcome::Success(wasm) => {
                    return Ok(RepairOutcome {
                        wasm,
                        attempts: attempt,
                        lessons,
                        corrections,
                    });
                }
                CompileOutcome::Errors(diags) => {
                    let sig = diagnostic_signature(&diags);
                    let errs = error_count(&diags);

                    // Stagnation (KTD7): identical diagnostics, or the error count
                    // failed to shrink versus the previous attempt.
                    let stagnated = prev_sig.as_ref() == Some(&sig)
                        || prev_errors.is_some_and(|prev| errs >= prev);
                    if attempt > 1 && stagnated {
                        return Err(AetherError::CompileFailed(format!(
                            "repair stagnated after {attempt} attempt(s): {errs} error(s) not \
                             shrinking; escalate to HITLC"
                        )));
                    }

                    if attempt == self.config.max_iterations {
                        return Err(AetherError::CompileFailed(format!(
                            "iteration cap ({}) reached with {errs} error(s) remaining; escalate \
                             to HITLC",
                            self.config.max_iterations
                        )));
                    }

                    let repair = agent.repair(&source, &diags).await?;
                    corrections.push(CorrectionRecord {
                        attempt,
                        diagnostics: sig.clone(),
                        lesson: repair.lesson.clone(),
                    });
                    lessons.push(repair.lesson);
                    source = repair.rust_source;
                    prev_sig = Some(sig);
                    prev_errors = Some(errs);
                }
            }
        }

        // The loop body returns on success, cap, or stagnation for every path.
        unreachable!("repair loop must terminate inside the attempt range")
    }
}
