//! rustc → `wasm32-wasip2` driver in the locked sandbox (U12, KTD7/KTD8).
//!
//! Compiles a single-file synthesized guest node to WebAssembly with
//! `--error-format=json`, running rustc inside the [`SandboxConfig`] hardening
//! (no network, no inherited env, resource caps). Returns the `.wasm` bytes on
//! success, or the parsed structured diagnostics for the U11 repair loop — so a
//! [`RustcDriver`] is a drop-in [`NodeCompiler`].
//!
//! The pinned toolchain's `rustc` is resolved to an absolute path up front (via
//! `rustc --print sysroot`), so the sandboxed invocation needs no rustup env and
//! still finds its `wasm32-wasip2` std. Multi-dependency nodes compiled via
//! `cargo build --offline --locked` against vendored crates are a documented
//! extension on top of this single-file path.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use async_trait::async_trait;

use aether_sdk::{AetherError, Result};

use crate::build_sandbox::SandboxConfig;
use crate::synth::parse_rustc_diagnostics;
use crate::synth::repair::{CompileOutcome, NodeCompiler};

const TARGET: &str = "wasm32-wasip2";

/// Compiles single-file guest nodes to `wasm32-wasip2` under the locked sandbox.
pub struct RustcDriver {
    rustc: PathBuf,
    scratch: PathBuf,
    sandbox: SandboxConfig,
}

impl RustcDriver {
    /// Build a driver from an explicit `rustc` path and scratch directory.
    pub fn new(rustc: impl Into<PathBuf>, scratch: impl Into<PathBuf>) -> Self {
        RustcDriver {
            rustc: rustc.into(),
            scratch: scratch.into(),
            sandbox: SandboxConfig::default(),
        }
    }

    /// Resolve the active toolchain's `rustc` to an absolute binary path and
    /// build a driver using `scratch` for sources/artifacts.
    pub fn discover(scratch: impl Into<PathBuf>) -> Result<Self> {
        let out = Command::new("rustc")
            .arg("--print")
            .arg("sysroot")
            .output()
            .map_err(|e| AetherError::CompileFailed(format!("locating rustc: {e}")))?;
        if !out.status.success() {
            return Err(AetherError::CompileFailed(
                "`rustc --print sysroot` failed".into(),
            ));
        }
        let sysroot = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let rustc = PathBuf::from(sysroot).join("bin").join("rustc");
        Ok(RustcDriver::new(rustc, scratch))
    }

    /// Compile `rust_source` to a wasm cdylib, returning bytes or diagnostics.
    pub fn compile_to_wasm(&self, rust_source: &str) -> Result<CompileOutcome> {
        fs::create_dir_all(&self.scratch).map_err(io)?;
        let src = self.scratch.join("node.rs");
        let out = self.scratch.join("node.wasm");
        fs::write(&src, rust_source).map_err(io)?;
        let _ = fs::remove_file(&out);

        let mut cmd = Command::new(&self.rustc);
        cmd.arg("--target")
            .arg(TARGET)
            .arg("--crate-type")
            .arg("cdylib")
            .arg("--error-format=json")
            .arg("-o")
            .arg(&out)
            .arg(&src)
            .current_dir(&self.scratch);
        // `harden` clears the environment; set TMPDIR afterwards so rustc's
        // intermediates land in the scratch dir.
        self.sandbox.harden(&mut cmd)?;
        cmd.env("TMPDIR", &self.scratch);

        let output = cmd
            .output()
            .map_err(|e| AetherError::CompileFailed(format!("spawning sandboxed rustc: {e}")))?;

        if output.status.success() && out.exists() {
            Ok(CompileOutcome::Success(fs::read(&out).map_err(io)?))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Ok(CompileOutcome::Errors(parse_rustc_diagnostics(&stderr)))
        }
    }
}

#[async_trait]
impl NodeCompiler for RustcDriver {
    async fn compile(&self, rust_source: &str) -> Result<CompileOutcome> {
        // The compile is a blocking subprocess; fine to run inline for V1.
        self.compile_to_wasm(rust_source)
    }
}

fn io(e: std::io::Error) -> AetherError {
    AetherError::Io(e.to_string())
}
