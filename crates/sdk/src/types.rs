//! Core domain types shared across AETHER subsystems.

use serde::{Deserialize, Serialize};

use crate::error::{AetherError, Result};
use crate::Timestamp;

// ---------------------------------------------------------------------------
// Money — integer cents only. Never use floating point for monetary values
// (KTD4): IEEE-754 makes invariant proofs (U7) far harder and admits surprising
// counterexamples.
// ---------------------------------------------------------------------------

/// A monetary amount in integer cents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Cents(pub i64);

impl Cents {
    pub const ZERO: Cents = Cents(0);

    pub fn is_non_negative(self) -> bool {
        self.0 >= 0
    }

    pub fn checked_add(self, rhs: Cents) -> Result<Cents> {
        self.0
            .checked_add(rhs.0)
            .map(Cents)
            .ok_or_else(|| AetherError::Ledger("cents addition overflow".into()))
    }

    pub fn checked_sub(self, rhs: Cents) -> Result<Cents> {
        self.0
            .checked_sub(rhs.0)
            .map(Cents)
            .ok_or_else(|| AetherError::Ledger("cents subtraction underflow".into()))
    }
}

// ---------------------------------------------------------------------------
// Capability — zero ambient authority by default (KTD: deny-by-default).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreopenedDir {
    pub host_path: String,
    pub guest_path: String,
    #[serde(default)]
    pub writable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetRule {
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockPolicy {
    #[default]
    Denied,
    Fixed,
    Wall,
}

/// The authority granted to a single sandboxed t-DAG node. A default-constructed
/// `Capability` grants nothing — the runtime (U5) injects only what a verified
/// node declares.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Capability {
    #[serde(default)]
    pub preopened_dirs: Vec<PreopenedDir>,
    #[serde(default)]
    pub net_allowlist: Vec<NetRule>,
    #[serde(default)]
    pub clock: ClockPolicy,
    #[serde(default)]
    pub fuel_budget: u64,
}

impl Capability {
    /// A capability granting zero ambient authority.
    pub fn none() -> Self {
        Self::default()
    }

    /// True if this capability grants any filesystem, network, or clock access.
    pub fn grants_any_authority(&self) -> bool {
        !self.preopened_dirs.is_empty()
            || !self.net_allowlist.is_empty()
            || self.clock != ClockPolicy::Denied
    }
}

