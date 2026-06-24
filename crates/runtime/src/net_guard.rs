//! Outbound network allowlist at the socket layer (U5, ADSI; deny-by-default).
//!
//! A guest may open an outbound socket only to an address explicitly granted by
//! its verified t-DAG node. This guard matches a *resolved* `SocketAddr`
//! (IP + port) against the node's [`NetRule`] allowlist. Hostname-only rules
//! cannot be enforced here — that needs DNS and belongs at the `wasi-http`
//! layer (deferred) — so they are counted but never silently treated as a pass.
//!
//! An empty allowlist denies everything.

use std::net::{IpAddr, SocketAddr};

use aether_sdk::types::NetRule;

/// A cheap, cloneable socket-layer allowlist derived from a node's capability.
#[derive(Clone, Default)]
pub struct NetGuard {
    /// (ip, optional port) entries. `None` port allows any port on that IP.
    allowed: Vec<(IpAddr, Option<u16>)>,
    /// Count of hostname rules not enforceable at the socket layer.
    host_rules: usize,
}

impl NetGuard {
    /// Build a guard from a node's net allowlist. Rules whose host parses as an
    /// IP literal are socket-enforceable; hostname rules are counted for the
    /// (deferred) wasi-http layer.
    pub fn from_rules(rules: &[NetRule]) -> Self {
        let mut allowed = Vec::new();
        let mut host_rules = 0;
        for rule in rules {
            match rule.host.parse::<IpAddr>() {
                Ok(ip) => allowed.push((ip, rule.port)),
                Err(_) => host_rules += 1,
            }
        }
        NetGuard {
            allowed,
            host_rules,
        }
    }

    /// Whether an outbound connection to `addr` is permitted. Deny-by-default.
    pub fn is_allowed(&self, addr: &SocketAddr) -> bool {
        self.allowed
            .iter()
            .any(|(ip, port)| *ip == addr.ip() && port.is_none_or(|p| p == addr.port()))
    }

    /// Whether any socket-enforceable rule exists (used to gate `allow_tcp`).
    pub fn has_socket_rules(&self) -> bool {
        !self.allowed.is_empty()
    }

    /// Number of hostname rules that the socket layer cannot enforce.
    pub fn unenforceable_host_rule_count(&self) -> usize {
        self.host_rules
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(host: &str, port: Option<u16>) -> NetRule {
        NetRule {
            host: host.into(),
            port,
        }
    }

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn empty_allowlist_denies_everything() {
        let guard = NetGuard::from_rules(&[]);
        assert!(!guard.is_allowed(&addr("127.0.0.1:443")));
        assert!(!guard.has_socket_rules());
    }

    #[test]
    fn matches_ip_and_port_exactly() {
        let guard = NetGuard::from_rules(&[rule("10.0.0.5", Some(8080))]);
        assert!(guard.is_allowed(&addr("10.0.0.5:8080")));
        assert!(!guard.is_allowed(&addr("10.0.0.5:9090")));
        assert!(!guard.is_allowed(&addr("10.0.0.6:8080")));
    }

    #[test]
    fn none_port_allows_any_port_on_that_ip() {
        let guard = NetGuard::from_rules(&[rule("192.168.1.1", None)]);
        assert!(guard.is_allowed(&addr("192.168.1.1:80")));
        assert!(guard.is_allowed(&addr("192.168.1.1:65000")));
        assert!(!guard.is_allowed(&addr("192.168.1.2:80")));
    }

    #[test]
    fn hostname_rules_are_counted_not_socket_enforced() {
        let guard = NetGuard::from_rules(&[rule("api.partner.com", Some(443))]);
        assert_eq!(guard.unenforceable_host_rule_count(), 1);
        assert!(!guard.has_socket_rules());
    }
}
