# Synthesis (IPSE — Intent Parse & Synthesis Engine)

The `aether-compiler` crate is the subsystem that turns a declarative intent into
compiled, sandboxed WASM modules. It contains the LLM client, intent parsing,
t-DAG validation, three AI agents (SAA, CA, CRA), a Reflexion-shaped repair
loop, a locked rustc→wasm driver, and the pipeline node library.

Source: `crates/compiler/src/`.

## Module map

| Module | File | Responsibility |
|--------|------|----------------|
| `llm` | `llm.rs` | Claude Messages API HTTP client, `Transport` trait, retry, strict tool-use parsing |
| `intent_parse` | `intent_parse.rs` | Intent JSON parsing + invariant reference validation |
| `tdag` | `tdag.rs` | `ExecutionGraph`: acyclicity, unique IDs, topological sort |
| `nodes` | `nodes.rs` | Per-kind template hints + least-privilege capability derivation |
| `agents` | `agents/mod.rs`, `agents/saa.rs`, `agents/ca.rs`, `agents/cra.rs` | SAA, CA, CRA agents + `IpseAgents` composition |
| `synth` | `synth/mod.rs`, `synth/diagnostics.rs`, `synth/repair.rs` | Repair loop, structured diagnostics, convergence guards |
| `rustc_driver` | `rustc_driver.rs` | rustc → `wasm32-wasip2` compilation driver |
| `build_sandbox` | `build_sandbox.rs` | Seccomp + rlimit locked build sandbox (Linux-only, fail-closed) |

## LLM client (`llm.rs`)

A thin internal Anthropic Messages client. Rust has no official Anthropic SDK, so
this calls `POST /v1/messages` over `reqwest` (KTD6).

### Transport abstraction

```rust
#[async_trait]
pub trait Transport: Send + Sync {
    async fn post_messages(&self, body: &Value) -> std::result::Result<Value, TransportError>;
}
```

The `Transport` trait isolates the network so all request-building, tool-use
parsing, refusal handling, and retry are testable without a socket. Tests use a
`MockTransport` that returns scripted responses.

### Request/response vocabulary

- `Model`: `Opus48` (`"claude-opus-4-8"`) for high-judgment SAA/CRA;
  `Haiku45` (`"claude-haiku-4-5"`) for cheap mechanical CA passes.
- `Effort`: `Low` / `Medium` / `High` / `Xhigh` / `Max` — maps to
  `output_config.effort`.
- `CompletionRequest`: model, max_tokens, system, messages, tools, force_tool,
  effort, thinking_adaptive, stream.
- `Completion`: model, stop_reason, text, tool_uses. `is_refusal()` checks
  `stop_reason == "refusal"` before reading content (no panic on empty).

### LlmClient

```rust
pub struct LlmClient<T: Transport> { transport: T, retry: RetryConfig }
```

- `build_request(&self, req)` — pure, deterministic JSON body builder. Keeps
  prompt prefix byte-stable for cache hits.
- `complete(&self, req)` — send with retry (exponential backoff on 429/5xx),
  then parse_response.
- `complete_tool<O: DeserializeOwned>(&self, req, tool_name)` — force a strict
  tool, deserialize its input into `O`. Errors on refusal/missing/schema
  mismatch.

`ReqwestTransport::from_env()` reads `ANTHROPIC_API_KEY` from env — no hardcoded
fallback. Absent key produces a clear startup error.

## Intent parsing (`intent_parse.rs`)

```rust
pub fn parse_intent(json: &str) -> Result<Intent>
pub fn validate_invariant_refs(intent: &Intent, known: &HashSet<String>) -> Result<()>
pub fn default_known_invariants() -> HashSet<String>
```

`parse_intent` deserializes JSON → `Intent`, then calls `intent.validate()`.
`validate_invariant_refs` enforces **KTD3**: every invariant reference in the
intent must resolve to one the engine actually defines. The LLM cannot
introduce invariants — an intent may only reference invariants the FVL registry
already defines.

## t-DAG validation (`tdag.rs`)

```rust
pub struct ExecutionGraph {
    graph: DiGraph<NodeId, EdgeKind>,
    order: Vec<NodeId>,
}

impl ExecutionGraph {
    pub fn build(tdag: &TDag) -> Result<Self>
    pub fn topo_order(&self) -> &[NodeId]
}
```

