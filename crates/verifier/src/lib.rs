//! AETHER FVL (Formal Verification Layer) — hot-path gate.
//!
//! V1 ships the meta-schema validation gate (U8) and the Z3 invariant engine
//! (U7). The offline Kani/Apalache tier (U14) runs out-of-process in CI.

pub mod invariants;
pub mod meta_schema;
pub mod z3_engine;

pub use invariants::{default_invariants, Invariant, MutationDelta};
pub use meta_schema::MetaSchema;
pub use z3_engine::Z3InvariantEngine;
