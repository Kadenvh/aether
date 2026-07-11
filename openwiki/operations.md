# Operations

How to build, test, run, and operate AETHER. This page covers the CLI, build
prerequisites, testing strategy, and the HITLC flow. See also the existing
[Contributing guide](../docs/CONTRIBUTING.md) and [Runbook](../docs/RUNBOOK.md).

## Prerequisites

| Requirement | Why | Notes |
|-------------|-----|-------|
| **Linux (or WSL2)** | The build sandbox (U12) uses seccomp + rlimits; Linux-only, fails closed elsewhere | Kernel ≥ 5.13 for Landlock/seccomp |
| **Rust 1.96** + `wasm32-wasip2` target | Pinned by `rust-toolchain.toml`; synthesized nodes compile to `wasm32-wasip2` | `rustup target add wasm32-wasip2` |
| **cmake + C++ toolchain** | Builds vendored Z3 for the invariant engine (U7) | `apt install cmake build-essential` |
| **JDK 21 + Apalache + Kani** | Offline verification tier only (U14) | Not needed for `cargo build`/`test` |
| **`ANTHROPIC_API_KEY`** | Live synthesis (SAA/CA/CRA call Claude) | Only for live `aether run`/`watch`; tests don't need it |

Cache-hit runs (a previously-synthesized pipeline) need no API key — synthesis
is skipped and only the runtime path executes.

The repo builds from `/mnt/c/AETHER` under WSL. Invoke cargo via:
`wsl -e bash -lc '. "$HOME/.cargo/env"; cd /mnt/c/AETHER && cargo …'`.

## Build and test commands

| Command | Description |
|---------|-------------|
| `cargo build --workspace` | Build every crate |
| `cargo test --workspace` | Run the full test suite |
| `cargo test -p <crate>` | Test one crate (e.g. `aether-runtime`) |
| `cargo fmt --all` | Format (rustfmt; required before commit) |
| `cargo clippy --all-targets` | Lint (treat warnings as errors before merge) |
| `cargo run -p aether-cli --bin aether -- run …` | Run the CLI locally |
| `cd verification/kani && cargo kani --default-unwind 1` | Offline: 4 proof harnesses |
| `cd verification/tla && apalache-mc check --config=ledger.cfg --length=6 ledger.tla` | Offline: ledger model check |

## CLI commands

### `aether run` — execute one intent

```sh
aether run --intent <file> [--input <file>] --ledger <db> \
           [--cache <dir>] [--scratch <dir>]
```

| Flag | Required | Default | Meaning |
|------|----------|---------|---------|
| `--intent` | yes | — | Path to `intent.json` (objective + invariant references) |
| `--input` | no | — | Input data source (e.g. a CSV) |
| `--ledger` | yes | — | Path to the CozoDB/SQLite ledger file (created if absent) |
| `--cache` | no | `.aether/cache` | Blueprint cache directory |
| `--scratch` | no | `.aether/scratch` | Sandboxed-compile scratch directory |

On success: prints `ok: N node(s) executed, net <cents> cents, ledger event <id>`.

### `aether watch` — daemon

```sh
aether watch --intent <file> --source <dir> --ledger <db> \
             [--cache <dir>] [--scratch <dir>]
```

Polls `--source` for new files and runs the pipeline per file. Steady-state runs
are blueprint-cache hits (synthesis bypassed). **Ctrl-C drains one final tick,
then exits** — graceful shutdown; in-flight files are processed before exit.

A dependency-free poll loop is used rather than an OS file-watch crate
(single-host V1 scope, minimal dependency surface).

### Example

```sh
export ANTHROPIC_API_KEY=sk-ant-…
aether run --intent examples/utility-bills/intent.json \
           --input ./bills.csv --ledger ./state.db
```

## Artifacts

| Path | What |
|------|------|
| `--ledger` (e.g. `state.db`) | The Unified Semantic Ledger — append-only, hash-chained event stream |
| `<cache>/blueprints/` | Layer-1: t-DAG signature → `.wasm` |
| `<cache>/aot/` | Layer-2: `.wasm` hash → engine-native AOT artifact |
| `<scratch>/` | Transient rustc inputs/outputs (safe to delete) |

## Orchestration flow (`cli/src/orchestrator.rs`)

