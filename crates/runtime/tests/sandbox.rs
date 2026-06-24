//! U4 verification: ephemeral execution is correct within budget and bounded
//! when a guest misbehaves. Uses inline `.wat` fixtures only — no rustc/LLM.

use std::time::Duration;

use aether_runtime::{ExecLimits, Sandbox};

/// A guest that immediately returns a constant — the happy path.
const RETURNS_42: &str = r#"
    (module
        (func (export "run") (result i32)
            i32.const 42))
"#;

/// A guest that spins forever — must be stopped by the fuel budget.
const INFINITE_LOOP: &str = r#"
    (module
        (func (export "run") (result i32)
            (loop $l (br $l))
            i32.const 0))
"#;

fn small_budget() -> ExecLimits {
    ExecLimits {
        fuel: 100_000,
        wall_timeout: Duration::from_secs(2),
        ..ExecLimits::default()
    }
}

#[tokio::test]
async fn runs_simple_function_within_budget() {
    let sandbox = Sandbox::new().expect("engine");
    let out = sandbox
        .run_i32(RETURNS_42.as_bytes(), "run", &small_budget())
        .await
        .expect("guest should succeed");
    assert_eq!(out, 42);
}

#[tokio::test]
async fn traps_on_fuel_exhaustion() {
    let sandbox = Sandbox::new().expect("engine");
    let err = sandbox
        .run_i32(INFINITE_LOOP.as_bytes(), "run", &small_budget())
        .await
        .expect_err("infinite loop must trap");
    let msg = err.to_string().to_lowercase();
    assert!(msg.contains("fuel"), "expected a fuel trap, got: {msg}");
}

#[tokio::test]
async fn errors_on_missing_export() {
    let sandbox = Sandbox::new().expect("engine");
    let err = sandbox
        .run_i32(RETURNS_42.as_bytes(), "nonexistent", &small_budget())
        .await
        .expect_err("missing export must error");
    assert!(err.to_string().contains("nonexistent"));
}

#[tokio::test]
async fn each_run_is_independent() {
    // The same warm engine serves multiple isolated transactions.
    let sandbox = Sandbox::new().expect("engine");
    for _ in 0..3 {
        let out = sandbox
            .run_i32(RETURNS_42.as_bytes(), "run", &small_budget())
            .await
            .expect("each run succeeds");
        assert_eq!(out, 42);
    }
}
