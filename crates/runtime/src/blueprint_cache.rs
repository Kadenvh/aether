//! The Blueprint Cache (U6, KTD2) — content-addressed, three layers:
//!
//! 1. **signature → `.wasm`** — keyed on the canonical t-DAG structure, lets a
//!    repeat run skip synthesis + rustc entirely.
//! 2. **`.wasm` hash → AOT artifact** — `precompile_module` once, then
//!    `deserialize` (Cranelift skipped) on every subsequent run.
//! 3. **warm `Engine`** — held by [`crate::Sandbox`]; combined with the AOT
//!    artifact this gives µs-scale instantiation.
//!
//! Everything is content-addressed on disk (blake3 of the source bytes), so the
//! cache is immutable and self-verifying: a different t-DAG or `.wasm` produces a
//! different key, and a stale/incompatible AOT artifact is rejected on load
//! (fail-closed) and recompiled rather than trusted.

use std::fs;
use std::path::PathBuf;

use wasmtime::{Engine, Module};

use aether_sdk::types::TDag;
use aether_sdk::{AetherError, Result};

use crate::aot;

const BLUEPRINTS_DIR: &str = "blueprints";
const AOT_DIR: &str = "aot";

/// A handle to an on-disk blueprint cache rooted at a directory.
pub struct BlueprintCache {
    root: PathBuf,
}

impl BlueprintCache {
    /// Open (creating if needed) a cache rooted at `root`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(root.join(BLUEPRINTS_DIR)).map_err(io)?;
        fs::create_dir_all(root.join(AOT_DIR)).map_err(io)?;
        Ok(BlueprintCache { root })
    }

    /// Layer-1 key: blake3 of the canonical t-DAG structure. Derived from the
    /// graph shape and node specs only — never from transient input data — so
    /// the same pipeline reuses the same blueprint across runs.
    pub fn tdag_signature(tdag: &TDag) -> Result<String> {
        let canonical = serde_json::to_vec(tdag)?;
        Ok(blake3::hash(&canonical).to_hex().to_string())
    }

    /// Layer-2 key: blake3 of the `.wasm` bytes.
    pub fn wasm_hash(wasm: &[u8]) -> String {
        blake3::hash(wasm).to_hex().to_string()
    }

    fn wasm_path(&self, signature: &str) -> PathBuf {
        self.root
            .join(BLUEPRINTS_DIR)
            .join(format!("{signature}.wasm"))
    }

    fn aot_path(&self, wasm_hash: &str) -> PathBuf {
        self.root.join(AOT_DIR).join(format!("{wasm_hash}.cwasm"))
    }

    /// Layer 1: the cached `.wasm` for a t-DAG signature, if present.
    pub fn wasm_for_signature(&self, signature: &str) -> Option<Vec<u8>> {
        fs::read(self.wasm_path(signature)).ok()
    }

    /// Layer 1: store a freshly compiled `.wasm` under its t-DAG signature.
    pub fn store_wasm(&self, signature: &str, wasm: &[u8]) -> Result<()> {
        fs::write(self.wasm_path(signature), wasm).map_err(io)
    }

    /// Layer 2: a ready-to-run [`Module`] for `wasm`, AOT-cached against `engine`.
    ///
    /// On a hit the artifact is loaded with Cranelift skipped; on a miss — or if
    /// a persisted artifact is rejected as stale/incompatible — it is
    /// (re)compiled, persisted, and loaded.
    pub fn module_for_wasm(&self, engine: &Engine, wasm: &[u8]) -> Result<Module> {
        let path = self.aot_path(&Self::wasm_hash(wasm));
        if let Ok(bytes) = fs::read(&path) {
            if let Ok(module) = aot::load_precompiled(engine, &bytes) {
                return Ok(module);
            }
            // Incompatible/corrupt artifact: discard and recompile (fail-closed).
            let _ = fs::remove_file(&path);
        }
        let artifact = aot::precompile(engine, wasm)?;
        fs::write(&path, &artifact).map_err(io)?;
        aot::load_precompiled(engine, &artifact)
    }
}

fn io(e: std::io::Error) -> AetherError {
    AetherError::Io(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_sdk::types::{EdgeKind, NodeKind, TDagEdge, TDagNode};

    fn tdag(nodes: &[(&str, NodeKind)]) -> TDag {
        TDag {
            nodes: nodes
                .iter()
                .map(|(id, kind)| TDagNode {
                    id: (*id).into(),
                    kind: *kind,
                    spec: serde_json::Value::Null,
                })
                .collect(),
            edges: vec![TDagEdge {
                from: nodes[0].0.into(),
                to: nodes[nodes.len() - 1].0.into(),
                kind: EdgeKind::DataFlow,
            }],
        }
    }

    #[test]
    fn signature_is_deterministic_and_structure_sensitive() {
        let a = tdag(&[("ingest", NodeKind::Ingest), ("persist", NodeKind::Persist)]);
        let a2 = tdag(&[("ingest", NodeKind::Ingest), ("persist", NodeKind::Persist)]);
        let b = tdag(&[("ingest", NodeKind::Ingest), ("flag", NodeKind::Flag)]);

        assert_eq!(
            BlueprintCache::tdag_signature(&a).unwrap(),
            BlueprintCache::tdag_signature(&a2).unwrap()
        );
        assert_ne!(
            BlueprintCache::tdag_signature(&a).unwrap(),
            BlueprintCache::tdag_signature(&b).unwrap()
        );
    }
}
