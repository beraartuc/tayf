//! Build script: emit version metadata constants without external crates.
//!
//! Replaces the `built` crate (which dragged in git2 + libgit2-sys + the full
//! ICU stack as transitive deps) with stdlib calls to `git rev-parse` and
//! `rustc --version`. Falls back to "unknown" when the source tree has no
//! `.git` directory (e.g., a crates.io tarball build) or when either command
//! is unavailable. The emitted env vars are consumed by `src/version.rs`.

use std::process::Command;

fn main() {
    let sha = git_sha().unwrap_or_else(|| "unknown".to_string());
    let dirty = git_dirty().unwrap_or(false);
    let rustc = rustc_version().unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=TAYF_GIT_SHA={sha}");
    println!("cargo:rustc-env=TAYF_GIT_DIRTY={}", if dirty { "1" } else { "0" });
    println!("cargo:rustc-env=TAYF_RUSTC={rustc}");
    // Declare `fuzzing` as a known cfg so rustc's check-cfg lint does not
    // warn when cargo-fuzz passes `--cfg fuzzing` through RUSTFLAGS.
    println!("cargo:rustc-check-cfg=cfg(fuzzing)");
    // Re-run when HEAD moves (commit, checkout) or the index changes (stage,
    // unstage). Both files are absent in tarball builds; cargo silently
    // ignores missing rerun-if-changed paths, which is the desired behaviour.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
}

/// Return the full 40-character git commit SHA for `HEAD`, or `None` if `git`
/// is unavailable or the working tree is not a repository.
fn git_sha() -> Option<String> {
    let out = Command::new("git").args(["rev-parse", "HEAD"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    Some(s.trim().to_string())
}

/// Return `true` if the working tree has uncommitted changes, `false` if
/// clean, or `None` if `git status` could not be invoked.
fn git_dirty() -> Option<bool> {
    let out = Command::new("git").args(["status", "--porcelain"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(!out.stdout.is_empty())
}

/// Return the output of `rustc --version` (typically `rustc 1.79.0 (...)`),
/// or `None` if `rustc` could not be invoked. Uses the `$RUSTC` env var that
/// Cargo sets when invoking build scripts, falling back to plain `rustc`
/// from `$PATH`.
fn rustc_version() -> Option<String> {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let out = Command::new(rustc).arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    Some(s.trim().to_string())
}
