//! AETHER USL (Unified Semantic Ledger).
//!
//! An append-only, bi-temporal, hash-chained event stream over CozoDB (U3).
//! Concrete [`CozoLedger`] implements the SDK `Ledger` trait (KTD5), so the
//! store is swappable. The correction log shares the same stream (R10).

pub mod cozo_store;
pub mod hashchain;
pub mod schema;
pub mod temporal;

pub use cozo_store::CozoLedger;
pub use hashchain::{verify_chain, GENESIS_HASH};
