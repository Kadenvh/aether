//! AETHER CLI library surface (U13).
//!
//! Exposes the orchestration pieces so integration tests can drive the
//! end-to-end runtime path directly; the `aether` binary (`main.rs`) is a thin
//! shell over these modules.

pub mod hitlc;
pub mod orchestrator;
pub mod run;
