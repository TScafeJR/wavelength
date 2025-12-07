//! Checks the engine against a real browser. This needs Chrome or Chromium
//! installed, so it only runs on request: `cargo test -- --ignored`.

#![allow(clippy::expect_used, reason = "a test should fail loudly")]

use std::path::Path;
use std::process::Command;

#[test]
#[ignore = "needs a local Chrome or Chromium"]
fn fixtures_match_chrome_exactly() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut fixtures: Vec<_> = std::fs::read_dir(root.join("fixtures"))
        .expect("fixtures directory")
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "html"))
        .collect();
    fixtures.sort();
    assert!(!fixtures.is_empty());

    let output = Command::new(env!("CARGO_BIN_EXE_wavelength"))
        .arg("compare")
        .args(&fixtures)
        .args(["--max-diff", "0", "--out"])
        .arg(Path::new(env!("CARGO_TARGET_TMPDIR")).join("parity"))
        .output()
        .expect("run the compare command");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
