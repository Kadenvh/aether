# AETHER OpenWiki

AETHER (Autonomic Ephemeral System Engine) is a Rust workspace that **synthesizes,
formally verifies, sandboxes, and runs ephemeral data pipelines**. It takes a
declarative intent plus raw data, decomposes it into a verified temporal DAG
(t-DAG) of execution steps, JIT-compiles each step from LLM-generated Rust into
a gas-metered WASM sandbox, verifies safety invariants before execution, runs
the pipeline, and appends the verified state change to an immutable
bi-temporal ledger — then tears the sandbox down.

The key thesis: code is a disposable intermediate state. The asset is the
business intent and the historical state ledger, not the code. Synthesized code
is treated as untrusted and is contained at every stage: locked build sandbox,
deny-by-default capabilities, hardcoded invariants the LLM can never author,
and fail-closed verification gates.

## Workspace at a glance

| Crate | Subsystem | Role | Key source |
|-------|-----------|------|------------|
| `crates/sdk` | Core | Shared vocabulary: `Intent`, `TDag`, `Cents`, `Capability`, `LedgerEvent`, `Mutation`, `Ledger` trait, `AetherError` | `crates/sdk/src/types.rs` |
| `crates/compiler` | IPSE | Claude HTTP client, intent parsing, t-DAG validation, SAA/CA/CRA agents, repair loop, locked rustc→wasm driver, node library | `crates/compiler/src/` |
| `crates/runtime` | TAR | Wasmtime sandbox, fuel/epoch limits, WASI capability injection, blueprint cache, AOT precompile | `crates/runtime/src/` |
| `crates/verifier` | FVL | Z3 invariant engine (fail-closed), meta-schema validation gate | `crates/verifier/src/` |
| `crates/ledger` | USL | Append-only bi-temporal hash-chained CozoDB store | `crates/ledger/src/` |
| `cli` | — | `aether run` / `aether watch` binary, orchestrator wiring | `cli/src/` |
| `verification/kani` | Offline | Kani proof harnesses over synthesized-code templates | `verification/kani/src/lib.rs` |
| `verification/tla` | Offline | Apalache/TLC ledger state-machine model | `verification/tla/ledger.tla` |

## Documentation pages

- **[Architecture](architecture.md)** — System dataflow, two-tier verification, crate dependency graph, key design decisions.
- **[Domain concepts](domain-concepts.md)** — Shared vocabulary: intent, t-DAG, mutations, ledger events, capabilities, Cents.
- **[Synthesis (IPSE)](synthesis.md)** — Compiler crate: LLM client, agents, repair loop, rustc driver, build sandbox, node library.
- **[Runtime (TAR)](runtime.md)** — Wasmtime sandbox, WASI capabilities, blueprint cache, AOT.
- **[Verification (FVL)](verification.md)** — Z3 invariant engine, meta-schema gate, offline Kani/Apalache tier.
- **[Ledger (USL)](ledger.md)** — CozoDB store, hash chain, bi-temporal queries.
- **[Operations](operations.md)** — CLI usage, build/test commands, prerequisites, testing guidance, HITLC.

## Quick links to existing docs

- [Contributing guide](../docs/CONTRIBUTING.md) — prerequisites, workspace layout, code style, PR checklist.
- [Runbook](../docs/RUNBOOK.md) — CLI commands, artifacts, common issues, offline verification.
- [V1 implementation plan](../docs/plans/2026-06-24-001-feat-aether-engine-v1-plan.md) — full design rationale, requirements R1–R15, implementation units U1–U16, key technical decisions KTD1–KTD9.

## What AETHER proves (V1)

The V1 deliverable is the **EDID path** — `aether run --intent ./intent.json --input ./raw.csv --ledger ./state.db` — demonstrating that arbitrary tabular input can be ingested, transformed by JIT-synthesized + formally-gated WASM, and written to an immutable ledger, with safety invariants enforced by static, human-audited Rust — never by the LLM.

V1 breadth covers batch ingestion, schema transforms, and domain-pinned outbound API sync as intent-expressible pipeline node types. A `watch` daemon monitors a directory and runs the pipeline per new file, with steady-state runs hitting the blueprint cache (synthesis bypassed).
