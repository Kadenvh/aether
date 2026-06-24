//! U6 verification: the blueprint cache round-trips wasm by signature, produces
//! a runnable AOT artifact, and reuses it on the second call.

use std::time::{SystemTime, UNIX_EPOCH};

use aether_runtime::{BlueprintCache, ExecLimits, Sandbox};

const RETURNS_42: &str = r#"
    (module
        (func (export "run") (result i32)
            i32.const 42))
"#;

/// A unique scratch directory for one test, cleaned up at the end.
struct TempDir {
    path: std::path::PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("aether-bp-{tag}-{nanos}"));
        std::fs::create_dir_all(&path).unwrap();
        TempDir { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn layer1_wasm_round_trips_by_signature() {
    let dir = TempDir::new("l1");
    let cache = BlueprintCache::open(&dir.path).unwrap();

    assert!(cache.wasm_for_signature("missing").is_none());
    cache.store_wasm("sig123", b"\0asm-bytes").unwrap();
    assert_eq!(
        cache.wasm_for_signature("sig123").as_deref(),
        Some(&b"\0asm-bytes"[..])
    );
}

#[tokio::test]
async fn aot_artifact_is_persisted_and_runnable() {
    let dir = TempDir::new("aot");
    let sandbox = Sandbox::new().unwrap();
    let cache = BlueprintCache::open(&dir.path).unwrap();
    let limits = ExecLimits::default();
    let wasm = RETURNS_42.as_bytes();

    // First call: cold precompile + persist.
    let module = cache.module_for_wasm(sandbox.engine(), wasm).unwrap();
    let out = sandbox
        .run_module_i32(&module, "run", &limits)
        .await
        .unwrap();
    assert_eq!(out, 42);

    // An artifact file now exists on disk.
    let aot_files: Vec<_> = std::fs::read_dir(dir.path.join("aot")).unwrap().collect();
    assert_eq!(
        aot_files.len(),
        1,
        "exactly one AOT artifact should be cached"
    );

    // Second call: warm load of the same artifact, still runnable.
    let module2 = cache.module_for_wasm(sandbox.engine(), wasm).unwrap();
    let out2 = sandbox
        .run_module_i32(&module2, "run", &limits)
        .await
        .unwrap();
    assert_eq!(out2, 42);
}

/// Warm (AOT deserialize) instantiation should be much faster than the cold
/// precompile path. Timing-sensitive, so it is opt-in.
#[tokio::test]
#[ignore = "microbenchmark; run explicitly with --ignored"]
async fn warm_load_beats_cold_precompile() {
    use std::time::Instant;

    let dir = TempDir::new("bench");
    let sandbox = Sandbox::new().unwrap();
    let cache = BlueprintCache::open(&dir.path).unwrap();
    let wasm = RETURNS_42.as_bytes();

    let t0 = Instant::now();
    let _cold = cache.module_for_wasm(sandbox.engine(), wasm).unwrap();
    let cold = t0.elapsed();

    let t1 = Instant::now();
    let _warm = cache.module_for_wasm(sandbox.engine(), wasm).unwrap();
    let warm = t1.elapsed();

    println!("cold precompile: {cold:?}, warm load: {warm:?}");
    assert!(
        warm < cold,
        "warm load ({warm:?}) should beat cold precompile ({cold:?})"
    );
}
