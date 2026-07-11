# Domain Concepts

The shared vocabulary that crosses every subsystem boundary lives in
`crates/sdk/`. Every other crate depends on `aether-sdk` for these types.
Understanding them is prerequisite to working in any subsystem.

Source: `crates/sdk/src/types.rs`, `crates/sdk/src/error.rs`,
`crates/sdk/src/lib.rs`.

## Money: Cents (never f64)

```rust
#[serde(transparent)]
pub struct Cents(pub i64);
```

All money in AETHER is integer cents. Floating-point is never used for
money-bearing values (KTD4). `Cents` provides `checked_add` / `checked_sub`
that return `AetherError::Ledger` on overflow/underflow, and `is_non_negative`.

This invariant is enforced at multiple levels: the CA system prompt instructs
the LLM to use `i64` cents, the Kani proofs verify template arithmetic never
produces negative balances, and the Z3 engine models money as integer
arithmetic (QF_LIA).

## Intent

```rust
pub struct Intent {
    pub objective: String,
    pub invariants: Vec<String>,       // references to hardcoded FVL invariants
    pub input: Option<IoDescriptor>,
    pub output: Option<IoDescriptor>,
}
```

An intent is a declarative description of what the pipeline should do. The
`invariants` field holds **references** to invariants the FVL registry defines
— the LLM cannot introduce new invariants (KTD3). `validate()` rejects empty
objectives. `validate_invariant_refs()` (in the compiler crate) rejects
references to unknown invariants.

Example (`examples/utility-bills/intent.json`):
```json
{
  "objective": "Import partner utility bills, convert Euros to USD, flag lines with anomalous variance > 20%, and save to ledger",
  "invariants": ["usd_amount >= 0.0", "partner_id must match known_partners in local state"]
}
```

The known invariants V1 recognizes are defined in
`crates/compiler/src/intent_parse.rs::default_known_invariants()`:
- `"usd_amount >= 0.0"`
- `"partner_id must match known_partners in local state"`
- `"balance >= 0"`

## Temporal DAG (t-DAG)

A t-DAG is a directed acyclic graph of typed pipeline steps. The SDK types are
pure data; the compiler crate's `ExecutionGraph` enforces acyclicity.

```rust
pub struct TDag {
    pub nodes: Vec<TDagNode>,
    pub edges: Vec<TDagEdge>,
}

pub struct TDagNode {
    pub id: NodeId,        // String
    pub kind: NodeKind,    // Ingest | Transform | Flag | Persist | ApiSync
    pub spec: serde_json::Value,
}

pub struct TDagEdge {
    pub from: NodeId,
    pub to: NodeId,
    pub kind: EdgeKind,    // DataFlow | Temporal
}
```

`NodeKind` determines what the node does and what capabilities it receives:

| Kind | What it does | Capability |
|------|-------------|------------|
| `Ingest` | Read/parse input from a read-only preopened dir | Read-only preopen of `spec.input_path` → `/input` |
| `Transform` | Pure function over records (e.g. currency conversion) | Zero authority (`Capability::none()`) |
| `Flag` | Detect/annotate anomalies using checked arithmetic | Zero authority |
| `Persist` | Compute final value; host appends to ledger | Zero authority |
| `ApiSync` | Synchronize to a single approved outbound endpoint | Single `NetRule` for the pinned host:port |

The `ExecutionGraph::build()` constructor (in `crates/compiler/src/tdag.rs`)
validates at construction time: duplicate node IDs, dangling edge references,
and cycles are all rejected. The topological sort is deterministic (tie-broken
by insertion index) so execution order is stable across runs.

## Capability (zero ambient authority)

```rust
pub struct Capability {
    pub preopened_dirs: Vec<PreopenedDir>,
    pub net_allowlist: Vec<NetRule>,
    pub clock: ClockPolicy,         // Denied (default) | Fixed | Wall
    pub fuel_budget: u64,
}
```

`Capability::default()` is **zero authority** — no filesystem, no network, no
clock. The runtime grants a guest only what its verified `Capability` declares.
This is deny-by-default capability-based security.

