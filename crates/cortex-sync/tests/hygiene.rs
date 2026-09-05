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

/// M9: the gate is verified against a seeded fake secret, not trusted because it exists.
#[test]
fn the_gate_refuses_a_seeded_fake_secret_and_passes_a_clean_file() {
    let script = repository_root().join("scripts").join("pre-commit");
    let repo = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git").arg("-C").arg(repo.path()).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "test@example.com"]);
    git(&["config", "user.name", "test"]);

    let gate = |expect_clean: bool, what: &str| {
        let out = Command::new("bash")
            .arg(&script)
            .current_dir(repo.path())
            .output()
            .unwrap();
        assert_eq!(
            out.status.success(),
            expect_clean,
            "{what}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };

    std::fs::write(repo.path().join("clean.md"), "nothing to see, see example.com\n").unwrap();
    git(&["add", "clean.md"]);
    gate(true, "a clean file");
    git(&["commit", "-q", "-m", "clean"]);

    // A provider-shaped token, assembled at runtime so the source of this test is itself clean.
    let token = format!("{}{}", "AKIA", "ABCDEFGHIJKLMNOP");
    std::fs::write(repo.path().join("leak.md"), format!("key = {token}\n")).unwrap();
    git(&["add", "leak.md"]);
    gate(false, "a seeded credential");

    // A private home path and a private address are refused the same way.
    let home_path = format!("/{}/{}/{}/", "home", "somebody", "secret-notes");
    std::fs::write(repo.path().join("leak.md"), format!("see {home_path}\n")).unwrap();
    git(&["add", "leak.md"]);
    gate(false, "a private home path");
    let endpoint = format!("{}://{}.{}.{}", "https", "brain", "somebody", "example");
    std::fs::write(repo.path().join("leak.md"), format!("peer = \"{endpoint}\"\n")).unwrap();
    git(&["add", "leak.md"]);
    gate(false, "an endpoint");
}

/// M9 §3: `git log -p` over the entire public history contains no secret and no private
/// identifier. Same patterns, every patch ever committed.
#[test]
fn the_entire_history_names_no_secret_and_no_private_identifier() {
    let root = repository_root();
    let inside_git = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !inside_git {
        eprintln!("not a git checkout; no history to scan");
        return;
    }
    let output = Command::new("bash")
        .arg(root.join("scripts").join("pre-commit"))
        .arg("--history")
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "the history contains something the hygiene gate refuses:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
