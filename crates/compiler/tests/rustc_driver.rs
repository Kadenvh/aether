//! U12 verification: the driver compiles a node to `wasm32-wasip2`, surfaces
//! structured diagnostics on bad input, and the sandbox denies network egress.
//!
//! Linux-only — the locked sandbox needs seccomp + rlimits. On other platforms
//! this test binary is empty (skip-with-warning per the unit's gating).
#![cfg(target_os = "linux")]

use std::os::unix::process::ExitStatusExt;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use aether_compiler::build_sandbox::SandboxConfig;
use aether_compiler::synth::repair::CompileOutcome;
use aether_compiler::RustcDriver;

fn scratch(tag: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!("aether-rustc-{tag}-{nanos}"));
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn compiles_valid_node_to_wasm() {
    let dir = scratch("ok");
    let driver = RustcDriver::discover(&dir).expect("discover rustc");
    let source = "#[no_mangle]\npub extern \"C\" fn run() -> i32 { 2 + 2 }\n";

    match driver.compile_to_wasm(source).expect("compile runs") {
        CompileOutcome::Success(wasm) => {
            assert!(wasm.len() > 8, "expected real wasm output");
            assert_eq!(
                &wasm[0..4],
                b"\0asm",
                "output must be a wasm module/component"
            );
        }
        CompileOutcome::Errors(diags) => panic!("expected success, got diagnostics: {diags:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn returns_structured_diagnostics_on_bad_source() {
    let dir = scratch("bad");
    let driver = RustcDriver::discover(&dir).expect("discover rustc");
    // Type error: assign a &str to a u32.
    let source = "#[no_mangle]\npub extern \"C\" fn run() -> i32 { let x: u32 = \"no\"; 0 }\n";

    match driver.compile_to_wasm(source).expect("compile runs") {
        CompileOutcome::Errors(diags) => {
            assert!(!diags.is_empty(), "expected at least one diagnostic");
            assert!(
                diags.iter().any(|d| d.level == "error"),
                "expected an error-level diagnostic"
            );
        }
        CompileOutcome::Success(_) => panic!("expected diagnostics for bad source"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sandbox_denies_network_egress() {
    // A trivial non-network program runs fine under the sandbox.
    let mut ok = Command::new("true");
    SandboxConfig::default().harden(&mut ok).unwrap();
    assert!(
        ok.status().unwrap().success(),
        "non-network command should run under the sandbox"
    );

    // Any attempt to create a socket is killed by SIGSYS (31).
    let mut net = Command::new("bash");
    net.arg("-c").arg("exec 3<>/dev/tcp/127.0.0.1/9");
    SandboxConfig::default().harden(&mut net).unwrap();
    let status = net.status().unwrap();
    assert_eq!(
        status.signal(),
        Some(31),
        "a socket() call must be killed by SIGSYS; got {status:?}"
    );
}