- `PreopenedDir`: maps a host path to a guest path, with a `writable` flag.
- `NetRule`: a host + optional port. IP literals are socket-enforceable;
  hostnames require `wasi-http` (deferred in V1).
- `ClockPolicy::Denied` or `Fixed` injects a deterministic fixed clock so the
  guest cannot observe real wall-clock time. `Wall` grants the real clock.

The node library (`crates/compiler/src/nodes.rs`) derives the least-privilege
capability for each node kind from its spec.

## Mutation

```rust
pub struct Mutation {
    pub subject: String,
    pub predicate: String,
    pub object: String,
}
```

A mutation is an RDF-like triple assertion (subject-predicate-object). It is
the proposed state change a pipeline produces. Every mutation passes the FVL
gate (meta-schema + Z3) before being appended to the ledger.

The meta-schema (`crates/verifier/schemas/mutation.schema.json`) requires all
three fields to be non-empty strings, with `additionalProperties: false`
(closed-world).

## LedgerEvent and EventKind

```rust
pub struct LedgerEvent {
    pub id: String,
    pub kind: EventKind,
    pub payload: serde_json::Value,
    pub tx_time: Timestamp,           // transaction time (when recorded)
    pub valid_from: Timestamp,        // valid time start
    pub valid_to: Option<Timestamp>,  // valid time end (None = still valid)
    pub prev_hash: String,            // hash of previous event
    pub curr_hash: String,            // blake3(canonical(event) || prev_hash)
}
```

```rust
pub enum EventKind {
    Assert,                 // assert a mutation
    Retract,                // retract a prior assertion
    CompileFailure,         // correction log: synthesis failed
    VerificationRejection,  // correction log: FVL rejected a mutation
    HumanIntervention,      // correction log: signed HITLC decision
}
```

The correction log (`CompileFailure`, `VerificationRejection`,
`HumanIntervention`) lives in the **same** stream as assertions and retractions
(R10) — so every record participates in the hash chain.

`LedgerEvent` provides:
- `compute_hash()` — BLAKE3 over canonical JSON (all fields except `curr_hash`).
- `sealed(self)` — sets `curr_hash = compute_hash()`.
- `hash_is_valid(&self)` — tamper check: recomputed hash matches stored hash.

## Timestamp

```rust
#[serde(transparent)]
pub struct Timestamp(pub i64);
```

Epoch-second timestamps. The SDK never reads a wall clock; the caller (CLI
layer) supplies transaction-time. This makes runs deterministic and testable.

## The Ledger trait

```rust
pub trait Ledger {
    fn append_event(&mut self, event: LedgerEvent) -> Result<()>;
    fn latest_hash(&self) -> Option<String>;
    fn query_as_of(&self, tx_time: Timestamp, valid_time: Timestamp) -> Result<Vec<LedgerEvent>>;
    fn verify_chain(&self) -> Result<()>;
}
```

Repository-pattern trait (KTD5). The concrete CozoDB implementation lives in
`crates/ledger/`. The trait keeps the store swappable — an Oxigraph or
hand-rolled rusqlite backend can be dropped in without touching IPSE/TAR/FVL.

## AetherError

```rust
pub enum AetherError {
    IntentInvalid(String),
    UnknownInvariant(String),
    VerificationRejected { stage: String, reason: String },
    CompileFailed(String),
    Llm(String),
    LlmRefusal(String),         // distinct from Llm — model safety refusal
    SandboxTrap(String),
    CapabilityDenied(String),
    Ledger(String),
    IntegrityFailed(String),
    Serde(serde_json::Error),
    Io(String),
}
```

Every fallible boundary returns `AetherError`. Key design notes:
- `LlmRefusal` is distinct from `Llm` so a model safety refusal is
  programmatically distinguishable from a transport/API error.
- `VerificationRejected` carries both `stage` and `reason` so the pipeline knows
  which gate failed (`"z3_invariant"` or `"meta_schema"`).
