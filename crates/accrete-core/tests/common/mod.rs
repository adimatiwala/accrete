//! Shared helpers for integration tests.
//!
//! Golden hashes live in `tests/golden/<name>.txt` at the repository root,
//! shared by every OS. To regenerate them deliberately (after a change that is
//! *meant* to alter results), run `ACCRETE_BLESS=1 cargo test` and note it in
//! the commit message.

#![allow(dead_code)]

use std::path::PathBuf;

pub const PRIMARY_SEED: u64 = 1;

pub fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("golden")
}

pub fn check_golden(name: &str, actual: u64) {
    let path = golden_dir().join(format!("{name}.txt"));
    let actual_hex = accrete_core::hash::hex(actual);
    if std::env::var_os("ACCRETE_BLESS").is_some() {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(&path, format!("{actual_hex}\n")).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "missing golden file {} (run with ACCRETE_BLESS=1)",
            path.display()
        )
    });
    assert_eq!(
        expected.trim(),
        actual_hex,
        "golden hash {name} changed; if intended, re-bless with ACCRETE_BLESS=1"
    );
}
