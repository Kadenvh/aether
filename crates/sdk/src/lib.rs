//! AETHER shared vocabulary (SDK).
//!
//! The single crate every other AETHER subsystem depends on: intent, t-DAG,
//! capability, mutation, ledger event/trait, money, and the error envelope.
//! Types here are pure data (`serde`-derived); behavior that needs heavier
//! dependencies (graph construction, the concrete ledger store, the WASM host)
//! lives in the subsystem crates that consume these types.

use serde::{Deserialize, Serialize};

pub mod error;
pub mod types;

pub use error::{AetherError, Result};
pub use types::{
    Capability, Cents, ClockPolicy, EdgeKind, EventKind, Intent, IoDescriptor, IoFormat, Ledger,
    LedgerEvent, Mutation, NetRule, NodeId, NodeKind, PreopenedDir, TDag, TDagEdge, TDagNode,
};

/// Epoch-millisecond timestamp. The SDK never reads a wall clock; time is
/// supplied by the caller (the ledger layer stamps transaction-time, U3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(pub i64);
