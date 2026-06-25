# Contributing to AETHER

AETHER (Autonomic Ephemeral System Engine) is a Rust workspace that synthesizes,
formally verifies, sandboxes, and runs ephemeral data pipelines, writing verified
state to an immutable ledger. This guide covers local development.

> Design context lives in [`docs/plans/2026-06-24-001-feat-aether-engine-v1-plan.md`](plans/2026-06-24-001-feat-aether-engine-v1-plan.md).

## Prerequisites

<!-- AUTO-GENERATED: derived from rust-toolchain.toml, crate manifests, and the U12 sandbox / U14 offline tier. -->

| Requirement | Why | Notes |
|-------------|-----|-------|
| **Linux (or WSL2)** | The build sandbox (U12) uses seccomp + rlimits; it is Linux-only and fails closed elsewhere | Kernel ≥ 5.13 for Landlock/seccomp; WSL2 Ubuntu works |
| **Rust 1.96** + `wasm32-wasip2` target | Pinned by `rust-toolchain.toml`; synthesized nodes compile to `wasm32-wasip2` | `rustup target add wasm32-wasip2` |
| **cmake + a C++ toolchain** | Builds vendored Z3 for the invariant engine (U7) | `apt install cmake build-essential` |
| **JDK 21 + Apalache + Kani** | Offline verification tier only (U14) | Not needed for `cargo build`/`test` |
| **`ANTHROPIC_API_KEY`** | Live synthesis (SAA/CA/CRA call Claude) | Only for live `aether run`/`watch`; tests don't need it |

The repo builds from `/mnt/c/AETHER` under WSL. Invoke cargo via:
`wsl -e bash -lc '. "$HOME/.cargo/env"; cd /mnt/c/AETHER && cargo …'`.

## Workspace layout

<!-- AUTO-GENERATED: from the workspace [members] in Cargo.toml. -->

| Crate | Subsystem | Responsibility |
|-------|-----------|----------------|
| `crates/sdk` | core | Shared types: `Intent`, t-DAG, `Cents`, `Capability`, `LedgerEvent`, the `Ledger` trait |
| `crates/compiler` | IPSE | Claude client, intent parse, t-DAG, SAA/CA/CRA agents, repair loop, locked rustc→wasm driver, node library |
| `crates/runtime` | TAR | Wasmtime sandbox, WASI capability injection, blueprint cache + AOT |
| `crates/verifier` | FVL | Z3 invariant engine, meta-schema gate |
| `crates/ledger` | USL | Append-only bi-temporal hash-chained CozoDB store |
| `cli` | — | `aether run` / `aether watch` binary + orchestrator |
| `verification/kani` | offline | Standalone Kani proof crate (own workspace; run via `cargo kani`) |

## Commands

<!-- AUTO-GENERATED: from Cargo workspace + verification tooling. Run under WSL. -->

| Command | Description |
|---------|-------------|
| `cargo build --workspace` | Build every crate |
| `cargo test --workspace` | Run the full test suite |
| `cargo test -p <crate>` | Test one crate (e.g. `aether-runtime`) |
| `cargo fmt --all` | Format (rustfmt; required before commit) |
| `cargo clippy --all-targets` | Lint (treat warnings as errors before merge) |
| `cargo run -p aether-cli --bin aether -- run …` | Run the CLI locally (see [RUNBOOK](RUNBOOK.md)) |
| `cd verification/kani && cargo kani --default-unwind 1` | Offline: 4 proof harnesses over synthesized-code templates |
| `cd verification/tla && apalache-mc check --config=ledger.cfg --length=6 ledger.tla` | Offline: ledger state-machine model check |

## Testing

- Unit tests live in `#[cfg(test)]` modules; integration tests in each crate's `tests/`.
- Network-free by design: the e2e tests (`cli/tests/e2e_edid.rs`, `watch_daemon.rs`) seed the blueprint cache so no LLM is called and no socket opens.
- The U12 sandbox test asserts a `socket()` call is killed by `SIGSYS` — it runs on Linux only (`#![cfg(target_os = "linux")]`).
- New behaviour needs a test: happy path, edge/boundary, and the failure path (especially for anything touching the FVL gate or the ledger).

## Code style

- **rustfmt** (100-col) and **clippy** clean are required.
- Money is **integer `Cents`**, never `f64`.
- **Invariants are authored only in `verifier/src/invariants.rs`** — never by the LLM, never widened from intent.
- **Capabilities are deny-by-default** — grant only what a node's verified spec declares.
- Verification is **fail-closed** — on `Unknown`/timeout/ambiguity, reject.
- Every `unsafe` block carries a `// SAFETY:` comment.

## PR checklist

- [ ] `cargo test --workspace` green
- [ ] `cargo fmt --all` applied; `cargo clippy --all-targets` clean
- [ ] Offline tier green if you touched invariants, templates, or the ledger (`cargo kani`, `apalache-mc check`)
- [ ] New behaviour has tests (incl. the failure path)
- [ ] Conventional commit messages (`feat(scope): …`, `fix:`, `docs:`, …)
- [ ] No secrets committed; `ANTHROPIC_API_KEY` only ever read from env
