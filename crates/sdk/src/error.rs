//! The unified AETHER error envelope.
//!
//! Every fallible boundary in the engine returns [`AetherError`]; subsystem
//! crates add context through the string-carrying variants rather than
//! introducing parallel error types.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, AetherError>;

#[derive(Debug, Error)]
pub enum AetherError {
    #[error("intent is invalid: {0}")]
    IntentInvalid(String),

    #[error("unknown invariant reference: {0}")]
    UnknownInvariant(String),

    #[error("verification rejected at stage '{stage}': {reason}")]
    VerificationRejected { stage: String, reason: String },

    #[error("compilation failed: {0}")]
    CompileFailed(String),

    #[error("LLM error: {0}")]
    Llm(String),

    #[error("LLM refused the request: {0}")]
    LlmRefusal(String),

    #[error("sandbox trap: {0}")]
    SandboxTrap(String),

    #[error("capability denied: {0}")]
    CapabilityDenied(String),

    #[error("ledger error: {0}")]
    Ledger(String),

    #[error("integrity check failed: {0}")]
    IntegrityFailed(String),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("io error: {0}")]
    Io(String),
}
