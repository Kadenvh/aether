//! Bi-temporal "as-of" reconstruction (U3, KTD5).
//!
//! Two distinct, non-collapsing temporal axes:
//! - **transaction time** (`tx_time`) — when the engine recorded the event,
//! - **valid time** (`valid_from`/`valid_to`) — when the fact is true in the
//!   modeled world.
//!
//! [`as_of`] returns the events visible at a `(tx_time, valid_time)` point: those
//! recorded at or before the query transaction time whose valid-time window
//! covers the query valid time. A `valid_to` of `None` means "still valid".

use aether_sdk::types::LedgerEvent;
use aether_sdk::Timestamp;

/// The events logically visible as of `(tx_time, valid_time)`.
pub fn as_of(
    events: &[LedgerEvent],
    tx_time: Timestamp,
    valid_time: Timestamp,
) -> Vec<LedgerEvent> {
    events
        .iter()
        .filter(|e| {
            e.tx_time.0 <= tx_time.0
                && e.valid_from.0 <= valid_time.0
                && e.valid_to.is_none_or(|vt| valid_time.0 < vt.0)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_sdk::types::{EventKind, LedgerEvent};

    fn event(id: &str, tx: i64, from: i64, to: Option<i64>) -> LedgerEvent {
        LedgerEvent {
            id: id.into(),
            kind: EventKind::Assert,
            payload: serde_json::Value::Null,
            tx_time: Timestamp(tx),
            valid_from: Timestamp(from),
            valid_to: to.map(Timestamp),
            prev_hash: String::new(),
            curr_hash: String::new(),
        }
    }

    fn ids(events: &[LedgerEvent]) -> Vec<&str> {
        events.iter().map(|e| e.id.as_str()).collect()
    }

    #[test]
    fn excludes_events_recorded_after_the_query_tx_time() {
        let events = vec![event("early", 10, 0, None), event("late", 30, 0, None)];
        assert_eq!(
            ids(&as_of(&events, Timestamp(20), Timestamp(100))),
            vec!["early"]
        );
    }

    #[test]
    fn respects_valid_time_window() {
        // "open" is valid from 0 forever; "closed" only on [0, 50).
        let events = vec![event("open", 1, 0, None), event("closed", 1, 0, Some(50))];
        // At valid_time 25 both apply; at 75 only the open one does.
        assert_eq!(
            ids(&as_of(&events, Timestamp(100), Timestamp(25))),
            vec!["open", "closed"]
        );
        assert_eq!(
            ids(&as_of(&events, Timestamp(100), Timestamp(75))),
            vec!["open"]
        );
    }

    #[test]
    fn excludes_facts_not_yet_valid() {
        let events = vec![event("future", 1, 200, None)];
        assert!(as_of(&events, Timestamp(100), Timestamp(100)).is_empty());
    }
}
