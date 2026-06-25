//! `aether watch` daemon core (U16).
//!
//! The watch loop monitors an input source and runs the matching intent's
//! pipeline on each new file. Steady-state runs hit the Blueprint Cache (U6), so
//! synthesis/compile are bypassed and only the cache → AOT → TAR → FVL → USL
//! runtime path executes. This module holds the dependency-free, testable core:
//! directory scanning and per-file orchestration. The live poll loop +
//! graceful-shutdown wiring lives in [`crate::watch`].

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use aether_sdk::types::{Ledger, TDag};
use aether_sdk::{AetherError, Result, Timestamp};

use crate::orchestrator::{NodeSynthesizer, Orchestrator};

/// Return the files in `source` not yet in `seen`, recording them as seen.
/// Deterministic order (sorted) so repeated runs over the same drop are stable.
pub fn scan_new(source: &Path, seen: &mut HashSet<PathBuf>) -> Result<Vec<PathBuf>> {
    let mut fresh = Vec::new();
    let entries = std::fs::read_dir(source)
        .map_err(|e| AetherError::Io(format!("watch source '{}': {e}", source.display())))?;
    for entry in entries {
        let path = entry.map_err(|e| AetherError::Io(e.to_string()))?.path();
        if path.is_file() && seen.insert(path.clone()) {
            fresh.push(path);
        }
    }
    fresh.sort();
    Ok(fresh)
}

/// Process every new file in `source` through the pipeline, one transaction
/// each. Returns how many were processed this tick. `base.0 + processed_before`
/// seeds each run's timestamp so successive runs are ordered and deterministic.
pub async fn process_new<S: NodeSynthesizer, L: Ledger>(
    orch: &mut Orchestrator<'_, S, L>,
    tdag: &TDag,
    source: &Path,
    seen: &mut HashSet<PathBuf>,
    base: Timestamp,
    processed_before: usize,
) -> Result<usize> {
    let fresh = scan_new(source, seen)?;
    let mut done = 0usize;
    for _file in &fresh {
        let ts = Timestamp(base.0 + (processed_before + done) as i64);
        orch.run(tdag, ts).await?;
        done += 1;
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_new_reports_each_file_once() {
        let dir = std::env::temp_dir().join(format!("aether-scan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.csv"), b"x").unwrap();
        std::fs::write(dir.join("b.csv"), b"y").unwrap();

        let mut seen = HashSet::new();
        let first = scan_new(&dir, &mut seen).unwrap();
        assert_eq!(first.len(), 2, "both files are new on first scan");

        // Nothing new on the second scan.
        assert!(scan_new(&dir, &mut seen).unwrap().is_empty());

        // A freshly dropped file is detected.
        std::fs::write(dir.join("c.csv"), b"z").unwrap();
        assert_eq!(scan_new(&dir, &mut seen).unwrap().len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
