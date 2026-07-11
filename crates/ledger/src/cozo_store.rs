//! `Ledger` implemented over CozoDB with SQLite storage (U3, KTD5, R10).
//!
//! Append-only: events are only ever `:put` under a fresh, monotonically
//! increasing `idx` — never updated or deleted. A retraction is itself a
//! `Retract` event, and the correction log (`CompileFailure`,
//! `VerificationRejection`, `HumanIntervention`) lives in the *same* stream
//! (R10), so every record participates in the hash chain. On append the store
//! links `prev_hash` to the latest event and seals `curr_hash` (SDK).

use std::collections::BTreeMap;
use std::path::Path;

use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};

use aether_sdk::types::{EventKind, Ledger, LedgerEvent};
use aether_sdk::{AetherError, Result, Timestamp};

use crate::hashchain::{self, GENESIS_HASH};
use crate::schema::{CREATE_EVENTS, EVENTS_RELATION, PUT_EVENT, SELECT_ALL, SELECT_TAIL};
use crate::temporal;

/// The Unified Semantic Ledger backed by an on-disk CozoDB/SQLite database.
pub struct CozoLedger {
    db: DbInstance,
}

impl CozoLedger {
    /// Open (creating if needed) a ledger at `path`, ensuring the schema exists.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let db =
            DbInstance::new("sqlite", path, "").map_err(|e| AetherError::Ledger(e.to_string()))?;
        let ledger = CozoLedger { db };
        ledger.ensure_schema()?;
        Ok(ledger)
    }

    fn run(
        &self,
        script: &str,
        params: BTreeMap<String, DataValue>,
        mutable: bool,
    ) -> Result<NamedRows> {
        let mutability = if mutable {
            ScriptMutability::Mutable
        } else {
            ScriptMutability::Immutable
        };
        self.db
            .run_script(script, params, mutability)
            .map_err(|e| AetherError::Ledger(e.to_string()))
    }

    fn ensure_schema(&self) -> Result<()> {
        let relations = self.run("::relations", BTreeMap::new(), false)?;
        let name_col = relations
            .headers
            .iter()
            .position(|h| h == "name")
            .unwrap_or(0);
        let exists = relations
            .rows
            .iter()
            .any(|row| row.get(name_col).and_then(DataValue::get_str) == Some(EVENTS_RELATION));
        if !exists {
            self.run(CREATE_EVENTS, BTreeMap::new(), true)?;
        }
        Ok(())
    }

    /// All events in chain (insertion) order.
    fn all_events(&self) -> Result<Vec<LedgerEvent>> {
        let rows = self.run(SELECT_ALL, BTreeMap::new(), false)?;
        rows.rows.iter().map(|row| row_to_event(row)).collect()
    }

    /// The chain tail — latest `(idx, curr_hash)` only, or `None` if empty. O(1)-ish:
    /// fetches a single row and parses no payloads (vs `all_events`, which materializes
    /// the whole stream — the source of the former O(n²)-per-append cost).
    fn tail(&self) -> Result<Option<(i64, String)>> {
        let rows = self.run(SELECT_TAIL, BTreeMap::new(), false)?;
        match rows.rows.first() {
            None => Ok(None),
            Some(row) => {
                let idx = row
                    .first()
                    .and_then(DataValue::get_int)
                    .ok_or_else(|| AetherError::Ledger("tail idx not an int".into()))?;
                let curr_hash = row
                    .get(1)
                    .and_then(DataValue::get_str)
                    .map(str::to_string)
                    .ok_or_else(|| AetherError::Ledger("tail curr_hash not a string".into()))?;
                Ok(Some((idx, curr_hash)))
            }
        }
    }
}

impl Ledger for CozoLedger {
    fn append_event(&mut self, mut event: LedgerEvent) -> Result<()> {
        // O(1)-ish tail lookup (was: materialize all events → O(n²) over a run of appends).
        let tail = self.tail()?;
        let idx = tail.as_ref().map_or(0, |(i, _)| i + 1);
        event.prev_hash = tail
            .map(|(_, hash)| hash)
            .unwrap_or_else(|| GENESIS_HASH.to_string());
        let event = event.sealed();

        let valid_to = match event.valid_to {
            Some(ts) => DataValue::from(ts.0),
            None => DataValue::Null,
        };
        let params = BTreeMap::from([
            ("idx".to_string(), DataValue::from(idx)),
            ("id".to_string(), DataValue::from(event.id.clone())),
            ("tx_time".to_string(), DataValue::from(event.tx_time.0)),
            (
                "valid_from".to_string(),
                DataValue::from(event.valid_from.0),
            ),
            ("valid_to".to_string(), valid_to),
            (
                "kind".to_string(),
                DataValue::from(kind_to_str(&event.kind)?),
            ),
            (
                "payload".to_string(),
                DataValue::from(serde_json::to_string(&event.payload)?),
            ),
            (
                "prev_hash".to_string(),
                DataValue::from(event.prev_hash.clone()),
            ),
            (
                "curr_hash".to_string(),
                DataValue::from(event.curr_hash.clone()),
            ),
        ]);
        self.run(PUT_EVENT, params, true)?;
        Ok(())
    }

    fn latest_hash(&self) -> Option<String> {
        // O(1)-ish: the tail row only (was: materialize every event).
        self.tail().ok().flatten().map(|(_, curr_hash)| curr_hash)
    }

    fn query_as_of(&self, tx_time: Timestamp, valid_time: Timestamp) -> Result<Vec<LedgerEvent>> {
        Ok(temporal::as_of(&self.all_events()?, tx_time, valid_time))
    }

    fn verify_chain(&self) -> Result<()> {
        hashchain::verify_chain(&self.all_events()?)
    }
}

/// Serialize an [`EventKind`] to its snake_case wire string.
fn kind_to_str(kind: &EventKind) -> Result<String> {
    serde_json::to_value(kind)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| AetherError::Ledger("event kind did not serialize to a string".into()))
}

fn kind_from_str(s: &str) -> Result<EventKind> {
    serde_json::from_value(serde_json::Value::String(s.to_string())).map_err(AetherError::from)
}

/// Reconstruct a [`LedgerEvent`] from a `SELECT_ALL` row. Column order:
/// 0 idx, 1 id, 2 tx_time, 3 valid_from, 4 valid_to, 5 kind, 6 payload,
/// 7 prev_hash, 8 curr_hash.
fn row_to_event(row: &[DataValue]) -> Result<LedgerEvent> {
    let str_at = |i: usize| -> Result<String> {
        row.get(i)
            .and_then(DataValue::get_str)
            .map(str::to_string)
            .ok_or_else(|| AetherError::Ledger(format!("expected string at column {i}")))
    };
    let int_at = |i: usize| -> Result<i64> {
        row.get(i)
            .and_then(DataValue::get_int)
            .ok_or_else(|| AetherError::Ledger(format!("expected int at column {i}")))
    };
    let valid_to = match row.get(4) {
        Some(DataValue::Null) | None => None,
        Some(v) => {
            Some(Timestamp(v.get_int().ok_or_else(|| {
                AetherError::Ledger("valid_to is not an int".into())
            })?))
        }
    };
    Ok(LedgerEvent {
        id: str_at(1)?,
        kind: kind_from_str(&str_at(5)?)?,
        payload: serde_json::from_str(&str_at(6)?)?,
        tx_time: Timestamp(int_at(2)?),
        valid_from: Timestamp(int_at(3)?),
        valid_to,
        prev_hash: str_at(7)?,
        curr_hash: str_at(8)?,
    })
}
