# Ledger (USL — Unified Semantic Ledger)

The `aether-ledger` crate implements AETHER's USL over CozoDB/SQLite. It stores
an append-only event stream, keeps transaction time and valid time as explicit
columns, and hash-chains each event so the stream is tamper-evident.

Source: `crates/ledger/src/`.

## Module map

| Module | File | Responsibility |
|--------|------|----------------|
| `cozo_store` | `cozo_store.rs` | `CozoLedger`: concrete `Ledger` trait impl over CozoDB |
| `schema` | `schema.rs` | CozoDB relation DDL + query scripts |
| `hashchain` | `hashchain.rs` | Chain verification + `GENESIS_HASH` |
| `temporal` | `temporal.rs` | Bi-temporal `as_of` reconstruction |

Public re-exports: `CozoLedger`, `verify_chain`, `GENESIS_HASH`.

## Storage model (`schema.rs`)

A single append-only relation `events`, keyed by a dense `idx` (insertion
order = chain order). Both temporal axes are explicit columns:

```
:create events {
    idx: Int
    =>
    id: String,
    tx_time: Int,
    valid_from: Int,
    valid_to: Int?,
    kind: String,
    payload: String,
    prev_hash: String,
    curr_hash: String,
}
```

The comments in `schema.rs` note an important implementation choice: the plan
(KTD5) suggested CozoDB's native `Validity` for transaction time, but the
current implementation keeps both temporal axes as explicit integer columns.
This keeps the two axes distinct (the KTD5 requirement) with a simpler query
surface. Native `Validity` remains an available optimization.

### Query scripts

| Constant | Purpose |
|----------|---------|
| `CREATE_EVENTS` | Create the relation (idempotent) |
| `SELECT_ALL` | Select every event in chain order (`:order idx`) |
| `SELECT_TAIL` | The latest event's `idx` + `curr_hash` only (one row) — O(1)-ish tail lookup |
| `PUT_EVENT` | Append one event via `:put` |

`SELECT_TAIL` is the key optimization: appends read only the tail row rather
than materializing the whole stream, which keeps appends effectively
constant-time. The previous approach (materializing all events per append) was
O(n²) over repeated appends.

## CozoLedger (`cozo_store.rs`)

```rust
pub struct CozoLedger { db: DbInstance }

impl CozoLedger {
    pub fn open(path: impl AsRef<Path>) -> Result<Self>
    fn ensure_schema(&self) -> Result<()>
    fn all_events(&self) -> Result<Vec<LedgerEvent>>
    fn tail(&self) -> Result<Option<(i64, String)>>
}
```

`open` creates the CozoDB/SQLite database (if absent) and ensures the `events`
relation exists. `ensure_schema` checks `::relations` and only runs
`CREATE_EVENTS` if the relation doesn't yet exist.

### Append path

```rust
impl Ledger for CozoLedger {
    fn append_event(&mut self, mut event: LedgerEvent) -> Result<()>
}
```

1. `tail()` → latest `(idx, curr_hash)`, or `None` if empty
2. `idx = tail.idx + 1` (or 0 for the first event)
3. `event.prev_hash = tail.curr_hash` (or `GENESIS_HASH` for the first event)
4. `event.sealed()` → sets `curr_hash = blake3(canonical(event) || prev_hash)`
5. `:put events { idx => ... }` — write the row

The event is sealed **before** being persisted. No `UPDATE` or `DELETE` is ever
issued; retraction is a `Retract` event in the same stream.

### Read and verification paths

- `all_events()` loads the full stream in chain order via `SELECT_ALL`.
- `query_as_of(tx_time, valid_time)` delegates to `temporal::as_of` after
  materializing the stream.
- `verify_chain()` delegates to `hashchain::verify_chain` over the full stream.
- `latest_hash()` returns the tail's `curr_hash` if any event exists.

## Hash chain (`hashchain.rs`)

```rust
pub const GENESIS_HASH: &str = "GENESIS";

pub fn verify_chain(events: &[LedgerEvent]) -> Result<()>
```

Each `LedgerEvent` seals `curr_hash = blake3(canonical(event) || prev_hash)`
(computed in the SDK via `LedgerEvent::sealed()`). A valid chain has:

1. Every event's `curr_hash` matches its recomputed content hash
   (`event.hash_is_valid()`)
2. Every event's `prev_hash` links to the previous event's `curr_hash`
3. The first event links to `GENESIS_HASH`

Any edit to a past event's content, ordering, or hashes breaks the chain and
returns `AetherError::IntegrityFailed`.

## Bi-temporal queries (`temporal.rs`)

```rust
pub fn as_of(events: &[LedgerEvent], tx_time: Timestamp, valid_time: Timestamp) -> Vec<LedgerEvent>
```

Two distinct, non-collapsing temporal axes:
- **Transaction time** (`tx_time`) — when the engine recorded the event
- **Valid time** (`valid_from`/`valid_to`) — when the fact is true in the
  modeled world

`as_of` returns events visible at a `(tx_time, valid_time)` point: those
recorded at or before the query transaction time whose valid-time window covers
the query valid time. `valid_to = None` means "still valid."

## Correction log (R10)

The correction log (`CompileFailure`, `VerificationRejection`,
`HumanIntervention`) lives in the **same** stream as assertions and retractions.
Every record participates in the hash chain. This is a deliberate design
decision: corrections are first-class events, not side-channel data.

The orchestrator (`cli/src/orchestrator.rs`) appends correction events:
- On synthesis/compile failure → `EventKind::CompileFailure`
- On FVL rejection → `EventKind::VerificationRejection`
- On HITLC decision → `EventKind::HumanIntervention`

## Tests

`crates/ledger/tests/ledger.rs` exercises the expected end-to-end behavior:
- Fresh ledger opens with no latest hash
- Appends can mix assertions, correction-log entries, and retractions
- The chain verifies after appends
- Bi-temporal queries return the right subset at a given tx_time and valid_time
- Reopening the same database preserves data and the chain still verifies

Inline tests in `hashchain.rs` verify: well-formed chain accepted, tampered
payload rejected, broken link rejected. Inline tests in `temporal.rs` verify:
events recorded after query tx_time excluded, valid-time window respected, facts
not yet valid excluded.

## Watch-outs for future changes

- **Preserve append-only semantics** — corrections belong in the same stream as
  events, not in a side channel. Never issue `UPDATE` or `DELETE`.
- **If you touch the schema, update the row-to-event mapping and the append
  parameter order together** — the `PUT_EVENT` script and the `row_to_event`
  function must stay in sync.
- **If you change the append logic, make sure the tail query still returns enough
  information** to chain the next event without scanning the entire stream. The
  O(1)-ish tail optimization is load-bearing for repeated appends.
- **The `Ledger` trait is the swap boundary** (KTD5) — any new backend
  implements the trait, and the rest of the system doesn't change.
- **Ledger behavior is covered by `crates/ledger/tests/ledger.rs`** — that test
  is the best first check after edits.
