//! U8 verification: the meta-schema gate accepts well-formed mutations and
//! fail-closed-rejects malformed ones, through the public API.

use serde_json::json;

use aether_sdk::AetherError;
use aether_verifier::MetaSchema;

#[test]
fn gate_admits_valid_and_rejects_invalid() {
    let gate = MetaSchema::new().expect("schema compiles");

    // Admit a well-formed triple.
    let ok = json!({"subject": "partner:7", "predicate": "balance_cents", "object": "0"});
    assert!(gate.validate_mutation(&ok).is_ok());

    // Reject a structurally invalid one, naming the gate stage.
    let bad = json!({"subject": "partner:7", "object": "0"});
    match gate.validate_mutation(&bad) {
        Err(AetherError::VerificationRejected { stage, .. }) => assert_eq!(stage, "meta_schema"),
        other => panic!("expected VerificationRejected, got {other:?}"),
    }
}