// ---------------------------------------------------------------------------
// Intent — the declarative input at the system boundary.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IoFormat {
    Csv,
    Json,
    Rdf,
    Sqlite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IoDescriptor {
    pub uri: String,
    pub format: IoFormat,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Intent {
    pub objective: String,
    /// References to hardcoded safety invariants (by id/expression). Resolved
    /// against the FVL invariant registry during planning (U9) — never authored
    /// by the LLM (KTD3).
    #[serde(default)]
    pub invariants: Vec<String>,
    #[serde(default)]
    pub input: Option<IoDescriptor>,
    #[serde(default)]
    pub output: Option<IoDescriptor>,
}

impl Intent {
    /// Structural validation at the system boundary. Does not resolve invariant
    /// references — that requires the FVL registry (see U9).
    pub fn validate(&self) -> Result<()> {
        if self.objective.trim().is_empty() {
            return Err(AetherError::IntentInvalid(
                "objective must not be empty".into(),
            ));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// t-DAG — the synthesized temporal execution graph (data only). The petgraph
// construction + construction-time acyclicity enforcement lives in the compiler
// crate (U9, KTD9).
// ---------------------------------------------------------------------------

pub type NodeId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Ingest,
    Transform,
    Flag,
    Persist,
    ApiSync,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TDagNode {
    pub id: NodeId,
    pub kind: NodeKind,
    #[serde(default)]
    pub spec: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    DataFlow,
    Temporal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TDagEdge {
    pub from: NodeId,
    pub to: NodeId,
    pub kind: EdgeKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TDag {
    pub nodes: Vec<TDagNode>,
    pub edges: Vec<TDagEdge>,
}

// ---------------------------------------------------------------------------
// Mutation + LedgerEvent — proposed state changes and the immutable, hash-
// chained event the ledger appends (R9, R10, KTD5).
// ---------------------------------------------------------------------------

/// A proposed RDF-like triple assertion (subject-predicate-object).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mutation {
    pub subject: String,
    pub predicate: String,
    pub object: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Assert,
    Retract,
    /// Correction-log: a synthesis compile that failed (R10).
    CompileFailure,
    /// Correction-log: the FVL rejected a mutation (R10).
    VerificationRejection,
    /// Correction-log: a signed human-in-the-loop decision (R10, R15).
    HumanIntervention,
}

/// An immutable, hash-chained ledger event. `curr_hash` is derived from every
/// other field plus `prev_hash`, giving tamper-evidence across the chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEvent {
    pub id: String,
    pub kind: EventKind,
    pub payload: serde_json::Value,
    pub tx_time: Timestamp,
    pub valid_from: Timestamp,
    #[serde(default)]
    pub valid_to: Option<Timestamp>,
    pub prev_hash: String,
    #[serde(default)]
    pub curr_hash: String,
}

impl LedgerEvent {
    /// Deterministic chain hash over every field except `curr_hash`.
    ///
    /// `serde_json` serializes struct fields in declaration order, so the
    /// canonical bytes are stable across runs and platforms for a given event.
    pub fn compute_hash(&self) -> String {
        #[derive(Serialize)]
        struct Canonical<'a> {
            id: &'a str,
            kind: &'a EventKind,
            payload: &'a serde_json::Value,
            tx_time: Timestamp,
            valid_from: Timestamp,
            valid_to: Option<Timestamp>,
            prev_hash: &'a str,
        }
        let canonical = Canonical {
            id: &self.id,
            kind: &self.kind,
            payload: &self.payload,
            tx_time: self.tx_time,
            valid_from: self.valid_from,
            valid_to: self.valid_to,
            prev_hash: &self.prev_hash,
        };
        let bytes = serde_json::to_vec(&canonical).expect("canonical event serialization");
        blake3::hash(&bytes).to_hex().to_string()
    }

    /// Return this event with `curr_hash` set to the computed chain hash.
    pub fn sealed(mut self) -> Self {
        self.curr_hash = self.compute_hash();
        self
    }

    /// Verify `curr_hash` matches the recomputed chain hash (tamper check).
    pub fn hash_is_valid(&self) -> bool {
        !self.curr_hash.is_empty() && self.compute_hash() == self.curr_hash
    }
}

// ---------------------------------------------------------------------------
// Ledger trait — repository pattern (KTD5). The concrete CozoDB impl (U3) lives
// in the `ledger` crate and is swappable behind this interface.
// ---------------------------------------------------------------------------

pub trait Ledger {
    /// Append an event. Implementations must chain `prev_hash`/`curr_hash` and
    /// never mutate or delete prior events.
    fn append_event(&mut self, event: LedgerEvent) -> Result<()>;

    /// The most recent event's `curr_hash`, or `None` for an empty ledger.
    fn latest_hash(&self) -> Option<String>;

    /// Reconstruct logical state visible at a transaction-time / valid-time point.
    fn query_as_of(&self, tx_time: Timestamp, valid_time: Timestamp) -> Result<Vec<LedgerEvent>>;

    /// Walk the hash chain and confirm tamper-evidence end to end.
    fn verify_chain(&self) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
    fn intent_round_trips_blueprint_example() {
        // Covers AE1: the intent.json from the blueprint parses and re-serializes.
        let intent: Intent = serde_json::from_str(BLUEPRINT_INTENT).unwrap();
        assert!(intent.objective.contains("Euros to USD"));
        assert_eq!(intent.invariants.len(), 2);
        assert!(intent.input.is_none());
        let reparsed: Intent =
            serde_json::from_str(&serde_json::to_string(&intent).unwrap()).unwrap();
        assert_eq!(intent, reparsed);
    }

    #[test]
    fn intent_validate_rejects_empty_objective() {
        let intent = Intent {
            objective: "   ".into(),
            invariants: vec![],
            input: None,
            output: None,
        };
        assert!(matches!(
            intent.validate(),
            Err(AetherError::IntentInvalid(_))
        ));
    }

    #[test]
    fn capability_default_grants_zero_authority() {
        let cap = Capability::none();
        assert!(cap.preopened_dirs.is_empty());
        assert!(cap.net_allowlist.is_empty());
        assert_eq!(cap.clock, ClockPolicy::Denied);
        assert!(!cap.grants_any_authority());
    }

    #[test]
    fn capability_missing_fields_default_to_zero_authority() {
        let cap: Capability = serde_json::from_str("{}").unwrap();
        assert!(!cap.grants_any_authority());
    }

    #[test]
    fn cents_arithmetic_errors_on_overflow() {
        assert!(Cents(i64::MAX).checked_add(Cents(1)).is_err());
        assert!(Cents(i64::MIN).checked_sub(Cents(1)).is_err());
        assert_eq!(Cents(100).checked_add(Cents(50)).unwrap(), Cents(150));
        assert!(!Cents(-1).is_non_negative());
        assert!(Cents::ZERO.is_non_negative());
    }

    #[test]
    fn ledger_event_hash_is_stable_and_detects_tampering() {
        let event = LedgerEvent {
            id: "evt-1".into(),
            kind: EventKind::Assert,
            payload: serde_json::json!({"b": 2, "a": 1}),
            tx_time: Timestamp(1000),
            valid_from: Timestamp(0),
            valid_to: None,
            prev_hash: "genesis".into(),
            curr_hash: String::new(),
        }
        .sealed();

        // Stable across recomputation.
        assert!(event.hash_is_valid());
        assert_eq!(event.compute_hash(), event.curr_hash);

        // Tamper the payload -> hash no longer matches.
        let mut tampered = event.clone();
        tampered.payload = serde_json::json!({"a": 1, "b": 999});
        assert!(!tampered.hash_is_valid());
    }

    #[test]
    fn ledger_event_chains_prev_hash() {
        let e1 = LedgerEvent {
            id: "e1".into(),
            kind: EventKind::Assert,
            payload: serde_json::json!({}),
            tx_time: Timestamp(1),
            valid_from: Timestamp(0),
            valid_to: None,
            prev_hash: "genesis".into(),
            curr_hash: String::new(),
        }
        .sealed();
        let e2 = LedgerEvent {
            id: "e2".into(),
            kind: EventKind::CompileFailure,
            payload: serde_json::json!({"error": "E0502"}),
            tx_time: Timestamp(2),
            valid_from: Timestamp(2),
            valid_to: None,
            prev_hash: e1.curr_hash.clone(),
            curr_hash: String::new(),
        }
        .sealed();
        assert_ne!(e1.curr_hash, e2.curr_hash);
        assert_eq!(e2.prev_hash, e1.curr_hash);
    }
}
