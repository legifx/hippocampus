//! M7 §4: "a grep of the public repository for endpoint patterns, private paths and
//! credentials returns nothing. This runs in CI."
//!
//! The grep is `scripts/pre-commit --all` — the same patterns the commit hook applies to staged
//! files, over every tracked file. Running it from `cargo test` makes it a gate wherever the
//! tests run, which is what "in CI" means for a project whose CI is its test suite. It lives
//! in this crate because federation is where an endpoint would be most tempting to inline.

use std::path::PathBuf;
use std::process::Command;

fn repository_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.pop();
    dir.pop();
    dir
}

#[test]
fn the_public_repository_names_no_endpoint_no_private_path_and_no_credential() {
    let root = repository_root();
    let script = root.join("scripts").join("pre-commit");
    assert!(script.exists(), "{} is missing", script.display());

    // Without a repository there is nothing to list; a source tarball is not what this gate
    // is about, and saying so beats a green result that checked nothing.
    let inside_git = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !inside_git {
        eprintln!("not a git checkout; the hygiene gate has nothing to scan here");
        return;
    }

    let output = Command::new("bash")
        .arg(&script)
        .arg("--all")
        .current_dir(&root)
        .output()
        .expect("bash runs the hygiene gate");
    assert!(
        output.status.success(),
        "the hygiene gate found something:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
