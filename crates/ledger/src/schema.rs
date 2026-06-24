//! The CozoDB relation schema and scripts for the event stream (U3).
//!
//! A single append-only stored relation `events`, keyed by a dense `idx`
//! (insertion order = chain order). Both temporal axes are explicit columns:
//! `tx_time` (transaction time) and `valid_from`/`valid_to` (valid time). The
//! hash-chain fields `prev_hash`/`curr_hash` make the stream tamper-evident.
//!
//! Note (KTD5): the plan suggested CozoDB's native `Validity` for transaction
//! time; this implementation uses explicit integer columns for *both* axes,
//! which keeps the two axes distinct (the KTD5 requirement) with far less
//! query-language surface. Native `Validity` remains an available optimization.

/// Name of the stored relation.
pub const EVENTS_RELATION: &str = "events";

/// Create the append-only event relation.
pub const CREATE_EVENTS: &str = r#"
:create events {
    idx: Int
    =>
    id: String,
    tx_time: Int,
    valid_from: Int,
    valid_to: Int?,
    kind: String,
    payload: String,
    prev_hash: String,
    curr_hash: String,
}
"#;

/// Select every event in chain order.
pub const SELECT_ALL: &str = r#"
?[idx, id, tx_time, valid_from, valid_to, kind, payload, prev_hash, curr_hash] :=
    *events{idx, id, tx_time, valid_from, valid_to, kind, payload, prev_hash, curr_hash}
:order idx
"#;

/// Append one event (parameters bound by [`crate::cozo_store`]).
pub const PUT_EVENT: &str = r#"
?[idx, id, tx_time, valid_from, valid_to, kind, payload, prev_hash, curr_hash] <-
    [[$idx, $id, $tx_time, $valid_from, $valid_to, $kind, $payload, $prev_hash, $curr_hash]]
:put events {idx => id, tx_time, valid_from, valid_to, kind, payload, prev_hash, curr_hash}
"#;
