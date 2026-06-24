//! Meta-schema validation gate (U8, KTD4).
//!
//! Every proposed mutation is validated against a static, closed-world JSON
//! Schema before it can be appended to the ledger. This is deliberately *flat
//! record validation* — typed structs + `jsonschema` — not SHACL/RDF ontology
//! checking (deferred; see Scope Boundaries). It runs in-process and sub-ms, and
//! composes after type checks and the Z3 invariant engine (U7) in the hot-path
//! verification gate.

use jsonschema::Validator;
use serde_json::Value;

use aether_sdk::{AetherError, Result};

const MUTATION_SCHEMA: &str = include_str!("../schemas/mutation.schema.json");
const STAGE: &str = "meta_schema";

/// A compiled meta-schema validator. Compile once, validate many.
pub struct MetaSchema {
    validator: Validator,
}

impl MetaSchema {
    /// Compile the embedded mutation meta-schema.
    pub fn new() -> Result<Self> {
        let schema: Value = serde_json::from_str(MUTATION_SCHEMA)?;
        let validator =
            jsonschema::validator_for(&schema).map_err(|e| AetherError::VerificationRejected {
                stage: "meta_schema_compile".into(),
                reason: e.to_string(),
            })?;
        Ok(MetaSchema { validator })
    }

    /// Validate a candidate mutation document. On failure, returns a
    /// fail-closed [`AetherError::VerificationRejected`] naming every violation.
    pub fn validate_mutation(&self, mutation: &Value) -> Result<()> {
        if self.validator.is_valid(mutation) {
            return Ok(());
        }
        let reason = self
            .validator
            .iter_errors(mutation)
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        Err(AetherError::VerificationRejected {
            stage: STAGE.into(),
            reason,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> MetaSchema {
        MetaSchema::new().expect("embedded schema must compile")
    }

    #[test]
    fn accepts_a_well_formed_mutation() {
        let m = json!({"subject": "partner:42", "predicate": "owes_usd_cents", "object": "12345"});
        assert!(schema().validate_mutation(&m).is_ok());
    }

    #[test]
    fn rejects_missing_field() {
        let m = json!({"subject": "partner:42", "predicate": "owes_usd_cents"});
        assert!(matches!(
            schema().validate_mutation(&m),
            Err(AetherError::VerificationRejected { .. })
        ));
    }

    #[test]
    fn rejects_unknown_field_closed_world() {
        let m = json!({
            "subject": "s", "predicate": "p", "object": "o", "injected": "evil"
        });
        assert!(schema().validate_mutation(&m).is_err());
    }

    #[test]
    fn rejects_empty_string_and_wrong_type() {
        let empty = json!({"subject": "", "predicate": "p", "object": "o"});
        assert!(schema().validate_mutation(&empty).is_err());
        let wrong_type = json!({"subject": "s", "predicate": "p", "object": 5});
        assert!(schema().validate_mutation(&wrong_type).is_err());
    }
}
