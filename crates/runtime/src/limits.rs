//! Per-transaction execution limits (R3, KTD: bounded ephemeral execution).
//!
//! Every sandbox run is bounded on three independent axes:
//! - **fuel** — deterministic instruction budget (`Trap::OutOfFuel` on exhaustion),
//! - **epoch + wall-clock** — a non-deterministic real-time kill-switch for code
//!   that consumes little fuel but blocks (host calls, tight async waits),
//! - **memory / instances** — a `StoreLimits` cap on linear-memory growth.
//!
//! Fuel is the primary, reproducible guard; the wall-clock deadline is
//! belt-and-suspenders (the runtime arms both — see [`crate::exec`]).

use std::time::Duration;

use wasmtime::{StoreLimits, StoreLimitsBuilder};

/// Default instruction budget. Generous enough for an ingestion/transform node,
/// small enough to bound a runaway loop quickly.
pub const DEFAULT_FUEL: u64 = 5_000_000;

/// Default wall-clock ceiling for a single node execution.
pub const DEFAULT_WALL_TIMEOUT: Duration = Duration::from_secs(5);

/// Default linear-memory cap per guest (32 MiB).
pub const DEFAULT_MAX_MEMORY_BYTES: usize = 32 * 1024 * 1024;

/// Default cap on instances created within a single store.
pub const DEFAULT_MAX_INSTANCES: usize = 16;

/// The resource envelope for one ephemeral execution.
#[derive(Debug, Clone)]
pub struct ExecLimits {
    /// Deterministic instruction budget consumed as the guest runs.
    pub fuel: u64,
    /// Real-time ceiling enforced via epoch interruption + async timeout.
    pub wall_timeout: Duration,
    /// Maximum linear memory the guest may grow to, in bytes.
    pub max_memory_bytes: usize,
    /// Maximum number of instances within the store.
    pub max_instances: usize,
}

impl Default for ExecLimits {
    fn default() -> Self {
        ExecLimits {
            fuel: DEFAULT_FUEL,
            wall_timeout: DEFAULT_WALL_TIMEOUT,
            max_memory_bytes: DEFAULT_MAX_MEMORY_BYTES,
            max_instances: DEFAULT_MAX_INSTANCES,
        }
    }
}

impl ExecLimits {
    /// Build the Wasmtime `StoreLimits` for the memory/instance axis.
    pub fn store_limits(&self) -> StoreLimits {
        StoreLimitsBuilder::new()
            .memory_size(self.max_memory_bytes)
            .instances(self.max_instances)
            .build()
    }
}
