# AETHER Runbook

How to operate AETHER. It is a **command-line tool, not a server** — there is no
port, no listener, and no health-check endpoint. Operation is file-system and
CLI based (a deliberate V1 scope decision).

## Environment

<!-- AUTO-GENERATED: env vars read by the source (no .env.example exists). -->

| Variable | Required | Description | Example |
|----------|----------|-------------|---------|
| `ANTHROPIC_API_KEY` | For live runs | API key the synthesis agents (SAA/CA/CRA) use to call Claude. Read from env only — never hardcoded; the engine refuses to start a live run without it. | `sk-ant-…` |
| `TMPDIR` | No (set internally) | The rustc driver points this at its scratch dir during sandboxed compiles; you do not set it. | — |

Cache-hit runs (a previously-synthesized pipeline) need no API key — synthesis is
skipped and only the runtime path executes.

## Commands

<!-- AUTO-GENERATED: from the `aether` CLI arg surface (cli/src/main.rs). -->

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

On success it prints `ok: N node(s) executed, net <cents> cents, ledger event <id>`.

### `aether watch` — daemon

```sh
aether watch --intent <file> --source <dir> --ledger <db> \
             [--cache <dir>] [--scratch <dir>]
```

Polls `--source` for new files and runs the pipeline per file. Steady-state runs
are blueprint-cache hits (synthesis bypassed). **Ctrl-C drains one final tick,
then exits** — graceful shutdown; in-flight files are processed before exit.

### Example

```sh
export ANTHROPIC_API_KEY=sk-ant-…
aether run --intent examples/utility-bills/intent.json \
           --input ./bills.csv --ledger ./state.db
```

(Under WSL: `wsl -e bash -lc '. "$HOME/.cargo/env"; cd /mnt/c/AETHER && \
ANTHROPIC_API_KEY=… cargo run -p aether-cli --bin aether -- run …'`.)

## Artifacts it creates

| Path | What |
|------|------|
| `--ledger` (e.g. `state.db`) | The Unified Semantic Ledger — append-only, hash-chained event stream |
| `<cache>/blueprints/` | Layer-1: t-DAG signature → `.wasm` |
| `<cache>/aot/` | Layer-2: `.wasm` hash → engine-native AOT artifact |
| `<scratch>/` | Transient rustc inputs/outputs (safe to delete) |

## Integrity, inspection & "rollback"

The ledger is **append-only and immutable** — there is no in-place update or
delete, so there is no destructive rollback to perform. Corrections are themselves
events (`Retract`, `CompileFailure`, `VerificationRejection`, `HumanIntervention`)
in the same hash chain.

- **Verify integrity:** the engine chain-verifies on open; tampering (edited
  payload, reordered or dropped event, broken link) is detected and surfaced as
  an integrity error. Programmatically, `Ledger::verify_chain()` walks the chain.
- **Inspect as-of a point in time:** the ledger is bi-temporal — query by
  transaction-time and valid-time via `Ledger::query_as_of(tx_time, valid_time)`.
- **Recover from a bad pipeline:** there is nothing to un-write — a rejected
  mutation never reaches the ledger (the FVL gate is fail-closed), and the
  rejection is recorded. To change behaviour, fix the intent/invariants and re-run.

## Common issues

<!-- AUTO-GENERATED: from the error surfaces in the source. -->

| Symptom | Cause | Fix |
|---------|-------|-----|
| `ANTHROPIC_API_KEY is not set; refusing to call the API` | No key in env on a cache-miss run | `export ANTHROPIC_API_KEY=…` (or pre-warm the cache) |
| `locked build sandbox requires Linux …` | Running synthesis on non-Linux | Run on Linux/WSL2 — the seccomp/rlimit sandbox is Linux-only and fails closed |
| `can't find crate for std` during compile | wasip2 target missing on the active toolchain | `rustup target add wasm32-wasip2` (target is pinned to Rust 1.96) |
| `verification rejected at stage 'z3_invariant'` | A mutation could violate an invariant (or Z3 returned `Unknown`) | Expected fail-closed behaviour; the rejection is recorded in the ledger. Fix the pipeline logic |
| `repair stagnated …` / `iteration cap … reached` | The compile-critic loop couldn't converge | Escalates to human review (HITLC); inspect the recorded `CompileFailure` lessons |
| `sandbox trap: out of fuel` | A node exceeded its instruction budget | Raise `ExecLimits.fuel` for that workload, or fix a runaway node |
| `AOT artifact rejected` | Cached artifact built by an incompatible engine version | Harmless — it is discarded and recompiled automatically; or clear `<cache>/aot/` |

## Offline verification (CI / pre-release)

Not on the request path. Run before releasing changes to invariants, templates,
or the ledger:

```sh
cd verification/kani && cargo kani --default-unwind 1     # expect: 4 verified, 0 failures
cd verification/tla  && apalache-mc check --config=ledger.cfg --length=6 ledger.tla   # expect: NoError
```

Both also run in CI via `.github/workflows/verify.yml`.
