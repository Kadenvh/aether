//! Tamper-evident hash-chain verification (U3, R10).
//!
//! Each [`LedgerEvent`] seals `curr_hash = blake3(canonical(event) || prev_hash)`
//! (computed in the SDK). A valid chain has every event's `curr_hash` matching
//! its content and every `prev_hash` linking to the previous event's `curr_hash`
//! — the first linking to [`GENESIS_HASH`]. Any edit to a past event's content,
//! ordering, or hashes breaks the chain here.

use aether_sdk::types::LedgerEvent;
use aether_sdk::{AetherError, Result};

/// The `prev_hash` of the very first event in a ledger.
pub const GENESIS_HASH: &str = "GENESIS";

/// Walk the chain in order and confirm end-to-end tamper-evidence.
pub fn verify_chain(events: &[LedgerEvent]) -> Result<()> {
    let mut expected_prev = GENESIS_HASH.to_string();
    for (i, event) in events.iter().enumerate() {
        if !event.hash_is_valid() {
            return Err(AetherError::IntegrityFailed(format!(
                "event {i} ('{}') has a tampered or unsealed hash",
                event.id
            )));
        }
        if event.prev_hash != expected_prev {
            return Err(AetherError::IntegrityFailed(format!(
                "event {i} ('{}') breaks the chain: prev_hash '{}' != expected '{}'",
                event.id, event.prev_hash, expected_prev
            )));
        }
        expected_prev = event.curr_hash.clone();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_sdk::types::{EventKind, LedgerEvent};
    use aether_sdk::Timestamp;

    fn event(id: &str, prev: &str) -> LedgerEvent {
        LedgerEvent {
            id: id.into(),
            kind: EventKind::Assert,
            payload: serde_json::json!({"v": id}),
            tx_time: Timestamp(1),
            valid_from: Timestamp(1),
            valid_to: None,
            prev_hash: prev.into(),
            curr_hash: String::new(),
        }
        .sealed()
    }

    fn chain() -> Vec<LedgerEvent> {
        let e0 = event("a", GENESIS_HASH);
        let e1 = event("b", &e0.curr_hash);
        vec![e0, e1]
    }

    #[test]
    fn accepts_a_well_formed_chain() {
        assert!(verify_chain(&chain()).is_ok());
    }

    #[test]
    fn rejects_tampered_payload() {
        let mut events = chain();
        events[0].payload = serde_json::json!({"v": "evil"}); // curr_hash no longer matches
        assert!(verify_chain(&events).is_err());
    }

    #[test]
    fn rejects_broken_link() {
        let mut events = chain();
        events[1].prev_hash = "wrong".into();
        // re-seal so the event's own hash is valid but the link is broken
        events[1] = events[1].clone().sealed();
        assert!(verify_chain(&events).is_err());
    }

    #[test]
    fn empty_chain_is_valid() {
        assert!(verify_chain(&[]).is_ok());
    }
}
