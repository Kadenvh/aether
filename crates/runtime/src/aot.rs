//! Ahead-of-time compilation (U6, KTD2 layer 2).
//!
//! `precompile` turns a validated `.wasm` into an engine-native artifact via
//! Cranelift once; `load_precompiled` deserializes it later, skipping Cranelift
//! entirely for µs instantiation. Deserialization is `unsafe` — it trusts the
//! artifact's layout — so AETHER only ever loads artifacts it produced itself,
//! content-addressed by the source `.wasm` hash (see [`crate::blueprint_cache`]).
//! Wasmtime additionally stamps a compatibility header and errors on mismatch,
//! so a stale artifact is rejected rather than mis-executed.

use wasmtime::{Engine, Module};

use aether_sdk::{AetherError, Result};

/// Compile `wasm` to a serialized, engine-native AOT artifact.
pub fn precompile(engine: &Engine, wasm: &[u8]) -> Result<Vec<u8>> {
    engine
        .precompile_module(wasm)
        .map_err(|e| AetherError::SandboxTrap(format!("AOT precompile failed: {e}")))
}

/// Load an artifact previously produced by [`precompile`] on a compatible engine.
///
/// # Safety contract
/// The caller must pass only artifacts AETHER itself wrote to its
/// content-addressed cache — never bytes from an untrusted source. The wrapped
/// `Module::deserialize` is `unsafe` for exactly this reason.
pub fn load_precompiled(engine: &Engine, artifact: &[u8]) -> Result<Module> {
    // SAFETY: `artifact` originates from AETHER's own cache, written by
    // `precompile` using this engine's configuration. Wasmtime validates a
    // compatibility header on deserialize and returns `Err` on any mismatch, so
    // an incompatible or corrupt artifact fails closed instead of executing.
    unsafe { Module::deserialize(engine, artifact) }
        .map_err(|e| AetherError::IntegrityFailed(format!("AOT artifact rejected: {e}")))
}
