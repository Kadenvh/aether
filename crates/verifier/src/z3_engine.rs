//! Z3-backed invariant proof engine (U7, KTD3, KTD4; fail-closed).
//!
//! To prove a proposed mutation cannot violate an invariant we encode the
//! *negation* and ask Z3 whether it is satisfiable: assume a valid pre-state
//! (every invariant holds), apply the mutation's delta, then assert that *some*
//! post-invariant fails. If that is `Unsat`, no valid pre-state can be driven
//! into a violation — the mutation is proven safe. `Sat` yields a
//! counterexample (reject); `Unknown` (timeout/incompleteness) also rejects —
//! verification is **fail-closed**.
//!
//! Constraints stay in the decidable QF_LIA fragment (linear integer
//! arithmetic over cents), so `check()` terminates.

use std::collections::BTreeMap;

use z3::ast::{Bool, Int};
use z3::{SatResult, Solver};

use aether_sdk::{AetherError, Result};

use crate::invariants::{Invariant, MutationDelta};

const STAGE: &str = "z3_invariant";

/// The fail-closed SMT invariant engine. One engine per worker (Z3 contexts are
/// not `Send`); cheap to construct.
#[derive(Default)]
pub struct Z3InvariantEngine;

impl Z3InvariantEngine {
    pub fn new() -> Self {
        Z3InvariantEngine
    }

    /// Prove that applying `deltas` to *any* invariant-satisfying pre-state
    /// preserves every invariant. Returns `Ok(())` only when Z3 proves it
    /// (`Unsat` for the negation); rejects on a counterexample or `Unknown`.
    pub fn prove_preserved(
        &self,
        invariants: &[Invariant],
        deltas: &[MutationDelta],
    ) -> Result<()> {
        if invariants.is_empty() {
            return Ok(());
        }
        let solver = Solver::new();

        // One symbolic pre-state constant per variable that appears.
        let mut pre: BTreeMap<&str, Int> = BTreeMap::new();
        for inv in invariants {
            pre.entry(inv.var())
                .or_insert_with(|| Int::new_const(inv.var()));
        }
        for d in deltas {
            pre.entry(d.var.as_str())
                .or_insert_with(|| Int::new_const(d.var.as_str()));
        }

        // Assume the pre-state is valid: every invariant held before the change.
        for inv in invariants {
            match inv {
                Invariant::NonNegative { var } => {
                    solver.assert(pre[var.as_str()].ge(Int::from_i64(0)));
                }
            }
        }

        // Post-state: pre + delta for changed vars, unchanged otherwise.
        // (`Int::add` is n-ary in z3 0.20 — an associated fn over a slice.)
        let delta_map: BTreeMap<&str, i64> =
            deltas.iter().map(|d| (d.var.as_str(), d.delta)).collect();
        let post: BTreeMap<&str, Int> = pre
            .iter()
            .map(|(name, pre_val)| {
                let post_val = match delta_map.get(name) {
                    Some(&d) => Int::add(&[pre_val.clone(), Int::from_i64(d)]),
                    None => pre_val.clone(),
                };
                (*name, post_val)
            })
            .collect();

        // Negation: assert that at least one post-invariant fails.
        let violations: Vec<Bool> = invariants
            .iter()
            .map(|inv| match inv {
                Invariant::NonNegative { var } => post[var.as_str()].ge(Int::from_i64(0)).not(),
            })
            .collect();
        if !violations.is_empty() {
            solver.assert(Bool::or(&violations));
        }

        match solver.check() {
            SatResult::Unsat => Ok(()),
            SatResult::Sat => Err(AetherError::VerificationRejected {
                stage: STAGE.into(),
                reason: "a valid pre-state exists in which this mutation violates an invariant"
                    .into(),
            }),
            SatResult::Unknown => Err(AetherError::VerificationRejected {
                stage: STAGE.into(),
                reason: "Z3 returned Unknown (timeout/incomplete); rejecting fail-closed".into(),
            }),
        }
    }
}
