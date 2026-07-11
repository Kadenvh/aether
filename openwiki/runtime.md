# Runtime (TAR — Transient Assembly Runtime)

The `aether-runtime` crate executes compiled WASM modules in an isolated
Wasmtime sandbox with zero ambient authority, fuel-metered execution, and a
content-addressed blueprint cache with AOT precompilation.

Source: `crates/runtime/src/`.

## Module map

| Module | File | Responsibility |
|--------|------|----------------|
| `exec` | `exec.rs` | `Sandbox`: owns the `Engine`, runs modules with fuel + epoch + timeout |
| `limits` | `limits.rs` | `ExecLimits`: fuel, wall timeout, memory, instances |
| `host` | `host.rs` | `HostState`: per-transaction store state wrapping `StoreLimits` |
| `wasi_caps` | `wasi_caps.rs` | `WasiHost` + `build_wasi_ctx`: deny-by-default WASI capability injection |
| `net_guard` | `net_guard.rs` | `NetGuard`: socket-layer outbound allowlist |
| `blueprint_cache` | `blueprint_cache.rs` | `BlueprintCache`: three-layer content-addressed cache |
| `aot` | `aot.rs` | AOT precompile + deserialize (unsafe, version-keyed) |

Public re-exports: `BlueprintCache`, `Sandbox`, `ExecLimits`, `NetGuard`,
`build_wasi_ctx`, `WasiHost`.

## Sandbox (`exec.rs`)

```rust
pub struct Sandbox {
    engine: Engine,
    ticker_stop: Arc<AtomicBool>,
    ticker: Option<JoinHandle<()>>,
}
```

The `Sandbox` owns a process-shared `Engine` configured for:
- `consume_fuel(true)` — deterministic instruction budget
- `epoch_interruption(true)` — cooperative interruption via epoch deadline

A background thread increments the engine epoch every 10ms (`EPOCH_TICK`),
stopped on `Drop`.

### Key methods

```rust
pub fn engine(&self) -> &Engine                                              // expose warm engine for AOT cache
pub async fn run_i32(&self, wasm: &[u8], func: &str, limits: &ExecLimits) -> Result<i32>
pub async fn run_module_i32(&self, module: &Module, func: &str, limits: &ExecLimits) -> Result<i32>
```

`run_module_i32` is the core execution path:
1. Fresh `Store::new(&self.engine, HostState::new(limits.store_limits()))`
2. Register limiter: `store.limiter(|state| &mut state.limits)`
3. Set fuel: `store.set_fuel(limits.fuel)`
4. Set epoch deadline: `store.set_epoch_deadline(wall_timeout_ticks(wall_timeout))`
5. Instantiate via `linker.instantiate_async(&mut store, module)`
6. Get typed export `get_typed_func::<(), i32>(&mut store, func)`
7. Call `typed.call_async(&mut store, ())`
8. Wrap entire async block in `tokio::time::timeout(wall_timeout, call)`

### Error mapping

| Trap | `AetherError` |
|------|---------------|
| `Trap::OutOfFuel` | `SandboxTrap("out of fuel (instruction budget exhausted)")` |
| `Trap::Interrupt` | `SandboxTrap("interrupted (epoch deadline / wall-clock)")` |
| Other | `SandboxTrap(guest trap message)` |

## ExecLimits (`limits.rs`)

Three independent axes bound every sandbox run:

```rust
pub const DEFAULT_FUEL: u64 = 5_000_000;                       // 5M instructions
pub const DEFAULT_WALL_TIMEOUT: Duration = Duration::from_secs(5);
pub const DEFAULT_MAX_MEMORY_BYTES: usize = 32 * 1024 * 1024;  // 32 MiB
pub const DEFAULT_MAX_INSTANCES: usize = 16;

pub struct ExecLimits {
    pub fuel: u64,
    pub wall_timeout: Duration,
    pub max_memory_bytes: usize,
    pub max_instances: usize,
}
```

- **Fuel** is the primary deterministic guard (traps `OutOfFuel` on exhaustion).
- **Epoch + wall-clock** is a non-deterministic backstop for host-call blocking.
- **Memory/instances** are capped via `StoreLimits`.

## WASI capability injection (`wasi_caps.rs`)

**Philosophy**: Zero ambient authority. A synthesized guest starts with an empty
`WasiCtxBuilder` — no inherited stdio, env, args, or network. Only what the
node's verified `Capability` declares is granted back.

```rust
pub fn build_wasi_ctx(cap: &Capability) -> Result<WasiCtx>
```

`build_wasi_ctx` does three things:

1. **Filesystem**: Only explicitly declared preopened directories, with
   narrowest permissions (read-only unless `writable: true`).
