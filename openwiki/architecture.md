# Architecture

AETHER's architecture is built around six subsystems wired into a single
orchestrating binary. The runtime path flows: intent → t-DAG synthesis →
blueprint cache check → (cache miss: compile in locked sandbox) → FVL gate →
AOT precompile → Wasmtime sandbox execution → ledger append.

## Runtime dataflow

```
intent.json + raw input
       │
       ▼
  ┌─────────── IPSE (compiler) ───────────┐
  │ SAA: intent → t-DAG (Claude, strict tool) │
  │ ExecutionGraph: validate acyclic + toposort │
  │ CA: node → Rust source (Claude)            │
  │ CRA: rustc diagnostics → patch (Claude)    │
  │ RepairLoop: generate→compile→repair (≤4×)  │
  │ RustcDriver: rustc → wasm32-wasip2         │
  │   (seccomp + rlimit locked sandbox)        │
  └──────────────────────────────────────────┘
       │ t-DAG signature
       ▼
  Blueprint Cache hit? ──── yes ──→ load .wasm
       │ no
       ▼
  RustcDriver.compile → .wasm bytes
       │
       ▼
  ┌─────────── FVL (verifier) ────────────┐
  │ MetaSchema: validate mutation JSON     │
  │ Z3InvariantEngine: prove ¬invariant    │
  │   is Unsat (fail-closed on Sat/Unknown) │
  └────────────────────────────────────────┘
       │ verified .wasm
       ▼
  AOT precompile (Engine::precompile_module)
       │
       ▼
  ┌─────────── TAR (runtime) ─────────────┐
  │ Sandbox: fresh Store per transaction   │
  │ Fuel (5M instructions) + epoch +       │
  │   tokio timeout (5s wall) backstop     │
  │ WASI caps: deny-by-default, only       │
  │   declared preopens/net/clock granted  │
  │ BlueprintCache: store .wasm + AOT      │
  └────────────────────────────────────────┘
       │ state delta (Mutation)
       ▼
  ┌─────────── USL (ledger) ──────────────┐
  │ CozoLedger: append-only, hash-chained  │
  │ Bi-temporal: tx_time + valid_from/to   │
  │ Correction log: CompileFailure,        │
  │   VerificationRejection,               │
  │   HumanIntervention — same stream      │
  └────────────────────────────────────────┘
```

Source: `cli/src/orchestrator.rs` (`Orchestrator::run`, `execute_node`, `gate`).

## Two-tier verification

AETHER uses a defense-in-depth verification strategy:

### Hot path (per transaction, milliseconds)

Runs on every pipeline execution. Composed in `Orchestrator::gate()`:

1. **Meta-schema validation** (`MetaSchema::validate_mutation`) — flat
   closed-world JSON Schema check against
   `crates/verifier/schemas/mutation.schema.json`. Rejects unknown fields, wrong
   types, missing required fields. Sub-millisecond.
2. **Z3 invariant proof** (`Z3InvariantEngine::prove_preserved`) — encodes
   pre-state + mutation delta, asserts the negation of each hardcoded
   invariant, and expects Z3 to return `Unsat`. Returns `Sat` (counterexample)
   or `Unknown` (timeout) → **fail-closed rejection**.

Invariants are **hardcoded static Rust** (`crates/verifier/src/invariants.rs`),
never LLM-authored. The LLM only proposes mutations; the FVL proves they cannot
violate any invariant. This confines hallucination to the safe failure mode
(proposed mutation gets rejected).

### Offline tier (CI / synthesis-admission, seconds to minutes)

Gates **templates and the invariant set** — never on the request path:

- **Kani** (`verification/kani/src/lib.rs`) — `#[kani::proof]` harnesses over
  the bounded code shapes the Compiler Agent may emit. Proves no panic, no
  arithmetic overflow, and invariant-preservation for `kani::any()` inputs under
  bounded unwinds.
- **Apalache** (`verification/tla/ledger.tla`) — TLA+ state-machine model of
  the USL. Proves structural invariants (dense monotonic indices, chain
  integrity) over all reachable states up to `MaxLen = 5`.

Both run in CI via `.github/workflows/verify.yml`. See
[Verification](verification.md) for details.

## Crate dependency graph

```
                    aether-sdk
                   /    |    \    \      \
                  /     |     \    \      \
     aether-compiler  runtime  verifier  ledger
                  \     |      /      /
                   \    |     /      /
                    \   |    /      /
                     cli (orchestrator)
```

- `aether-sdk` is the foundation: pure data types, the `Ledger` trait, and
  `AetherError`. Every other crate depends on it.
- `aether-compiler` depends on `sdk` (types only). It does not depend on
  runtime, verifier, or ledger — it produces `.wasm` bytes and t-DAGs.
- `aether-runtime` depends on `sdk`. It executes `.wasm` in a sandbox.
- `aether-verifier` depends on `sdk`. It gates mutations.
- `aether-ledger` depends on `sdk`. It persists events.
- `cli` depends on all five and wires them together in `Orchestrator`.

The `Ledger` trait in `sdk` keeps the store swappable (KTD5): the default is
CozoDB/SQLite, but the trait allows dropping in an alternative backend without
touching IPSE/TAR/FVL.

## Key design decisions

These are abbreviated from the V1 plan's KTD1–KTD9. See
[the plan](../docs/plans/2026-06-24-001-feat-aether-engine-v1-plan.md) for full
rationale.

| ID | Decision | Why |
|----|----------|-----|
| KTD1 | Cargo workspace of crates, one orchestrating binary | High cohesion / low coupling per subsystem; `sdk` as shared vocabulary |
| KTD2 | Blueprint Cache is load-bearing, not an optimization | Cold `rustc` compile is 2–8s; cache hit is sub-ms. Targets are only achievable on hit |
| KTD3 | Invariants are hardcoded static Rust; LLM only proposes mutations | Confines hallucination to "proposing a mutation that gets rejected" |
| KTD4 | Two-tier FVL | Hot path (Z3, ms) + offline (Kani/Apalache, CI). WASM symbolic execution deferred |
| KTD5 | Ledger behind swappable `Ledger` trait; CozoDB default | CozoDB is single-maintainer; trait allows swap without touching IPSE/TAR |
| KTD6 | Claude over raw HTTP; strict tool use | No official Rust SDK; pinned HTTP gives control over retries/streaming/headers |
| KTD7 | Reflexion-shaped compile-critic loop | Actor (CA) → Evaluator (rustc) → Self-Reflection (CRA). Hard cap + stagnation detection |
| KTD8 | Compilation is the real attack surface; lock the build | `build.rs`/proc-macros run arbitrary native code. Seccomp + rlimits, no network, no secrets |
| KTD9 | t-DAG enforced acyclic at construction | `petgraph` `toposort` rejects cycles at build, not at runtime |

## Where to start when changing things

| If you are changing… | Start here |
|----------------------|-----------|
| Shared types crossing subsystem boundaries | [Domain concepts](domain-concepts.md), `crates/sdk/src/types.rs` |
| LLM interaction, agent prompts, or synthesis | [Synthesis](synthesis.md), `crates/compiler/src/` |
| Sandbox execution, capabilities, or caching | [Runtime](runtime.md), `crates/runtime/src/` |
| Safety invariants, Z3, or meta-schema | [Verification](verification.md), `crates/verifier/src/` |
| Ledger storage, hash chain, or temporal queries | [Ledger](ledger.md), `crates/ledger/src/` |
| CLI commands, orchestration flow, or e2e tests | [Operations](operations.md), `cli/src/` |
| Build environment, tests, or CI | [Operations](operations.md), `docs/CONTRIBUTING.md` |
