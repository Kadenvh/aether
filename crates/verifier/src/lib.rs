//! AETHER FVL (Formal Verification Layer) — hot-path gate.
//!
//! V1 ships the meta-schema validation gate (U8). The Z3 invariant engine (U7)
//! and the offline Kani/Apalache tier (U14) attach here as they land.

pub mod meta_schema;

pub use meta_schema::MetaSchema;
