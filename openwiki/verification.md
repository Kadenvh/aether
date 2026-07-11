# Verification (FVL — Formal Verification Layer)

AETHER uses a two-tier verification strategy. The **hot path** (per transaction,
milliseconds) gates individual live mutations. The **offline tier** (CI /
synthesis-admission, seconds to minutes) gates synthesized-code templates and
the ledger state machine.

Source: `crates/verifier/src/`, `verification/`.

## Hot path: Z3 invariant engine (`z3_engine.rs`)

```rust
pub struct Z3InvariantEngine;

impl Z3InvariantEngine {
    pub fn new() -> Self
    pub fn prove_preserved(&self, invariants: &[Invariant], deltas: &[MutationDelta]) -> Result<()>
}
```

### Proof strategy

To prove a proposed mutation cannot violate an invariant, the engine encodes
the **negation** and asks Z3 whether it is satisfiable:

1. Assume a valid pre-state (every invariant holds).
2. Apply the mutation's delta to compute the post-state.
3. Assert that *some* post-invariant fails.
4. `solver.check()`:
   - `Unsat` → proven safe (no valid pre-state can be driven into violation) → `Ok`
   - `Sat` → counterexample exists → `Err(VerificationRejected { stage: "z3_invariant" })`
   - `Unknown` → timeout/incomplete → **fail-closed rejection**

Constraints stay in the decidable **QF_LIA** fragment (linear integer arithmetic
over cents), so `check()` always terminates.

Z3 contexts are not `Send`, so one engine per worker; cheap to construct.

### Hardcoded invariants (`invariants.rs`)

```rust
pub enum Invariant {
    NonNegative { var: String },
}

pub fn default_invariants() -> Vec<Invariant> {
    vec![
        Invariant::non_negative("balance"),
        Invariant::non_negative("usd_amount"),
    ]
}
```

