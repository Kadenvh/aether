//! Human-In-The-Loop Consensus — terminal dry-run (U13, R12, R15).
//!
//! When a run is low-confidence or hits a soft (non-fatal) constraint, the
//! engine does not silently proceed: it serializes the exact pending state to a
//! deterministic JSON dry-run and surfaces it to the operator on the terminal
//! (no GUI — R12). The operator's signed decision is then written to the USL.
//! V1 ships the deterministic serialization; the interactive capture is a thin
//! shell over it.

use serde_json::{json, Value};

/// A pending decision serialized for operator review.
#[derive(Debug, Clone)]
pub struct DryRun {
    pub reason: String,
    pub proposed: Value,
}

impl DryRun {
    pub fn new(reason: impl Into<String>, proposed: Value) -> Self {
        DryRun {
            reason: reason.into(),
            proposed,
        }
    }

    /// The deterministic JSON dry-run document shown to the operator.
    pub fn document(&self) -> Value {
        json!({
            "kind": "hitlc_dry_run",
            "reason": self.reason,
            "proposed": self.proposed,
            "decision_required": ["approve", "reject"],
        })
    }

    /// A terminal-friendly rendering of the dry-run.
    pub fn render(&self) -> String {
        format!(
            "── AETHER human review required ──\nreason: {}\nproposed:\n{}\n(approve/reject)",
            self.reason,
            serde_json::to_string_pretty(&self.proposed).unwrap_or_default()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_run_document_is_deterministic_and_complete() {
        let dr = DryRun::new("variance over 20%", json!({"usd_amount_cents": 12345}));
        let doc = dr.document();
        assert_eq!(doc["kind"], "hitlc_dry_run");
        assert_eq!(doc["reason"], "variance over 20%");
        assert_eq!(doc["proposed"]["usd_amount_cents"], 12345);
        assert!(dr.render().contains("human review"));
    }
}