The `Orchestrator` struct wires all subsystems and drives the t-DAG end to end:

```rust
pub struct Orchestrator<'a, S: NodeSynthesizer, L: Ledger> { ... }

impl Orchestrator {
    pub fn new(synthesizer, cache, sandbox, invariants, ledger) -> Result<Self>
    pub async fn run(&mut self, tdag: &TDag, now: Timestamp) -> Result<RunOutcome>
}
```

`run()` executes each node in topological order:

1. **Cache check**: compute t-DAG signature, look up `.wasm` in `BlueprintCache`
2. **Cache miss**: `synthesizer.synthesize(node)` → `.wasm` bytes, store in cache.
   On failure, append `CompileFailure` to the ledger and abort.
3. **AOT**: `cache.module_for_wasm(engine, &wasm)` → load or precompile
4. **Execute**: `sandbox.run_module_i32(&module, "run", &limits)` → `i32` result
5. After all nodes: propose a `Mutation` from the terminal node's output
6. **FVL gate**: `meta_schema.validate_mutation` → `z3.prove_preserved`.
   On failure, append `VerificationRejection` and abort.
7. **Ledger append**: append `EventKind::Assert` with the verified mutation

The `NodeSynthesizer` trait is the testing seam: `LlmSynthesizer` drives
CA/CRA + rustc in production, but tests use stubs that seed the cache and assert
the runtime path (no LLM, no network).

### `run.rs` — the live entry point

`run::execute()` wires the live subsystems:
1. Parse intent, validate invariant refs
2. `ReqwestTransport::from_env()` — reads `ANTHROPIC_API_KEY`
3. `SystemArchitect::plan()` → t-DAG
4. Build `LlmSynthesizer` (CA + CRA + `RustcDriver::discover(scratch)`)
5. `Orchestrator::new(...)` → `orch.run(&tdag, now())`

## HITLC (`cli/src/hitlc.rs`)

Human-In-The-Loop Consensus — terminal dry-run (R15). When a run is
low-confidence or hits a soft constraint, the engine serializes the pending
state to a deterministic JSON dry-run and surfaces it to the operator:

```rust
pub struct DryRun {
    pub reason: String,
    pub proposed: Value,
}

impl DryRun {
    pub fn document(&self) -> Value    // deterministic JSON for operator review
    pub fn render(&self) -> String     // terminal-friendly rendering
}
```

The operator's signed decision is written to the USL as a `HumanIntervention`
event. V1 ships the deterministic serialization; the interactive capture is a
thin shell over it. No GUI (R12).

## Watch daemon (`cli/src/watch.rs`, `cli/src/daemon.rs`)

`watch()` plans the t-DAG once, then polls `--source` in a loop:

```rust
loop {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => { drain one final tick; return }
        _ = tokio::time::sleep(poll) => { process_new(...) }
    }
}
```

`daemon::scan_new()` reads the directory, filters to files not yet in `seen`,
and sorts them for deterministic order. `daemon::process_new()` runs the
orchestrator per file with a deterministic timestamp (`base.0 + processed`).

## Testing strategy

- Unit tests live in `#[cfg(test)]` modules; integration tests in each crate's
  `tests/` directory.
- **Network-free by design**: the e2e tests (`cli/tests/e2e_edid.rs`,
  `watch_daemon.rs`) seed the blueprint cache with real core-wasm `.wat` modules
  so no LLM is called and no socket opens. A `NoSynth` synthesizer asserts
  synthesis never runs.
- The U12 sandbox test asserts a `socket()` call is killed by `SIGSYS` — it
  runs on Linux only (`#![cfg(target_os = "linux")]`).
- New behaviour needs a test: happy path, edge/boundary, and the failure path
  (especially for anything touching the FVL gate or the ledger).

### Key test files