`build` performs construction-time validation:
1. Duplicate node IDs → `AetherError::IntentInvalid`
2. Dangling edge references → `AetherError::IntentInvalid`
3. Cycle detection via `petgraph::algo::toposort` → `AetherError::IntentInvalid`
4. Topological sort — deterministic (insertion-order tie-break)

An `ExecutionGraph` only ever exists for a valid, acyclic, topologically-
orderable t-DAG.

## Node library (`nodes.rs`)

```rust
pub fn template_hint(kind: NodeKind) -> &'static str
pub fn capability_for(node: &TDagNode) -> Capability
```

Each node kind gets:
1. A **template hint** appended to the CA's prompt, steering it to a bounded
   code shape (e.g. "integer-cents currency conversion, never f64").
2. A **least-privilege capability** derived from kind + spec, deny-by-default.

See [Domain concepts](domain-concepts.md) for the per-kind capability table.

## AI agents (`agents/`)

### System Architect Agent (SAA) — `agents/saa.rs`

Decomposes an intent into a t-DAG via Claude. Designs **structure only** — never
writes code, never authors invariants. Uses **Opus 4.8** at **High** effort with
adaptive thinking.

```rust
pub async fn plan(&self, intent: &Intent, known_invariants: &[String], ledger_summary: &str) -> Result<TDag>
```

Flow: build `CompletionRequest` with the SAA system prompt
(`agents/prompts/saa_system.txt`), strict tool `emit_tdag` (closed-world JSON
schema), `client.complete_tool::<TDag>(req, "emit_tdag")` deserializes the tool
output directly into a `TDag`, then `ExecutionGraph::build()` validates it.

### Compiler Agent (CA) — `agents/ca.rs`

Synthesizes Rust source + WIT for a single t-DAG node. Uses **Haiku 4.5** by
default (cheap mechanical generation) at **Medium** effort.

```rust
pub async fn generate(&self, node_spec: &str, lessons: &[String]) -> Result<GeneratedNode>
```

`lessons` from prior CRA diagnoses are prepended to the prompt so the agent
avoids repeating mistakes. Strict tool `emit_node` with schema
`{rust_source: string, wit: string}`.

The CA system prompt (`agents/prompts/ca_system.txt`) enforces: target
`wasm32-wasip2`, panic-free Rust, money as `i64` cents (never f64), only stdlib
+ vendored allowlisted crates, zero ambient authority, no `unwrap()`/`expect()`.

### Critic-Refiner Agent (CRA) — `agents/cra.rs`

Turns structured rustc diagnostics into a minimal localized patch + a one-line
lesson (Reflexion self-reflection). Uses **Opus 4.8** at **High** effort —
repair is the high-judgment step.

```rust
pub async fn repair(&self, prev_source: &str, diagnostics: &[Diagnostic]) -> Result<Repair>
```

Returns `Repair { rust_source: String, lesson: String }`. The prompt includes
the current source in a rust code block and structured diagnostics formatted as
`[level] code: message`. The lesson is a single sentence naming the root cause
and preventing rule.

### Agent prompts

All prompts are `include_str!`'d as byte-stable constants for prompt-cache hits:
- `agents/prompts/saa_system.txt` — t-DAG decomposition rules
- `agents/prompts/ca_system.txt` — code generation constraints
- `agents/prompts/cra_system.txt` — repair constraints

## Repair loop (`synth/`)

### Structured diagnostics (`synth/diagnostics.rs`)

```rust
pub fn parse_rustc_diagnostics(stderr: &str) -> Vec<Diagnostic>
pub fn error_count(diags: &[Diagnostic]) -> usize
pub fn diagnostic_signature(diags: &[Diagnostic]) -> Vec<String>  // sorted, for stagnation detection
```

Parses rustc `--error-format=json` output (one JSON object per line).

### RepairLoop (`synth/repair.rs`)

```rust
pub struct RepairConfig { pub max_iterations: u32 }  // default 4

pub async fn run(&self, node_spec: &str, agent: &dyn CodeAgent, compiler: &dyn NodeCompiler) -> Result<RepairOutcome>
```