2. **Network**: Deny-by-default. TCP enabled only if `NetGuard::has_socket_rules()`
   is true. UDP always off. DNS lookup always off. Each outbound address checked
   via `socket_addr_check` callback → `guard.is_allowed(&addr)`.
3. **Clock**: `Denied` or `Fixed` inject deterministic fixed clocks
   (`FixedWallClock` returns `Duration::ZERO`, `FixedMonotonicClock` returns `0`).
   `Wall` grants the real clock.

## NetGuard (`net_guard.rs`)

```rust
pub struct NetGuard {
    allowed: Vec<(IpAddr, Option<u16>)>,
    host_rules: usize,  // hostname rules not enforceable at socket layer
}
```

Rules whose `host` parses as an `IpAddr` are socket-enforceable. Hostname rules
cannot be enforced at the socket layer (they need DNS, deferred to `wasi-http`
in V2+). An empty allowlist denies everything.

## Blueprint Cache (`blueprint_cache.rs`)

Three-layer content-addressed cache (KTD2):

### Layer 1 — signature → `.wasm` (skip synthesis + rustc)

```rust
pub fn tdag_signature(tdag: &TDag) -> Result<String>   // blake3 of canonical serde_json
pub fn wasm_hash(wasm: &[u8]) -> String                 // blake3 of wasm bytes
pub fn wasm_for_signature(&self, signature: &str) -> Option<Vec<u8>>
pub fn store_wasm(&self, signature: &str, wasm: &[u8]) -> Result<()>
```

The signature is derived from the canonicalized t-DAG structure (node types +
edges + WIT contracts), **not** transient data values. Two intents with the
same structural t-DAG share a blueprint.

### Layer 2 — `.wasm` hash → AOT artifact (skip Cranelift)

```rust
pub fn module_for_wasm(&self, engine: &Engine, wasm: &[u8]) -> Result<Module>
```

1. Compute `wasm_hash(wasm)` → path `{root}/aot/{hash}.cwasm`
2. Try to read the artifact; if found, attempt `aot::load_precompiled(engine, &bytes)`
3. If deserialization fails (stale/corrupt/incompatible engine version) →
   delete the artifact (fail-closed) and recompile
4. On a miss: `aot::precompile(engine, wasm)`, write to disk, then load

### Layer 3 — warm Engine

Held by `Sandbox`; combined with the AOT artifact gives µs-scale instantiation.
Cold synthesis (2–8s rustc + tens-to-hundreds ms Cranelift) is unavoidable on
first sight of a novel intent; the blueprint's sub-ms targets are only
achievable on cache hit.

## AOT precompile (`aot.rs`)

```rust
pub fn precompile(engine: &Engine, wasm: &[u8]) -> Result<Vec<u8>>
pub fn load_precompiled(engine: &Engine, artifact: &[u8]) -> Result<Module>
```

`precompile` calls `engine.precompile_module(wasm)` — turns validated `.wasm`
into a serialized, engine-native AOT artifact via Cranelift.

`load_precompiled` calls `unsafe { Module::deserialize(engine, artifact) }` —
skips Cranelift entirely. **Safety contract**: caller must pass only artifacts
AETHER itself wrote to its content-addressed cache. Wasmtime stamps a
compatibility header and errors on mismatch, so stale/corrupt artifacts fail
closed → `AetherError::IntegrityFailed`.

## Tests

| File | What it tests |
|------|---------------|
| `tests/sandbox.rs` | Happy path returns 42; infinite loop traps on fuel; missing export errors; repeated runs are independent |
| `tests/capabilities.rs` | Zero authority by default; socket allowlist permits only declared addresses; declared preopen granted; nonexistent preopen denied |
| `tests/blueprint_cache.rs` | Layer 1 wasm round-trips by signature; AOT artifact persisted and runnable; warm load faster than cold precompile (`#[ignore]` benchmark) |

## Watch-outs for future changes

- **Never relax deny-by-default** — a `Capability::default()` must grant zero
  authority. Any new WASI feature must be opt-in via `Capability`.
- **AOT artifacts are `unsafe` to load** — only load artifacts from AETHER's
  own content-addressed cache. Never deserialize untrusted artifacts. The
  cache key includes the Wasmtime/Cranelift version; stale artifacts are
  discarded automatically.
- **Epoch interruption requires Cranelift** — it is incompatible with Winch.
  Do not switch to Winch without removing the epoch backstop.
- **One `Store` = one ephemeral execution unit** — never share a `Store` across
  transactions. Each run gets a fresh `Store` with its own fuel and limits.
- **`NetGuard` hostname rules are not enforced at the socket layer** — they are
  counted via `unenforceable_host_rule_count()` and need `wasi-http` for
  enforcement. Do not assume a hostname `NetRule` is enforced in V1.
