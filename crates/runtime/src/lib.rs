//! AETHER TAR (Transient Assembly Runtime).
//!
//! Instantiates and runs synthesized `.wasm` modules in isolated, fuel-metered,
//! time-bounded sandboxes (R3). Later units layer on this engine contract:
//! WASI capability injection (U5), the AOT blueprint cache (U6).

pub mod aot;
pub mod blueprint_cache;
pub mod exec;
pub mod host;
pub mod limits;
pub mod net_guard;
pub mod wasi_caps;

pub use blueprint_cache::BlueprintCache;
pub use exec::Sandbox;
pub use limits::ExecLimits;
pub use net_guard::NetGuard;
pub use wasi_caps::{build_wasi_ctx, WasiHost};
