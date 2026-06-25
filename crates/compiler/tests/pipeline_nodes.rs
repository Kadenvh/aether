//! U15 verification: the pipeline node library derives least-privilege
//! capabilities and kind-specific synthesis hints through the public API.

use aether_compiler::nodes::{capability_for, template_hint};
use aether_sdk::types::{NodeKind, TDagNode};
use serde_json::json;

fn node(kind: NodeKind, spec: serde_json::Value) -> TDagNode {
    TDagNode {
        id: "node".into(),
        kind,
        spec,
    }
}

#[test]
fn each_node_kind_is_scoped_to_least_privilege() {
    // Ingest: read-only filesystem, nothing else.
    let ingest = capability_for(&node(NodeKind::Ingest, json!({"input_path": "/data/in"})));
    assert_eq!(ingest.preopened_dirs.len(), 1);
    assert!(!ingest.preopened_dirs[0].writable);
    assert!(ingest.net_allowlist.is_empty());

    // Transform: zero ambient authority.
    assert!(!capability_for(&node(NodeKind::Transform, json!({}))).grants_any_authority());

    // ApiSync: exactly one outbound endpoint, no filesystem.
    let sync = capability_for(&node(
        NodeKind::ApiSync,
        json!({"endpoint_host": "192.0.2.10", "port": 8443}),
    ));
    assert_eq!(sync.net_allowlist.len(), 1);
    assert_eq!(sync.net_allowlist[0].port, Some(8443));
    assert!(sync.preopened_dirs.is_empty());
}

#[test]
fn hints_distinguish_node_kinds() {
    assert!(template_hint(NodeKind::Ingest).contains("INGEST"));
    assert!(template_hint(NodeKind::Transform).contains("TRANSFORM"));
    assert!(template_hint(NodeKind::ApiSync).contains("API_SYNC"));
}
