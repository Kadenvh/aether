# AETHER offline verification tier (U14, R14)

Two checkers gate **templates and the invariant set** at admission time / in CI —
never on the request path. They complement the runtime Z3 gate (U7), which proves
individual live mutations.

## Kani — proofs over synthesized-code templates

`verification/kani/` proves properties of the bounded code shapes the Compiler
Agent may emit: money stays integer cents (KTD4), arithmetic is overflow-checked,
and the non-negativity invariant is preserved by the deposit/convert templates.

```sh
cd verification/kani
cargo kani --default-unwind 1
```

Expected: `4 successfully verified harnesses, 0 failures`. Harnesses are
loop-free, so `--default-unwind 1` is sufficient; bump it if a template grows a
bounded loop. To confirm the proofs are *live* (negative test), flip an assertion
(e.g. assert `post < 0` in `deposit_preserves_non_negativity`) and re-run — Kani
must report a failing harness with a counterexample.

## Apalache — ledger state-machine model

`verification/tla/ledger.tla` models the USL as an append-only, hash-chained log
and proves the structural invariant `Inv` (dense, monotonic indices + chain
integrity) over all reachable states.

```sh
cd verification/tla
apalache-mc check --config=ledger.cfg --length=6 ledger.tla
```

Expected: `The outcome is: NoError`. Bounds are explicit: `MaxLen = 5` (in
`ledger.cfg`) and `--length=6`. To confirm liveness of the check, break
`ChainIntact` (e.g. link every event to `Genesis`) and re-run — Apalache must
report an invariant violation with a trace.

## CI

`.github/workflows/verify.yml` runs both on every push/PR. Both are fail-closed:
a failing proof or a model violation fails the build.

## Scope

- These gate *templates and invariants*, not live mutation values (that is U7).
- WASM symbolic execution is out of scope for V1 (research-grade; deferred).
