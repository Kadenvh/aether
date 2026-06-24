//! AETHER IPSE (Intent Parse & Synthesis Engine).
//!
//! Hosts the Claude HTTP client (`llm`), and — in later units — intent parsing,
//! the t-DAG model (U9), the synthesis agents (U10/U11), and the rustc→WASM
//! driver (U12).

pub mod agents;
pub mod build_sandbox;
pub mod intent_parse;
pub mod llm;
pub mod rustc_driver;
pub mod synth;
pub mod tdag;

pub use rustc_driver::RustcDriver;
