//! Intent parsing and boundary validation (R1).
//!
//! Parses an `intent.json` into [`Intent`] and resolves its invariant
//! references against a *known* set. Critically, invariants are never authored
//! by the LLM (KTD3) — an intent may only *reference* invariants the FVL
//! registry already defines; an unknown reference is rejected here.

use std::collections::HashSet;

use aether_sdk::{AetherError, Intent, Result};

/// Parse and structurally validate an `intent.json` document.
pub fn parse_intent(json: &str) -> Result<Intent> {
    let intent: Intent = serde_json::from_str(json)?;
    intent.validate()?;
    Ok(intent)
}

/// Verify every invariant the intent references is one the engine actually
/// defines. `known` is supplied by the FVL invariant registry (U7); until that
/// crate is wired in, [`default_known_invariants`] stands in.
pub fn validate_invariant_refs(intent: &Intent, known: &HashSet<String>) -> Result<()> {
    for reference in &intent.invariants {
        if !known.contains(reference) {
            return Err(AetherError::UnknownInvariant(reference.clone()));
        }
    }
    Ok(())
}

/// The hardcoded invariant references recognized in V1. Mirrors the blueprint
/// example; the authoritative set will live in the `verifier` crate (U7).
pub fn default_known_invariants() -> HashSet<String> {
    [
        "usd_amount >= 0.0",
        "partner_id must match known_partners in local state",
        "balance >= 0",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLUEPRINT_INTENT: &str = r#"{
        "objective": "Import partner utility bills, convert Euros to USD, flag lines with anomalous variance > 20% compared to historical average, and save to ledger",
        "invariants": [
            "usd_amount >= 0.0",
            "partner_id must match known_partners in local state"
        ]
    }"#;

    #[test]
    fn parses_blueprint_intent_and_resolves_known_invariants() {
        // Covers AE1.
        let intent = parse_intent(BLUEPRINT_INTENT).unwrap();
        assert!(validate_invariant_refs(&intent, &default_known_invariants()).is_ok());
    }

    #[test]
    fn rejects_unknown_invariant_reference() {
        // KTD3: the LLM cannot introduce invariants — an unrecognized ref fails.
        let intent = Intent {
            objective: "do a thing".into(),
            invariants: vec!["drop table customers".into()],
            input: None,
            output: None,
        };
        assert!(matches!(
            validate_invariant_refs(&intent, &default_known_invariants()),
            Err(AetherError::UnknownInvariant(_))
        ));
    }

    #[test]
    fn rejects_malformed_json() {
        assert!(matches!(parse_intent("{ not json"), Err(AetherError::Serde(_))));
    }

    #[test]
    fn rejects_empty_objective_at_boundary() {
        let json = r#"{"objective": "", "invariants": []}"#;
        assert!(matches!(parse_intent(json), Err(AetherError::IntentInvalid(_))));
    }
}