Algorithm (KTD7):
1. `agent.generate(node_spec, &[])` → initial source
2. For `attempt` in `1..=max_iterations`:
   a. `compiler.compile(&source)` → `CompileOutcome`
   b. If `Success(wasm)` → return `RepairOutcome`
   c. If `Errors(diags)`:
      - **Stagnation check**: if signature identical to previous OR error count
        didn't shrink → `Err(CompileFailed("repair stagnated... escalate to HITLC"))`
      - **Iteration cap**: if `attempt == max_iterations` → `Err(CompileFailed("iteration cap reached..."))`
      - Otherwise: `agent.repair(&source, &diags)` → new source + lesson
      - Push `CorrectionRecord` and lesson for feedback to next CA attempt

Both `NodeCompiler` and `CodeAgent` are traits, so the convergence logic is
tested against scripted stubs (no real rustc, no live model).

## rustc driver (`rustc_driver.rs`)

```rust
pub struct RustcDriver { rustc: PathBuf, scratch: PathBuf, sandbox: SandboxConfig }

impl RustcDriver {
    pub fn discover(scratch: impl Into<PathBuf>) -> Result<Self>
    pub fn compile_to_wasm(&self, rust_source: &str) -> Result<CompileOutcome>
}
```

`compile_to_wasm` flow:
1. Write source to `scratch/node.rs`
2. Build rustc command: `--target wasm32-wasip2 --crate-type cdylib --error-format=json -o node.wasm node.rs`
3. `sandbox.harden(&mut cmd)` — clears env, installs seccomp + rlimits
4. Set `TMPDIR` to scratch (since env was cleared)
5. Execute; on success → `CompileOutcome::Success(bytes)`; on failure → `CompileOutcome::Errors(parse_rustc_diagnostics(stderr))`

The toolchain's rustc is resolved to an absolute path via `rustc --print sysroot`
so the sandboxed invocation needs no rustup env and still finds its
`wasm32-wasip2` std.

## Build sandbox (`build_sandbox.rs`)

```rust
pub struct SandboxConfig {
    pub cpu_seconds: u64,           // RLIMIT_CPU, default 60
    pub address_space_bytes: u64,   // RLIMIT_AS, default 4 GiB
    pub file_size_bytes: u64,       // RLIMIT_FSIZE, default 512 MiB
}
```

**Linux** (`cfg(target_os = "linux")`): `harden` performs:
1. `cmd.env_clear()` — no inherited environment (no secret leaks)
2. In the `pre_exec` closure (unsafe, async-signal-safe):
   - `prctl(PR_SET_NO_NEW_PRIVS, 1)`
   - `setrlimit` for CPU, address space, file size
   - `seccompiler::apply_filter` — seccomp BPF: default `Allow`, `SYS_socket`
     → `KillProcess` (denies all network egress)

**Non-Linux**: `harden` returns `Err(CompileFailed)`. AETHER will not run an
LLM-synthesized compile without the Linux seccomp/rlimit sandbox (fail-closed).

This is the mitigation for KTD8: `build.rs` scripts and proc-macros run
arbitrary native code at compile time, outside any WASM sandbox. The locked
build sandbox prevents network egress, limits resources, and strips secrets
from the environment.

## Tests

| File | What it tests |
|------|---------------|
| `tests/saa.rs` | SAA produces valid acyclic t-DAG from recorded fixture; rejects cycles; surfaces refusals as errors |
| `tests/repair_loop.rs` | Loop converges after one repair; aborts on stagnation; aborts at iteration cap. Uses `ScriptedCompiler` + `ScriptedAgent` stubs |
| `tests/rustc_driver.rs` | (Linux-only) Compiles valid node to wasm; returns structured diagnostics on bad source; sandbox denies network egress |
| `tests/pipeline_nodes.rs` | Each node kind gets least-privilege capability; template hints distinguish node kinds |

## Watch-outs for future changes

- **Invariants are authored only in `crates/verifier/src/invariants.rs`** — never
  by the LLM, never widened from intent. Do not add invariant logic to the
  compiler crate.
- **Prompts are `include_str!`'d** — changing a prompt file changes the
  byte-stable prefix and may affect prompt-cache hit rates.
- **The `Transport` trait is the testing seam** — any new agent should accept a
  generic `T: Transport` and be tested with `MockTransport`, not live API calls.
- **The build sandbox is Linux-only and fail-closed** — do not relax the
  non-Linux `Err` return to allow compilation without seccomp.
- **Repair loop convergence guards are load-bearing** — the stagnation check
  (identical diagnostic signature) and iteration cap prevent non-converging
  loops from burning tokens. Do not remove them.
