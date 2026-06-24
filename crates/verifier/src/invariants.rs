//! Hardcoded safety invariants (U7, KTD3, KTD4).
//!
//! Invariants are authored **only** here in static Rust — never by the LLM. The
//! synthesis agents may propose *mutations*; the Z3 engine ([`crate::z3_engine`])
//! proves a proposed mutation cannot violate any of these. V1 covers the
//! arithmetic non-negativity class (money is integer cents, never `f64` — KTD4):
//! "balance >= 0", "usd_amount >= 0". Set-membership invariants (e.g. "partner_id
//! must be a known partner") are a different, non-arithmetic class enforced by
//! the registry/meta-schema, not the SMT engine.

/// A hardcoded invariant over the integer state. `var` names a quantity tracked
/// in cents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invariant {
    /// The named quantity must never be negative in the post-state.
    NonNegative { var: String },
}

impl Invariant {
    /// The state variable this invariant constrains.
    pub fn var(&self) -> &str {
        match self {
            Invariant::NonNegative { var } => var,
        }
    }

    /// Convenience constructor.
    pub fn non_negative(var: impl Into<String>) -> Self {
        Invariant::NonNegative { var: var.into() }
    }
}

/// A proposed change to one state variable, in cents (may be negative).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationDelta {
    pub var: String,
    pub delta: i64,
}

impl MutationDelta {
    pub fn new(var: impl Into<String>, delta: i64) -> Self {
        MutationDelta {
            var: var.into(),
            delta,
        }
    }
}

/// The V1 hardcoded invariant set. Mirrors the blueprint's numeric invariants.
pub fn default_invariants() -> Vec<Invariant> {
    vec![
        Invariant::non_negative("balance"),
        Invariant::non_negative("usd_amount"),
    ]
}
