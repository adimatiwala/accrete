//! Embed the git commit the binary was built from, for run manifests.
//! Works offline; falls back to "unknown" outside a git checkout.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() {
    let commit = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into());
    // Tracked-file changes only; untracked files (e.g. results/) don't count.
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"])
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    println!("cargo:rustc-env=ACCRETE_GIT_COMMIT={commit}");
    println!("cargo:rustc-env=ACCRETE_GIT_DIRTY={dirty}");
    // Rebuild when HEAD moves or the index changes.
    for p in ["../../.git/HEAD", "../../.git/index"] {
        println!("cargo:rerun-if-changed={p}");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
