//! U7 verification: the Z3 engine proves safe mutations and fail-closed-rejects
//! unsafe ones. Exercises the real vendored Z3 (built in CI).

use aether_sdk::AetherError;
use aether_verifier::{Invariant, MutationDelta, Z3InvariantEngine};

fn engine() -> Z3InvariantEngine {
    Z3InvariantEngine::new()
}

#[test]
fn deposit_preserves_non_negativity() {
    // balance >= 0, deposit +500 cents: provably cannot make balance negative.
    let invs = [Invariant::non_negative("balance")];
    let deltas = [MutationDelta::new("balance", 500)];
    assert!(engine().prove_preserved(&invs, &deltas).is_ok());
}

#[test]
fn unguarded_withdrawal_is_rejected() {
    // balance >= 0, withdraw 500: a pre-state of 0..499 underflows -> counterexample.
    let invs = [Invariant::non_negative("balance")];
    let deltas = [MutationDelta::new("balance", -500)];
    match engine().prove_preserved(&invs, &deltas) {
        Err(AetherError::VerificationRejected { stage, .. }) => assert_eq!(stage, "z3_invariant"),
        other => panic!("expected rejection, got {other:?}"),
    }
}

#[test]
fn no_op_mutation_preserves_invariants() {
    let invs = [
        Invariant::non_negative("balance"),
        Invariant::non_negative("usd_amount"),
    ];
    assert!(engine().prove_preserved(&invs, &[]).is_ok());
}

#[test]
fn unrelated_delta_does_not_affect_invariant() {
    // Depositing into usd_amount cannot violate balance>=0, and the usd_amount
    // deposit is itself safe.
    let invs = [
        Invariant::non_negative("balance"),
        Invariant::non_negative("usd_amount"),
    ];
    let deltas = [MutationDelta::new("usd_amount", 1000)];
    assert!(engine().prove_preserved(&invs, &deltas).is_ok());
}

#[test]
fn one_unsafe_delta_among_many_rejects() {
    // A safe balance deposit plus an unguarded usd_amount withdrawal -> reject.
    let invs = [
        Invariant::non_negative("balance"),
        Invariant::non_negative("usd_amount"),
    ];
    let deltas = [
        MutationDelta::new("balance", 100),
        MutationDelta::new("usd_amount", -100),
    ];
    assert!(engine().prove_preserved(&invs, &deltas).is_err());
}
