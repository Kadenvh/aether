//! Per-store host state (R3, R8).
//!
//! Each ephemeral execution gets its own `Store<HostState>`. In this unit the
//! host state carries only the `StoreLimits` enforcing the memory/instance cap;
//! the WASI capability context (preopened dirs, network allowlist, clock policy)
//! is layered on here by U5 (`wasi_caps`). Keeping it as a single owned struct
//! means the store has exactly one mutable authority surface.

use wasmtime::StoreLimits;

/// State owned by a single sandbox `Store`. One per ephemeral transaction.
pub struct HostState {
    /// Memory/instance growth limits enforced by Wasmtime's limiter hook.
    pub limits: StoreLimits,
}

impl HostState {
    /// Create host state seeded with the given store limits.
    pub fn new(limits: StoreLimits) -> Self {
        HostState { limits }
    }
}