Invariants are authored **only** in static Rust — never by the LLM (KTD3). The
LLM only proposes mutations; the Z3 engine proves they cannot violate any
invariant. V1 covers the **arithmetic non-negativity class** (money is integer
cents, never `f64`). Set-membership invariants (e.g. "partner_id must be a known
partner") are enforced by the registry/meta-schema, not the SMT engine.

### MutationDelta

```rust
pub struct MutationDelta {
    pub var: String,
    pub delta: i64,  // may be negative
}
```

A proposed change to one state variable, in cents.

## Hot path: meta-schema gate (`meta_schema.rs`)

```rust
pub struct MetaSchema { validator: Validator }

impl MetaSchema {
    pub fn new() -> Result<Self>
    pub fn validate_mutation(&self, mutation: &Value) -> Result<()>
}
```

Every proposed mutation is validated against a static, closed-world JSON Schema
before it can be appended to the ledger. This is deliberately **flat record
validation** (typed structs + `jsonschema`), not SHACL/RDF ontology checking.
Runs in-process, sub-ms.

The schema is embedded at compile time via
`include_str!("../schemas/mutation.schema.json")`. On validation failure, all
errors from `validator.iter_errors(mutation)` are joined into a single string
and returned as `VerificationRejected { stage: "meta_schema" }`.

### Mutation schema (`schemas/mutation.schema.json`)

```json
{
  "type": "object",
  "properties": {
    "subject":   { "type": "string", "minLength": 1 },
    "predicate": { "type": "string", "minLength": 1 },
    "object":    { "type": "string", "minLength": 1 }
  },
  "required": ["subject", "predicate", "object"],
  "additionalProperties": false
}
```

A mutation is a flat subject-predicate-object triple. All three fields are
non-empty strings. No additional properties are allowed (closed-world).

## How the hot-path gate composes

The `Orchestrator::gate()` method in `cli/src/orchestrator.rs` composes both
checks:

```rust
fn gate(&self, mutation: &Mutation, result_cents: i64) -> Result<()> {
    self.meta_schema.validate_mutation(&mutation_json(mutation))?;
    let deltas = [MutationDelta::new(RESULT_VAR, result_cents)];
    self.z3.prove_preserved(&self.invariants, &deltas)
}
```

If either stage fails, the rejection is recorded as a `VerificationRejection`
event in the ledger (R10), and the pipeline aborts for that transaction.

## Offline tier: Kani proofs (`verification/kani/`)

`verification/kani/src/lib.rs` proves properties of the bounded code shapes the
Compiler Agent may emit. These are **templates** — not live mutations — gated at
admission time / in CI.

### Template functions

```rust
pub fn apply_deposit(balance_cents: i64, amount_cents: i64) -> Option<i64>
pub fn checked_convert(minor_units: i64, rate_ppm: i64) -> Option<i64>
pub fn flag_variance(value_cents: i64, average_cents: i64, pct: i64) -> Option<bool>
```

All use checked arithmetic and return `Option` — never panic, never silently
overflow, never produce negative results from non-negative inputs.

### Proof harnesses

Four `#[kani::proof]` harnesses over `kani::any()` inputs:

| Harness | Proves |
|---------|--------|
| `deposit_preserves_non_negativity` | A deposit onto a non-negative balance never yields a negative balance |
| `negative_deposit_is_rejected` | A negative "deposit" is always rejected |
| `conversion_is_non_negative` | Converting a non-negative amount at a non-negative rate is non-negative |
| `variance_flag_never_panics` | The variance flag never panics for any inputs |

Harnesses are loop-free, so `--default-unwind 1` is sufficient.

```sh
cd verification/kani && cargo kani --default-unwind 1
# Expected: 4 successfully verified harnesses, 0 failures
```

## Offline tier: Apalache ledger model (`verification/tla/`)

`verification/tla/ledger.tla` models the USL as an append-only, hash-chained log
and proves the structural invariant `Inv` over all reachable states.

### Invariants

| TLA+ invariant | What it checks |
|----------------|----------------|
| `DenseMonotonic` | Indices are dense and monotonic: event at position `i` has `idx = i` |
| `ChainIntact` | First event links to Genesis; every later event links to its predecessor's `idx` |
| `Inv` | Conjunction of `DenseMonotonic` and `ChainIntact` |

Bounds: `MaxLen = 5` (in `ledger.cfg`), `--length=6`. Stutters when full so
behaviours are infinite (no deadlock flagged).

```sh
cd verification/tla && apalache-mc check --config=ledger.cfg --length=6 ledger.tla
# Expected: The outcome is: NoError
```

## CI workflow (`.github/workflows/verify.yml`)

Two parallel GitHub Actions jobs run on every push to `main` and on PRs:

1. **Kani** — installs `kani-verifier`, runs `cargo kani --default-unwind 1` in
   `verification/kani/`.
2. **Apalache** — installs Java 21 + Apalache, runs
   `apalache-mc check --config=ledger.cfg --length=6 ledger.tla` in
   `verification/tla/`.

Both are fail-closed: a failing proof or a model violation fails the build.

## Tests

| File | What it tests |
|------|---------------|
| `crates/verifier/tests/invariants.rs` | Deposit preserves non-negativity; unguarded withdrawal rejected; no-op mutation OK; unrelated delta OK; one unsafe delta among many rejects |
| `crates/verifier/tests/meta_schema.rs` | Gate admits valid mutation; rejects missing field, unknown field, empty string, wrong type |

## Watch-outs for future changes

- **Invariants are authored only in `invariants.rs`** — never by the LLM, never
  widened from intent. Adding a new invariant means adding it to
  `default_invariants()` and writing a test for both the safe and violating
  case.
- **Z3 `Unknown` is a rejection** — do not change the `Unknown` arm to `Ok` or
  a warning. Fail-closed is the security boundary.
- **The offline tier gates templates and the invariant set, not live mutations**
  — that is the Z3 engine's job. Do not conflate the two tiers.
- **Kani unwind bounds are explicit** — `--default-unwind 1` is sufficient only
  because templates are loop-free. If a template grows a bounded loop, bump the
  unwind and document the bound.
- **Apalache `MaxLen` bounds are explicit** — if you change the ledger state
  machine, update the model and the bounds together.
- **The mutation schema is closed-world** (`additionalProperties: false`) — do
  not relax this without a corresponding change to the `Mutation` type in the
  SDK.
