//! IPSE synthesis agents (R5).
//!
//! - [`saa`] — System Architect Agent: intent -> t-DAG (U10).
//! - [`ca`] — Compiler Agent: node spec -> Rust + WIT (U11).
//! - [`cra`] — Critic-Reviewer Agent: diagnostics -> minimal patch + lesson (U11).
//!
//! [`IpseAgents`] composes the CA and CRA into the [`CodeAgent`] the repair loop
//! drives.

pub mod ca;
pub mod cra;
pub mod saa;

use async_trait::async_trait;

use aether_sdk::Result;

use crate::llm::Transport;
use crate::synth::repair::{CodeAgent, Repair};
use crate::synth::Diagnostic;

use ca::CompilerAgent;
use cra::CriticAgent;

/// The IPSE code agents wired together as the loop's [`CodeAgent`]: the Compiler
/// Agent does the initial synthesis, the Critic-Reviewer Agent does repairs.
pub struct IpseAgents<T: Transport> {
    pub compiler: CompilerAgent<T>,
    pub critic: CriticAgent<T>,
}

impl<T: Transport> IpseAgents<T> {
    pub fn new(compiler: CompilerAgent<T>, critic: CriticAgent<T>) -> Self {
        IpseAgents { compiler, critic }
    }
}

#[async_trait]
impl<T: Transport> CodeAgent for IpseAgents<T> {
    async fn generate(&self, node_spec: &str, lessons: &[String]) -> Result<String> {
        // The loop compiles Rust; the generated WIT travels with U12/U15 plumbing.
        Ok(self
            .compiler
            .generate(node_spec, lessons)
            .await?
            .rust_source)
    }

    async fn repair(&self, prev_source: &str, diagnostics: &[Diagnostic]) -> Result<Repair> {
        self.critic.repair(prev_source, diagnostics).await
    }
}
