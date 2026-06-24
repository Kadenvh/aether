//! U3 verification: a temp-file CozoDB ledger is created, populated across the
//! event + correction streams, hash-chain-verified, queried bi-temporally, and
//! survives reopen.

use std::time::{SystemTime, UNIX_EPOCH};

use aether_sdk::types::{EventKind, Ledger, LedgerEvent};
use aether_sdk::Timestamp;

use aether_ledger::CozoLedger;

/// A unique temp database path, removed on drop.
struct TempDb {
    path: std::path::PathBuf,
}

impl TempDb {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        TempDb {
            path: std::env::temp_dir().join(format!("aether-ledger-{nanos}.db")),
        }
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn event(id: &str, kind: EventKind, tx: i64, from: i64, to: Option<i64>) -> LedgerEvent {
    LedgerEvent {
        id: id.into(),
        kind,
        payload: serde_json::json!({ "id": id }),
        tx_time: Timestamp(tx),
        valid_from: Timestamp(from),
        valid_to: to.map(Timestamp),
        prev_hash: String::new(),
        curr_hash: String::new(),
    }
}

#[test]
fn append_verify_query_and_persist() {
    let db = TempDb::new();

    {
        let mut ledger = CozoLedger::open(&db.path).expect("open ledger");
        assert!(ledger.latest_hash().is_none(), "fresh ledger has no events");

        // A normal assertion, a correction-log entry (R10), and a retraction —
        // all in the same hash-chained stream.
        ledger
            .append_event(event("partner-bill-1", EventKind::Assert, 10, 10, None))
            .unwrap();
        ledger
            .append_event(event(
                "synth-fail-1",
                EventKind::CompileFailure,
                20,
                20,
                None,
            ))
            .unwrap();
        ledger
            .append_event(event("partner-bill-1", EventKind::Retract, 30, 30, None))
            .unwrap();

        // The chain is intact end to end.
        ledger.verify_chain().expect("hash chain must verify");
        assert!(ledger.latest_hash().is_some());

        // Bi-temporal query: as of tx_time 15 (only the first event was recorded)
        // and valid_time 15, exactly the first assertion is visible.
        let visible = ledger.query_as_of(Timestamp(15), Timestamp(15)).unwrap();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, "partner-bill-1");
        assert_eq!(visible[0].kind, EventKind::Assert);

        // As of the latest tx_time, all three events are present.
        let all = ledger.query_as_of(Timestamp(100), Timestamp(100)).unwrap();
        assert_eq!(all.len(), 3);
    }

    // Reopen the same file: data persisted and the chain still verifies.
    let reopened = CozoLedger::open(&db.path).expect("reopen ledger");
    reopened
        .verify_chain()
        .expect("chain verifies after reopen");
    assert_eq!(
        reopened
            .query_as_of(Timestamp(100), Timestamp(100))
            .unwrap()
            .len(),
        3
    );
}