| File | What it verifies |
|------|------------------|
| `cli/tests/e2e_edid.rs` | Full runtime thesis: cache → AOT → TAR → FVL → USL. Verified pipeline appends to immutable ledger; invariant-violating result rejected fail-closed and recorded. |
| `cli/tests/watch_daemon.rs` | Watch daemon processes a sequence of dropped files into a consistent ledger, one transaction per file, on the cache-hit path. |
| `crates/runtime/tests/sandbox.rs` | Fuel exhaustion traps infinite loop; happy path returns expected value. |
| `crates/runtime/tests/capabilities.rs` | Zero authority by default; socket allowlist enforced. |
| `crates/runtime/tests/blueprint_cache.rs` | Layer 1 + Layer 2 cache round-trips; AOT artifact persisted. |
| `crates/verifier/tests/invariants.rs` | Z3 admits safe mutations, rejects violations. |
| `crates/verifier/tests/meta_schema.rs` | Meta-schema admits valid mutations, rejects invalid. |
| `crates/compiler/tests/repair_loop.rs` | Repair loop converges, aborts on stagnation and iteration cap. |
| `crates/compiler/tests/saa.rs` | SAA produces valid acyclic t-DAG from recorded fixture. |
| `crates/ledger/tests/ledger.rs` | Append, bi-temporal query, chain verification, reopen. |

## Code style

- **rustfmt** (100-col) and **clippy** clean are required.
- Money is **integer `Cents`**, never `f64`.
- **Invariants are authored only in `verifier/src/invariants.rs`** — never by
  the LLM, never widened from intent.
- **Capabilities are deny-by-default** — grant only what a node's verified spec
  declares.
- Verification is **fail-closed** — on `Unknown`/timeout/ambiguity, reject.
- Every `unsafe` block carries a `// SAFETY:` comment.
- Conventional commit messages (`feat(scope): …`, `fix:`, `docs:`, …).

## PR checklist

- [ ] `cargo test --workspace` green
- [ ] `cargo fmt --all` applied; `cargo clippy --all-targets` clean
- [ ] Offline tier green if you touched invariants, templates, or the ledger
- [ ] New behaviour has tests (incl. the failure path)
- [ ] No secrets committed; `ANTHROPIC_API_KEY` only ever read from env

## Integrity, inspection, and "rollback"

The ledger is **append-only and immutable** — there is no in-place update or
delete, so there is no destructive rollback. Corrections are themselves events
(`Retract`, `CompileFailure`, `VerificationRejection`, `HumanIntervention`) in
the same hash chain.

- **Verify integrity**: the engine chain-verifies on open; tampering is detected
  and surfaced as an integrity error. `Ledger::verify_chain()` walks the chain.
- **Inspect as-of a point in time**: the ledger is bi-temporal —
  `Ledger::query_as_of(tx_time, valid_time)`.
- **Recover from a bad pipeline**: there is nothing to un-write — a rejected
  mutation never reaches the ledger (the FVL gate is fail-closed), and the
  rejection is recorded. To change behaviour, fix the intent/invariants and re-run.

## Common issues

| Symptom | Cause | Fix |
|---------|-------|-----|
| `ANTHROPIC_API_KEY is not set` | No key in env on a cache-miss run | `export ANTHROPIC_API_KEY=…` (or pre-warm the cache) |
| `locked build sandbox requires Linux` | Running synthesis on non-Linux | Run on Linux/WSL2 — the seccomp/rlimit sandbox is Linux-only and fails closed |
| `can't find crate for std` | wasip2 target missing | `rustup target add wasm32-wasip2` |
| `verification rejected at stage 'z3_invariant'` | A mutation could violate an invariant (or Z3 returned `Unknown`) | Expected fail-closed behavior; the rejection is recorded. Fix the pipeline logic |
| `repair stagnated` / `iteration cap reached` | The compile-critic loop couldn't converge | Escalates to human review (HITLC); inspect recorded `CompileFailure` lessons |
| `sandbox trap: out of fuel` | A node exceeded its instruction budget | Raise `ExecLimits.fuel` for that workload, or fix a runaway node |
| `AOT artifact rejected` | Cached artifact built by incompatible engine version | Harmless — discarded and recompiled automatically; or clear `<cache>/aot/` |

## Examples

Three example intents under `examples/`:

- **`utility-bills/intent.json`** — Import partner utility bills, convert EUR→USD,
  flag variance >20%, save to ledger. (The EDID path example.)
- **`schema-transform/intent.json`** — Ingest a CSV, rename legacy columns,
  derive a `total_cents` column, save transformed records.
- **`api-sync/intent.json`** — Ingest reconciled invoice totals, synchronize to
  the partner billing API at the approved endpoint.

All three share the `usd_amount >= 0.0` non-negativity invariant.
