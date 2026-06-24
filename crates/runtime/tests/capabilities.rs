//! U5 verification: capability injection is deny-by-default and grants back
//! exactly what a node declares.
//!
//! The socket allowlist (the security-critical predicate) is exercised directly;
//! filesystem and clock grants are verified by successful, faithful `WasiCtx`
//! construction. Live in-guest enforcement (a running wasip2 component reading a
//! preopen / opening a socket) is exercised at the U13 orchestration layer,
//! which depends on the rustc→wasip2 driver (U12).

use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use aether_runtime::{build_wasi_ctx, NetGuard, WasiHost};
use aether_sdk::types::{Capability, ClockPolicy, NetRule, PreopenedDir};

fn addr(s: &str) -> SocketAddr {
    s.parse().unwrap()
}

#[test]
fn zero_authority_by_default() {
    // A default capability grants nothing, yet a valid ctx still builds.
    let cap = Capability::none();
    assert!(!cap.grants_any_authority());
    assert!(build_wasi_ctx(&cap).is_ok());
    assert!(WasiHost::from_capability(&cap).is_ok());
}

#[test]
fn socket_allowlist_permits_only_declared_addresses() {
    let cap = Capability {
        net_allowlist: vec![NetRule {
            host: "10.0.0.7".into(),
            port: Some(8443),
        }],
        ..Capability::none()
    };
    let guard = NetGuard::from_rules(&cap.net_allowlist);

    assert!(
        guard.is_allowed(&addr("10.0.0.7:8443")),
        "declared address allowed"
    );
    assert!(!guard.is_allowed(&addr("10.0.0.7:22")), "other port denied");
    assert!(
        !guard.is_allowed(&addr("8.8.8.8:8443")),
        "other host denied"
    );

    // The ctx wiring this guard still builds.
    assert!(build_wasi_ctx(&cap).is_ok());
}

#[test]
fn declared_preopened_dir_is_granted() {
    // Create a real host directory and grant read-only access to the guest.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("aether-cap-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();

    let cap = Capability {
        preopened_dirs: vec![PreopenedDir {
            host_path: dir.to_string_lossy().into_owned(),
            guest_path: "/data".into(),
            writable: false,
        }],
        clock: ClockPolicy::Denied,
        ..Capability::none()
    };
    assert!(cap.grants_any_authority());
    assert!(
        build_wasi_ctx(&cap).is_ok(),
        "granting an existing dir must succeed"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn nonexistent_preopen_is_denied() {
    let cap = Capability {
        preopened_dirs: vec![PreopenedDir {
            host_path: "/no/such/aether/path/xyz".into(),
            guest_path: "/data".into(),
            writable: false,
        }],
        ..Capability::none()
    };
    assert!(
        build_wasi_ctx(&cap).is_err(),
        "a missing host dir must fail closed"
    );
}
