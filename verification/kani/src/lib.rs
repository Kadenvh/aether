//! Kani proof harnesses over AETHER's synthesized-code *templates* (U14, R14).
//!
//! These prove properties of the bounded code shapes the Compiler Agent is
//! allowed to emit — money is integer cents (never `f64`), arithmetic is
//! overflow-checked, and the non-negativity invariant is preserved by the
//! deposit/convert templates. This is the **offline** tier: it gates templates
//! and the invariant set in CI, distinct from the runtime Z3 gate (U7) that
//! proves individual live mutations. Harnesses are loop-free, so a
//! `--default-unwind 1` is sufficient.
//!
//! Run with `cargo kani` (from this directory). A normal `cargo build` compiles
//! only the templates; the `#[cfg(kani)]` harnesses are Kani-only.

/// Deposit template: add a non-negative amount to a balance, overflow-checked.
/// Returns `None` on overflow rather than wrapping (the CA must handle it).
pub fn apply_deposit(balance_cents: i64, amount_cents: i64) -> Option<i64> {
    if amount_cents < 0 {
        return None; // a deposit is non-negative by construction
    }
    balance_cents.checked_add(amount_cents)
}

/// Currency-convert template: `minor * rate_ppm / 1_000_000`, in integer math
/// (parts-per-million rate). Overflow-checked; no floating point (KTD4).
pub fn checked_convert(minor_units: i64, rate_ppm: i64) -> Option<i64> {
    minor_units.checked_mul(rate_ppm).map(|scaled| scaled / 1_000_000)
}

/// Variance-flag template: true when |value - average| * 100 > average * pct,
/// computed in checked integer math. Returns `None` if the comparison would
/// overflow (caller treats that as "cannot evaluate" — fail-closed upstream).
pub fn flag_variance(value_cents: i64, average_cents: i64, pct: i64) -> Option<bool> {
    let diff = value_cents.checked_sub(average_cents)?.checked_abs()?;
    let lhs = diff.checked_mul(100)?;
    let rhs = average_cents.checked_mul(pct)?;
    Some(lhs > rhs)
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// A deposit onto a valid (non-negative) balance never yields a negative
    /// balance: it either succeeds with a non-negative result or reports overflow.
    #[kani::proof]
    fn deposit_preserves_non_negativity() {
        let balance: i64 = kani::any();
        let amount: i64 = kani::any();
        kani::assume(balance >= 0);
        kani::assume(amount >= 0);
        if let Some(post) = apply_deposit(balance, amount) {
            assert!(post >= 0);
        }
    }

    /// A negative "deposit" is always rejected by the template (deposits are
    /// non-negative by construction).
    #[kani::proof]
    fn negative_deposit_is_rejected() {
        let balance: i64 = kani::any();
        let amount: i64 = kani::any();
        kani::assume(amount < 0);
        assert!(apply_deposit(balance, amount).is_none());
    }

    /// Conversion of a non-negative amount at a non-negative rate is itself
    /// non-negative (or overflow-reported) — never silently negative.
    #[kani::proof]
    fn conversion_is_non_negative() {
        let minor: i64 = kani::any();
        let rate: i64 = kani::any();
        kani::assume(minor >= 0);
        kani::assume(rate >= 0);
        if let Some(usd) = checked_convert(minor, rate) {
            assert!(usd >= 0);
        }
    }

    /// The variance flag never panics for any inputs (all arithmetic is checked).
    #[kani::proof]
    fn variance_flag_never_panics() {
        let value: i64 = kani::any();
        let average: i64 = kani::any();
        let pct: i64 = kani::any();
        let _ = flag_variance(value, average, pct);
    }
}
