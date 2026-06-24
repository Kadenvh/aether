//! The locked build sandbox (U12, KTD8).
//!
//! `build.rs`/proc-macros and even a benign-looking compile are an RCE surface,
//! so the rustc invocation runs with hardened limits applied to the child
//! process just before `exec`:
//! - **no network egress** — a seccomp filter kills the process if it calls
//!   `socket(2)`, so a synthesized node can never open an outbound connection;
//! - **no inherited environment** — the env is cleared (no secrets/tokens leak in);
//! - **resource ceilings** — CPU-time, address-space, and output-file rlimits
//!   bound a runaway or fork-bombing compile.
//!
//! This is Linux-only (seccomp + rlimits). On other platforms [`SandboxConfig::harden`]
//! refuses the compile fail-closed rather than running it unsandboxed.

use std::process::Command;

use aether_sdk::{AetherError, Result};

/// Resource ceilings for a sandboxed compile.
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// CPU-time limit in seconds (RLIMIT_CPU).
    pub cpu_seconds: u64,
    /// Virtual address-space cap in bytes (RLIMIT_AS).
    pub address_space_bytes: u64,
    /// Maximum output file size in bytes (RLIMIT_FSIZE).
    pub file_size_bytes: u64,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        SandboxConfig {
            cpu_seconds: 60,
            address_space_bytes: 4 * 1024 * 1024 * 1024,
            file_size_bytes: 512 * 1024 * 1024,
        }
    }
}

#[cfg(target_os = "linux")]
impl SandboxConfig {
    /// Clear the environment and install the seccomp + rlimit hardening that
    /// fires in the forked child immediately before `exec`.
    pub fn harden(&self, cmd: &mut Command) -> Result<()> {
        use std::os::unix::process::CommandExt;

        cmd.env_clear();

        // Build the seccomp BPF in the parent — no allocation is permitted in
        // the post-fork, pre-exec child.
        let program = build_no_network_filter()?;
        let cpu = self.cpu_seconds;
        let address_space = self.address_space_bytes;
        let file_size = self.file_size_bytes;

        // SAFETY: this closure runs in the forked child before `exec`. It performs
        // only async-signal-safe syscalls (prctl, setrlimit) plus installing a
        // pre-built, already-allocated seccomp program. It allocates nothing and
        // returns `Err` (aborting the exec, fail-closed) on any failure.
        unsafe {
            cmd.pre_exec(move || {
                // seccomp install requires NO_NEW_PRIVS (or CAP_SYS_ADMIN).
                if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                set_rlimit(libc::RLIMIT_CPU, cpu)?;
                set_rlimit(libc::RLIMIT_AS, address_space)?;
                set_rlimit(libc::RLIMIT_FSIZE, file_size)?;
                seccompiler::apply_filter(&program)
                    .map_err(|_| std::io::Error::from_raw_os_error(libc::EPERM))?;
                Ok(())
            });
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn set_rlimit(resource: libc::__rlimit_resource_t, value: u64) -> std::io::Result<()> {
    let limit = libc::rlimit {
        rlim_cur: value,
        rlim_max: value,
    };
    // SAFETY: `limit` is fully initialized; `setrlimit` is async-signal-safe.
    if unsafe { libc::setrlimit(resource, &limit) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// A seccomp program that allows everything except `socket(2)`, which kills the
/// process — denying any outbound network egress from the compile.
#[cfg(target_os = "linux")]
fn build_no_network_filter() -> Result<seccompiler::BpfProgram> {
    use std::collections::BTreeMap;

    use seccompiler::{SeccompAction, SeccompFilter, TargetArch};

    let rules = BTreeMap::from([(libc::SYS_socket, Vec::new())]);
    let filter = SeccompFilter::new(
        rules,
        SeccompAction::Allow,       // default: allow
        SeccompAction::KillProcess, // socket(): no egress, ever
        TargetArch::x86_64,
    )
    .map_err(|e| AetherError::CompileFailed(format!("seccomp filter build failed: {e}")))?;
    filter
        .try_into()
        .map_err(|e| AetherError::CompileFailed(format!("seccomp filter compile failed: {e}")))
}

#[cfg(not(target_os = "linux"))]
impl SandboxConfig {
    /// Fail-closed: AETHER will not run an LLM-synthesized compile without the
    /// Linux seccomp/rlimit sandbox.
    pub fn harden(&self, _cmd: &mut Command) -> Result<()> {
        Err(AetherError::CompileFailed(
            "locked build sandbox requires Linux (seccomp + rlimits); refusing to compile \
             unsandboxed"
                .into(),
        ))
    }
}
