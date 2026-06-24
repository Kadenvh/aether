//! U11 verification: the compile-critic loop converges, caps, and detects
//! stagnation. Driven by scripted stubs — no real rustc, no live model (per the
//! unit's execution note).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use async_trait::async_trait;

use aether_compiler::synth::repair::{CodeAgent, CompileOutcome, NodeCompiler, Repair};
use aether_compiler::synth::{Diagnostic, RepairConfig, RepairLoop};
use aether_sdk::Result;

/// A scripted compile result (cloneable so the stub can replay it).
#[derive(Clone)]
enum Step {
    Ok(Vec<u8>),
    Err(Vec<Diagnostic>),
}

/// Returns each scripted step in turn, clamping to the last once exhausted.
struct ScriptedCompiler {
    steps: Vec<Step>,
    idx: Mutex<usize>,
}

impl ScriptedCompiler {
    fn new(steps: Vec<Step>) -> Self {
        ScriptedCompiler {
            steps,
            idx: Mutex::new(0),
        }
    }
}

#[async_trait]
impl NodeCompiler for ScriptedCompiler {
    async fn compile(&self, _rust_source: &str) -> Result<CompileOutcome> {
        let mut idx = self.idx.lock().unwrap();
        let i = (*idx).min(self.steps.len() - 1);
        *idx += 1;
        Ok(match self.steps[i].clone() {
            Step::Ok(wasm) => CompileOutcome::Success(wasm),
            Step::Err(diags) => CompileOutcome::Errors(diags),
        })
    }
}

/// Generates a fixed initial source; each repair returns a fresh source + lesson
/// (the compiler is scripted independently, so the source content is irrelevant).
struct ScriptedAgent {
    repairs: AtomicU32,
}

impl ScriptedAgent {
    fn new() -> Self {
        ScriptedAgent {
            repairs: AtomicU32::new(0),
        }
    }
}

#[async_trait]
impl CodeAgent for ScriptedAgent {
    async fn generate(&self, _node_spec: &str, _lessons: &[String]) -> Result<String> {
        Ok("fn run() {}".to_string())
    }
    async fn repair(&self, _prev: &str, _diags: &[Diagnostic]) -> Result<Repair> {
        let n = self.repairs.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(Repair {
            rust_source: format!("// attempt {n}\nfn run() {{}}"),
            lesson: format!("lesson {n}"),
        })
    }
}

/// `n` distinct error diagnostics (distinct signatures across different `n`).
fn errors(n: usize) -> Vec<Diagnostic> {
    (0..n)
        .map(|i| Diagnostic {
            level: "error".into(),
            message: format!("error number {i}"),
            code: Some(format!("E{i:04}")),
        })
        .collect()
}

#[tokio::test]
async fn converges_after_one_repair() {
    let compiler =
        ScriptedCompiler::new(vec![Step::Err(errors(2)), Step::Ok(b"wasm-bytes".to_vec())]);
    let agent = ScriptedAgent::new();
    let loop_ = RepairLoop::new(RepairConfig { max_iterations: 4 });

    let outcome = loop_
        .run("spec", &agent, &compiler)
        .await
        .expect("should converge");
    assert_eq!(outcome.attempts, 2);
    assert_eq!(outcome.wasm, b"wasm-bytes");
    assert_eq!(outcome.lessons, vec!["lesson 1".to_string()]);
    assert_eq!(outcome.corrections.len(), 1);
    assert_eq!(outcome.corrections[0].attempt, 1);
}

#[tokio::test]
async fn aborts_on_stagnation() {
    // Compiler always returns the identical diagnostic set -> no progress.
    let compiler = ScriptedCompiler::new(vec![Step::Err(errors(2))]);
    let agent = ScriptedAgent::new();
    let loop_ = RepairLoop::new(RepairConfig { max_iterations: 5 });

    let err = loop_
        .run("spec", &agent, &compiler)
        .await
        .expect_err("must abort");
    assert!(
        err.to_string().contains("stagnat"),
        "expected stagnation, got: {err}"
    );
}

#[tokio::test]
async fn aborts_at_iteration_cap() {
    // Strictly shrinking but never zero -> not stagnation; must hit the cap.
    let compiler = ScriptedCompiler::new(vec![
        Step::Err(errors(5)),
        Step::Err(errors(4)),
        Step::Err(errors(3)),
    ]);
    let agent = ScriptedAgent::new();
    let loop_ = RepairLoop::new(RepairConfig { max_iterations: 3 });

    let err = loop_
        .run("spec", &agent, &compiler)
        .await
        .expect_err("must hit cap");
    assert!(
        err.to_string().contains("iteration cap"),
        "expected cap, got: {err}"
    );
}
