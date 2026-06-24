//! WASI capability injection (U5, ADSI; KTD: zero ambient authority).
//!
//! A synthesized guest is a `wasm32-wasip2` component with **zero** ambient
//! authority. [`build_wasi_ctx`] starts from an empty [`WasiCtxBuilder`] — no
//! inherited stdio, env, args, or network — and grants back *only* what the
//! node's verified [`Capability`] declares: specific preopened directories, an
//! outbound-socket allowlist (via [`NetGuard`]), and a clock policy.
//!
//! [`WasiHost`] is the per-store host state; it implements [`WasiView`] so the
//! wasmtime-wasi host functions can reach the ctx + resource table when U13
//! instantiates a component against this engine.

use std::net::SocketAddr;
use std::pin::Pin;
use std::time::Duration;

use wasmtime_wasi::sockets::SocketAddrUse;
use wasmtime_wasi::{
    DirPerms, FilePerms, HostMonotonicClock, HostWallClock, ResourceTable, WasiCtx, WasiCtxBuilder,
    WasiCtxView, WasiView,
};

use aether_sdk::types::{Capability, ClockPolicy};
use aether_sdk::{AetherError, Result};

use crate::net_guard::NetGuard;

/// Per-store WASI host state for a component guest. Holds only the authority the
/// verified node declared.
pub struct WasiHost {
    ctx: WasiCtx,
    table: ResourceTable,
}

impl WasiHost {
    /// Build host state granting exactly what `cap` declares.
    pub fn from_capability(cap: &Capability) -> Result<Self> {
        Ok(WasiHost {
            ctx: build_wasi_ctx(cap)?,
            table: ResourceTable::new(),
        })
    }
}

impl WasiView for WasiHost {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

/// Build a [`WasiCtx`] granting only the authority in `cap`. Deny-by-default:
/// nothing is inherited from the host process.
pub fn build_wasi_ctx(cap: &Capability) -> Result<WasiCtx> {
    let mut builder = WasiCtxBuilder::new();

    // Filesystem: only the explicitly declared preopened directories, with the
    // narrowest permissions the node asked for.
    for dir in &cap.preopened_dirs {
        let (dir_perms, file_perms) = if dir.writable {
            (
                DirPerms::READ | DirPerms::MUTATE,
                FilePerms::READ | FilePerms::WRITE,
            )
        } else {
            (DirPerms::READ, FilePerms::READ)
        };
        builder
            .preopened_dir(&dir.host_path, &dir.guest_path, dir_perms, file_perms)
            .map_err(|e| {
                AetherError::CapabilityDenied(format!("preopen '{}' failed: {e}", dir.host_path))
            })?;
    }

    // Network: deny by default. TCP is enabled only if a socket-enforceable rule
    // exists; UDP and DNS name lookup stay off. Every outbound address is
    // checked against the allowlist.
    let guard = NetGuard::from_rules(&cap.net_allowlist);
    builder.allow_tcp(guard.has_socket_rules());
    builder.allow_udp(false);
    builder.allow_ip_name_lookup(false);
    builder.socket_addr_check(move |addr: SocketAddr, _use: SocketAddrUse| {
        let allowed = guard.is_allowed(&addr);
        Box::pin(async move { allowed })
            as Pin<Box<dyn std::future::Future<Output = bool> + Send + Sync>>
    });

    // Clock: withhold real time unless explicitly granted. Denied and Fixed both
    // inject a deterministic fixed clock so a guest cannot observe wall-clock
    // time; Wall grants the host's real clock.
    match cap.clock {
        ClockPolicy::Denied | ClockPolicy::Fixed => {
            builder.wall_clock(FixedWallClock);
            builder.monotonic_clock(FixedMonotonicClock);
        }
        ClockPolicy::Wall => {}
    }

    Ok(builder.build())
}

/// A wall clock pinned to the Unix epoch — reveals no real time to the guest.
struct FixedWallClock;

impl HostWallClock for FixedWallClock {
    fn resolution(&self) -> Duration {
        Duration::from_secs(1)
    }
    fn now(&self) -> Duration {
        Duration::ZERO
    }
}

/// A monotonic clock pinned at zero.
struct FixedMonotonicClock;

impl HostMonotonicClock for FixedMonotonicClock {
    fn resolution(&self) -> u64 {
        1
    }
    fn now(&self) -> u64 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_sdk::types::NetRule;

    #[test]
    fn zero_authority_capability_builds() {
        // Capability::none() grants nothing; the ctx must still build cleanly.
        assert!(build_wasi_ctx(&Capability::none()).is_ok());
    }

    #[test]
    fn capability_with_net_rule_builds() {
        let cap = Capability {
            net_allowlist: vec![NetRule {
                host: "10.0.0.1".into(),
                port: Some(443),
            }],
            clock: ClockPolicy::Denied,
            ..Capability::none()
        };
        assert!(build_wasi_ctx(&cap).is_ok());
        assert!(WasiHost::from_capability(&cap).is_ok());
    }

    #[test]
    fn wall_clock_policy_builds() {
        let cap = Capability {
            clock: ClockPolicy::Wall,
            ..Capability::none()
        };
        assert!(build_wasi_ctx(&cap).is_ok());
    }
}
