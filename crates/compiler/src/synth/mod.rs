//! Node synthesis: the compile-critic repair loop and its diagnostics (U11).
//!
//! Named `synth` rather than `loop` because `loop` is a reserved keyword in
//! Rust and cannot be a module identifier.

pub mod diagnostics;
pub mod repair;

pub use diagnostics::{diagnostic_signature, error_count, parse_rustc_diagnostics, Diagnostic};
pub use repair::{
    CodeAgent, CompileOutcome, CorrectionRecord, NodeCompiler, Repair, RepairConfig, RepairLoop,
    RepairOutcome,
};
