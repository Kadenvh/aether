//! Pipeline node library (U15): the V1 node vocabulary beyond the EDID example.
//!
//! Each node kind contributes two things to the synthesis pipeline:
//! - a **template hint** appended to the Compiler Agent's prompt, steering it to
//!   the bounded code shape for that kind (the same shapes Kani proves in U14);
//! - a **least-privilege capability** ([`capability_for`]) the orchestrator
//!   injects into the TAR sandbox (U5) — deny-by-default, widened only by what
//!   the node's verified spec declares.
//!
//! Every node kind flows through the *same* synthesize → verify → sandbox →
//! ledger path; nothing here special-cases the trust boundary.

use aether_sdk::types::{Capability, ClockPolicy, NetRule, NodeKind, PreopenedDir, TDagNode};

/// Synthesis guidance for a node kind, appended to the CA prompt.
pub fn template_hint(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Ingest => ingest::HINT,
        NodeKind::Transform => transform::HINT,
        NodeKind::Flag => transform::FLAG_HINT,
        NodeKind::Persist => transform::PERSIST_HINT,
        NodeKind::ApiSync => api_sync::HINT,
    }
}

/// Derive the least-privilege [`Capability`] a node needs from its kind + spec.
/// Anything not explicitly required stays denied (KTD: zero ambient authority).
pub fn capability_for(node: &TDagNode) -> Capability {
    match node.kind {
        NodeKind::Ingest => ingest::capability(&node.spec),
        NodeKind::ApiSync => api_sync::capability(&node.spec),
        // Pure-compute and ledger-bound kinds touch nothing: the host persists,
        // the guest only computes.
        NodeKind::Transform | NodeKind::Flag | NodeKind::Persist => Capability::none(),
    }
}

/// Read an optional string field from a node spec object.
fn spec_str<'a>(spec: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    spec.get(key).and_then(serde_json::Value::as_str)
}

/// Batch ingestion: read/parse an input source (CSV/JSON/structured records).
pub mod ingest {
    use super::*;

    pub const HINT: &str = "This is an INGEST node: read and parse the input records from the \
        single read-only preopened directory mounted at `/input`. Support CSV and JSON. Validate \
        every row at the boundary; reject malformed input. Emit the parsed record count as i32.";

    /// Read-only preopen of the declared input directory; no network, no clock.
    pub fn capability(spec: &serde_json::Value) -> Capability {
        let host_path = spec_str(spec, "input_path").unwrap_or(".").to_string();
        Capability {
            preopened_dirs: vec![PreopenedDir {
                host_path,
                guest_path: "/input".into(),
                writable: false,
            }],
            net_allowlist: vec![],
            clock: ClockPolicy::Denied,
            fuel_budget: 0,
        }
    }
}

/// Pure data transformation: schema change, unit/currency conversion, derive,
/// type-coerce. Also covers Flag (annotate) and Persist (compute-then-host-writes).
pub mod transform {
    pub const HINT: &str = "This is a TRANSFORM node: a pure function over the input records — \
        column rename/derive/type-coerce, or integer-cents currency conversion (never f64). No \
        I/O: zero ambient authority. Emit the transformed net result in cents as i32.";

    pub const FLAG_HINT: &str = "This is a FLAG node: detect/annotate anomalous records (e.g. \
        variance over a threshold) using checked integer arithmetic. Pure, no I/O. Emit the count \
        of flagged records as i32.";

    pub const PERSIST_HINT: &str = "This is a PERSIST node: compute the final verified value to \
        write; the host appends it to the ledger (the guest performs no I/O). Emit the value in \
        cents as i32.";
}

/// Outbound API sync routed through the U5 net allowlist (domain-pinned).
pub mod api_sync {
    use super::*;

    pub const HINT: &str = "This is an API_SYNC node: synchronize results to the single approved \
        outbound endpoint. You may connect ONLY to the host:port granted by the capability — no \
        other address. Handle transport errors explicitly. Emit the count of synced records as i32.";

    /// A single outbound rule for the declared endpoint; nothing else granted.
    pub fn capability(spec: &serde_json::Value) -> Capability {
        let host = spec_str(spec, "endpoint_host").unwrap_or("").to_string();
        let port = spec
            .get("port")
            .and_then(serde_json::Value::as_u64)
            .map(|p| p as u16);
        let net_allowlist = if host.is_empty() {
            vec![]
        } else {
            vec![NetRule { host, port }]
        };
        Capability {
            preopened_dirs: vec![],
            net_allowlist,
            clock: ClockPolicy::Denied,
            fuel_budget: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(kind: NodeKind, spec: serde_json::Value) -> TDagNode {
        TDagNode {
            id: "n".into(),
            kind,
            spec,
        }
    }

    #[test]
    fn ingest_gets_readonly_preopen_only() {
        let cap = capability_for(&node(NodeKind::Ingest, json!({"input_path": "/srv/bills"})));
        assert_eq!(cap.preopened_dirs.len(), 1);
        assert_eq!(cap.preopened_dirs[0].host_path, "/srv/bills");
        assert!(!cap.preopened_dirs[0].writable, "ingest must be read-only");
        assert!(cap.net_allowlist.is_empty(), "ingest gets no network");
    }

    #[test]
    fn transform_and_flag_have_zero_authority() {
        for kind in [NodeKind::Transform, NodeKind::Flag, NodeKind::Persist] {
            let cap = capability_for(&node(kind, json!({})));
            assert!(!cap.grants_any_authority(), "{kind:?} must grant nothing");
        }
    }

    #[test]
    fn api_sync_gets_only_the_declared_endpoint() {
        let cap = capability_for(&node(
            NodeKind::ApiSync,
            json!({"endpoint_host": "10.0.0.9", "port": 443}),
        ));
        assert_eq!(cap.net_allowlist.len(), 1);
        assert_eq!(cap.net_allowlist[0].host, "10.0.0.9");
        assert_eq!(cap.net_allowlist[0].port, Some(443));
        assert!(cap.preopened_dirs.is_empty(), "api_sync gets no filesystem");
    }

    #[test]
    fn api_sync_without_endpoint_grants_nothing() {
        let cap = capability_for(&node(NodeKind::ApiSync, json!({})));
        assert!(
            !cap.grants_any_authority(),
            "no endpoint => deny-by-default"
        );
    }

    #[test]
    fn every_kind_has_a_template_hint() {
        for kind in [
            NodeKind::Ingest,
            NodeKind::Transform,
            NodeKind::Flag,
            NodeKind::Persist,
            NodeKind::ApiSync,
        ] {
            assert!(!template_hint(kind).is_empty());
        }
    }
}
