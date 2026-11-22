//! Embed the git commit the binary was built from, and whether tracked files
//! had uncommitted changes, for run manifests. Works offline; falls back to
//! "unknown" outside a git checkout.
//!
//! Staleness: the dirty flag must change when sources change, so the script
//! re-runs whenever any workspace source, manifest or the lock file changes,
//! and when git's HEAD, index or current branch ref move. Paths come from
//! `git rev-parse`, so this also works in a git worktree (where `.git` is a
//! file) without forcing a rebuild every time.

use std::path::{Path, PathBuf};
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        // `git status` would otherwise refresh the index, which would itself
        // trigger the next re-run of this script.
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn watch(p: &Path) {
    if p.exists() {
        println!("cargo:rerun-if-changed={}", p.display());
    }
}

fn main() {
    let commit = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into());
    // Tracked-file changes only; untracked files (e.g. results/) don't count.
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"])
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    println!("cargo:rustc-env=ACCRETE_GIT_COMMIT={commit}");
    println!("cargo:rustc-env=ACCRETE_GIT_DIRTY={dirty}");

    // Workspace sources: any edit can flip the dirty flag.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for p in [
        "crates/accrete-core/src",
        "crates/accrete-core/Cargo.toml",
        "crates/accrete-viz/src",
        "crates/accrete-viz/Cargo.toml",
        "crates/accrete-cli/src",
        "crates/accrete-cli/Cargo.toml",
        "Cargo.toml",
        "Cargo.lock",
        "configs",
    ] {
        watch(&root.join(p));
    }
    // Git state: HEAD and the index (per worktree), and the branch ref.
    if let Some(dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        let dir = PathBuf::from(dir);
        watch(&dir.join("HEAD"));
        watch(&dir.join("index"));
    }
    if let (Some(common), Some(head_ref)) = (
        git(&["rev-parse", "--path-format=absolute", "--git-common-dir"]),
        git(&["symbolic-ref", "-q", "HEAD"]),
    ) {
        watch(&PathBuf::from(common).join(head_ref));
    }
    println!("cargo:rerun-if-changed=build.rs");
}
