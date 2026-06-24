//! AETHER IPSE (Intent Parse & Synthesis Engine).
//!
//! Hosts the Claude HTTP client (`llm`), and — in later units — intent parsing,
//! the t-DAG model (U9), the synthesis agents (U10/U11), and the rustc→WASM
//! driver (U12).

pub mod agents;
pub mod intent_parse;
pub mod llm;
pub mod tdag;
