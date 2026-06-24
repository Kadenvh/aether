//! The ephemeral execution sandbox (R3, KTD9 runtime side).
//!
//! A [`Sandbox`] owns a process-shared Wasmtime [`Engine`] configured for fuel
//! metering, epoch interruption, and async execution. Each call to
//! [`Sandbox::run_i32`] builds a fresh `Store<HostState>` — one ephemeral
//! execution unit — instantiates the guest, and runs an exported function under
//! three independent bounds:
//!
//! - **fuel** — `store.set_fuel`; exhaustion traps deterministically,
//! - **epoch** — a background ticker increments the engine epoch; the store's
//!   epoch deadline turns a wall-clock overrun into a trap even if the guest
//!   burns little fuel,
//! - **wall-clock** — the whole async call is wrapped in `tokio::time::timeout`
//!   as a final backstop (the async fiber unwinds when the future is dropped).
//!
//! The epoch ticker runs on a dedicated OS thread for the `Sandbox`'s lifetime
//! and is stopped on `Drop`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use wasmtime::{Config, Engine, Linker, Module, Store, Trap};

use aether_sdk::{AetherError, Result};

use crate::host::HostState;
use crate::limits::ExecLimits;

/// How often the background thread advances the engine epoch. The wall-clock
/// deadline is rounded up to a whole number of these ticks.
const EPOCH_TICK: Duration = Duration::from_millis(10);

/// A reusable, isolated WASM execution host.
///
/// One `Sandbox` (and its warm `Engine`) is shared across many transactions;
/// each transaction still executes in its own `Store`, sharing nothing.
pub struct Sandbox {
    engine: Engine,
    ticker_stop: Arc<AtomicBool>,
    ticker: Option<JoinHandle<()>>,
}

impl Sandbox {
    /// Build a sandbox with a fuel-metered, epoch-interruptible, async engine
    /// and start the epoch ticker.
    pub fn new() -> Result<Self> {
        // async support is always available in wasmtime 46 (the old
        // `async_support` toggle is a deprecated no-op); we drive guests via
        // `instantiate_async`/`call_async` so the wall-clock timeout can cancel.
        let mut config = Config::new();
        config.consume_fuel(true);
        config.epoch_interruption(true);

        let engine = Engine::new(&config)
            .map_err(|e| AetherError::SandboxTrap(format!("engine init failed: {e}")))?;

        let ticker_stop = Arc::new(AtomicBool::new(false));
        let ticker = {
            let engine = engine.clone();
            let stop = Arc::clone(&ticker_stop);
            thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    thread::sleep(EPOCH_TICK);
                    engine.increment_epoch();
                }
            })
        };

        Ok(Sandbox {
            engine,
            ticker_stop,
            ticker: Some(ticker),
        })
    }

    /// Instantiate `wasm` (core module, `.wat` text or binary) and call the
    /// nullary exported function `func`, returning its `i32` result.
    ///
    /// Fails with [`AetherError::SandboxTrap`] on fuel exhaustion, epoch/timeout
    /// expiry, a guest trap, or a missing/mistyped export.
    pub async fn run_i32(&self, wasm: &[u8], func: &str, limits: &ExecLimits) -> Result<i32> {
        let module = Module::new(&self.engine, wasm)
            .map_err(|e| AetherError::SandboxTrap(format!("module load failed: {e}")))?;

        let mut store = Store::new(&self.engine, HostState::new(limits.store_limits()));
        store.limiter(|state| &mut state.limits);

        store
            .set_fuel(limits.fuel)
            .map_err(|e| AetherError::SandboxTrap(format!("set_fuel failed: {e}")))?;

        let deadline_ticks = wall_timeout_ticks(limits.wall_timeout);
        store.set_epoch_deadline(deadline_ticks);

        // Zero-import core module: an empty linker suffices. WASI imports are
        // injected by U5 on top of this same engine/store contract.
        let linker = Linker::new(&self.engine);

        let call = async {
            let instance = linker
                .instantiate_async(&mut store, &module)
                .await
                .map_err(map_trap)?;
            let typed = instance
                .get_typed_func::<(), i32>(&mut store, func)
                .map_err(|e| AetherError::SandboxTrap(format!("missing export '{func}': {e}")))?;
            typed.call_async(&mut store, ()).await.map_err(map_trap)
        };

        match tokio::time::timeout(limits.wall_timeout, call).await {
            Ok(result) => result,
            Err(_elapsed) => Err(AetherError::SandboxTrap(format!(
                "wall-clock timeout after {:?}",
                limits.wall_timeout
            ))),
        }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        self.ticker_stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.ticker.take() {
            let _ = handle.join();
        }
    }
}

/// Round a wall-clock budget up to a whole number of epoch ticks (minimum 1).
fn wall_timeout_ticks(timeout: Duration) -> u64 {
    let ticks = timeout.as_millis() / EPOCH_TICK.as_millis().max(1);
    (ticks as u64).max(1)
}

/// Classify a Wasmtime execution error into an [`AetherError::SandboxTrap`],
/// naming the well-known resource-exhaustion traps explicitly.
fn map_trap(err: wasmtime::Error) -> AetherError {
    if let Some(trap) = err.downcast_ref::<Trap>() {
        let detail = match trap {
            Trap::OutOfFuel => "out of fuel (instruction budget exhausted)".to_string(),
            Trap::Interrupt => "interrupted (epoch deadline / wall-clock)".to_string(),
            other => format!("guest trap: {other}"),
        };
        return AetherError::SandboxTrap(detail);
    }
    AetherError::SandboxTrap(format!("execution error: {err}"))
}
