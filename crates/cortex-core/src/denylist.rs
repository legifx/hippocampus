//! The deny-list, per hard rule R1.
//!
//! Two separate lists with different consequences, and conflating them is a real bug:
//!
//! - **Sensitive**: never read, never extracted, and not indexed at all. Credentials, keys,
//!   browser profiles. The user gains nothing from these being findable and loses a great deal
//!   if their contents end up in an index or in a search result snippet.
//! - **Bulky**: indexed as metadata (the paths are genuinely useful) but never extracted.
//!   Build output, caches, dependency trees.
//!
//! The built-in entries cannot be removed by configuration. Extra patterns may be added.
//! This is deliberate: a user who accidentally deletes the `.ssh` entry from their config
//! should not silently start indexing their private keys.

use std::path::Path;

/// Directory names whose entire subtree is sensitive and must never be read or indexed.
const SENSITIVE_DIRS: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".kube",
    ".password-store",
    "keyrings",
];

/// Path fragments that mark a sensitive location. Matched against the whole path.
const SENSITIVE_PATH_FRAGMENTS: &[&str] = &[
    "/.mozilla/",
    "/.config/google-chrome/",
    "/.config/chromium/",
    "/.config/BraveSoftware/",
    "/.local/share/keyrings/",
    "/.docker/config.json",
];

/// Exact file names that are sensitive wherever they appear.
const SENSITIVE_NAMES: &[&str] = &[
    ".env",
    ".netrc",
    ".pgpass",
    ".my.cnf",
    "known_hosts",
    "authorized_keys",
];

/// Sensitive by prefix.
const SENSITIVE_PREFIXES: &[&str] = &["id_rsa", "id_ed25519", "id_ecdsa", "id_dsa", ".env."];

/// Sensitive by extension.
const SENSITIVE_EXTENSIONS: &[&str] = &["pem", "key", "p12", "pfx", "kdbx", "keystore", "jks"];

/// Sensitive when the name contains one of these, case-insensitively.
const SENSITIVE_SUBSTRINGS: &[&str] = &["credential", "secret", "token"];

/// Directory names that are bulky: worth knowing about, never worth reading.
const BULKY_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "build",
    "dist",
    ".next",
    ".dart_tool",
    ".venv",
    "venv",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".gradle",
    ".cargo",
    ".rustup",
    ".npm",
    ".cache",
    ".git",
];

/// Extensions that are compiled or otherwise opaque output.
const BULKY_EXTENSIONS: &[&str] = &[
    "o", "a", "so", "dylib", "rlib", "pyc", "class", "wasm", "obj", "lib", "pdb",
    // Data, not documents. A directory of benchmark results or training examples in JSON Lines
    // is thousands of files of varied natural language, and a semantic index finds a little of
    // every question in it: measured on a real workstation, such files took the first places
    // for questions about projects they had nothing to do with. Their paths stay searchable.
    "jsonl", "ndjson", "parquet", "arrow", "feather", "npy", "npz", "safetensors", "gguf", "onnx",
    "pt", "ckpt",
];

/// What the classifier is allowed to do with a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Index metadata and extract content (subject to rule R3's content roots).
    Allow,
    /// Index metadata only. Never open the file.
    MetadataOnly,
    /// Do not index at all, and never open the file.
    Sensitive,
}

impl Verdict {
    /// Whether an extractor may open this file. The single question rule R1 exists to answer.
    pub fn may_read_content(self) -> bool {
        matches!(self, Verdict::Allow)
    }

    pub fn may_index(self) -> bool {
        !matches!(self, Verdict::Sensitive)
    }
}

/// Classify a path.
///
/// `extra` holds additional user-configured patterns; they can only make the verdict stricter.
/// Each entry is matched as a directory name, a file name, or a `*.ext` suffix.
pub fn classify(path: &Path, extra: &[String]) -> Verdict {
    let path_str = path.to_string_lossy();

    for component in path.components() {
        let Some(name) = component.as_os_str().to_str() else {
            continue;
        };
        if SENSITIVE_DIRS.iter().any(|d| d.eq_ignore_ascii_case(name)) {
            return Verdict::Sensitive;
        }
    }

    if SENSITIVE_PATH_FRAGMENTS
        .iter()
        .any(|fragment| path_str.contains(fragment))
    {
        return Verdict::Sensitive;
    }

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let lower_name = file_name.to_ascii_lowercase();

    if SENSITIVE_NAMES.iter().any(|n| n.eq_ignore_ascii_case(file_name))
        || SENSITIVE_PREFIXES.iter().any(|p| lower_name.starts_with(p))
        || SENSITIVE_SUBSTRINGS.iter().any(|s| lower_name.contains(s))
    {
        return Verdict::Sensitive;
    }

    if let Some(ext) = extension_of(&lower_name) {
        if SENSITIVE_EXTENSIONS.iter().any(|e| *e == ext) {
            return Verdict::Sensitive;
        }
    }

    for pattern in extra {
        if matches_pattern(pattern, path, &lower_name) {
            return Verdict::Sensitive;
        }
    }

    for component in path.components() {
        let Some(name) = component.as_os_str().to_str() else {
            continue;
        };
        if BULKY_DIRS.contains(&name) {
            return Verdict::MetadataOnly;
        }
    }

    if let Some(ext) = extension_of(&lower_name) {
        if BULKY_EXTENSIONS.iter().any(|e| *e == ext) {
            return Verdict::MetadataOnly;
        }
    }

    Verdict::Allow
}

/// Whether an entire directory subtree can be skipped during a walk.
///
/// Descending into `node_modules` to record half a million paths costs far more than it is
/// worth, and nothing in there is a document a human is looking for.
pub fn should_skip_subtree(dir_name: &str) -> bool {
    BULKY_DIRS.contains(&dir_name) || SENSITIVE_DIRS.iter().any(|d| d.eq_ignore_ascii_case(dir_name))
}

/// The marker a directory carries to keep itself and everything below it out of the index.
///
/// An empty file with this name is enough. It is the user's own deny-list entry, placed where
/// the data is rather than in a configuration file that has to know the path — which is what
/// makes it survive a move, a second machine, and a second user who never edited a config.
pub const IGNORE_MARKER: &str = ".brainignore";

/// Whether a directory asked to be left alone: it contains [`IGNORE_MARKER`].
///
/// Every walk — first index, reconcile, watcher — asks this for each directory it is about to
/// descend into, so a marked tree is neither indexed, nor watched, nor re-added by the scan
/// that exists to catch what the watcher missed. The case that made it necessary: the
/// benchmark corpus lives inside the repository, the repository lives inside a content root,
/// and for two weeks the live index was 74 % synthetic Zipf text — the first hit for "report"
/// was a generated file, and the working-set arm of every context pack offered the corpus
/// database. The generator now drops this marker beside what it writes.
pub fn marked_ignored(dir: &Path) -> bool {
    dir.join(IGNORE_MARKER).is_file()
}

/// Extension without the dot, lowercase, or `None` when there is none.
///
/// Deliberately hand-rolled rather than `Path::extension` because this operates on an
/// already-lowercased name and must treat a leading dot as "no extension" — `.env` has no
/// extension, it is a name.
fn extension_of(lower_name: &str) -> Option<&str> {
    let dot = lower_name.rfind('.')?;
    if dot == 0 {
        return None;
    }
    Some(&lower_name[dot + 1..])
}

fn matches_pattern(pattern: &str, path: &Path, lower_name: &str) -> bool {
    let pattern_lower = pattern.to_ascii_lowercase();
    if let Some(ext) = pattern_lower.strip_prefix("*.") {
        return extension_of(lower_name) == Some(ext);
    }
    if lower_name == pattern_lower {
        return true;
    }
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .is_some_and(|n| n.eq_ignore_ascii_case(&pattern_lower))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn verdict(path: &str) -> Verdict {
        classify(&PathBuf::from(path), &[])
    }

    #[test]
    fn credential_locations_are_never_indexed() {
        for path in [
            "/home/user/.ssh/id_ed25519",
            "/home/user/.ssh/config",
            "/home/user/.gnupg/secring.gpg",
            "/home/user/.aws/credentials",
            "/home/user/.kube/config",
            "/home/user/.local/share/keyrings/login.keyring",
            "/home/user/.password-store/mail.gpg",
        ] {
            assert_eq!(verdict(path), Verdict::Sensitive, "{path} must be sensitive");
        }
    }

    #[test]
    fn secret_bearing_file_names_are_sensitive_anywhere() {
        for path in [
            "/srv/app/.env",
            "/srv/app/.env.production",
            "/srv/app/service-account-credentials.json",
            "/srv/app/api_token.txt",
            "/srv/app/client_secret.json",
            "/srv/certs/server.pem",
            "/srv/certs/server.key",
            "/home/user/vault.kdbx",
            "/home/user/.netrc",
        ] {
            assert_eq!(verdict(path), Verdict::Sensitive, "{path} must be sensitive");
        }
    }

    #[test]
    fn browser_profiles_are_sensitive_because_they_hold_session_cookies() {
        for path in [
            "/home/user/.mozilla/firefox/abc.default/cookies.sqlite",
            "/home/user/.config/BraveSoftware/Brave-Browser/Default/Login Data",
            "/home/user/.config/google-chrome/Default/Cookies",
        ] {
            assert_eq!(verdict(path), Verdict::Sensitive, "{path} must be sensitive");
        }
    }

    #[test]
    fn build_output_is_indexed_but_never_read() {
        for path in [
            "/home/user/proj/node_modules/react/index.js",
            "/home/user/proj/target/debug/binary",
            "/home/user/proj/__pycache__/mod.pyc",
            "/home/user/proj/.git/objects/ab/cdef",
            "/home/user/proj/lib/libfoo.so",
        ] {
            let v = verdict(path);
            assert_eq!(v, Verdict::MetadataOnly, "{path} should be metadata-only");
            assert!(!v.may_read_content(), "{path} must never be opened");
            assert!(v.may_index(), "{path} should still be findable by path");
        }
    }

    #[test]
    fn ordinary_documents_are_allowed() {
        for path in [
            "/home/user/Documents/report.pdf",
            "/home/user/notes/2026-08-17.md",
            "/home/user/proj/src/main.rs",
            "/home/user/Pictures/holiday.jpg",
        ] {
            assert_eq!(verdict(path), Verdict::Allow, "{path} should be allowed");
        }
    }

    #[test]
    fn sensitive_wins_over_bulky() {
        // A key inside node_modules is still a key. Order of checks matters here and it is
        // exactly the kind of thing a refactor silently inverts.
        let v = verdict("/home/user/proj/node_modules/pkg/private.pem");
        assert_eq!(v, Verdict::Sensitive);
    }

    #[test]
    fn matching_is_case_insensitive_for_names() {
        assert_eq!(verdict("/home/user/SECRET_notes.txt"), Verdict::Sensitive);
        assert_eq!(verdict("/home/user/certs/SERVER.PEM"), Verdict::Sensitive);
    }

    #[test]
    fn a_dotfile_name_is_not_read_as_an_extension() {
        // `.env` must be caught by name. If it were parsed as "extension env" the check would
        // also have to be duplicated, and one of the two copies would eventually drift.
        assert_eq!(extension_of(".env"), None);
        assert_eq!(extension_of("archive.tar.gz"), Some("gz"));
        assert_eq!(extension_of("noextension"), None);
    }

    #[test]
    fn extra_patterns_can_only_tighten() {
        let path = PathBuf::from("/home/user/Documents/taxes-2026.ods");
        assert_eq!(classify(&path, &[]), Verdict::Allow);
        assert_eq!(
            classify(&path, &["*.ods".to_string()]),
            Verdict::Sensitive,
            "a user pattern must be able to exclude a file type"
        );
        assert_eq!(
            classify(&path, &["Documents".to_string()]),
            Verdict::Sensitive,
            "a user pattern must be able to exclude a directory"
        );
    }

    #[test]
    fn extra_patterns_cannot_loosen_a_built_in() {
        // There is no mechanism to allow something the built-in list denies, and there must
        // not be one — an accidental config edit must never expose private keys.
        let path = PathBuf::from("/home/user/.ssh/id_ed25519");
        assert_eq!(classify(&path, &["allow-everything".to_string()]), Verdict::Sensitive);
    }

    #[test]
    fn whole_subtrees_are_skippable() {
        for dir in ["node_modules", ".git", "target", ".cache", ".ssh"] {
            assert!(should_skip_subtree(dir), "{dir} should be skippable");
        }
        for dir in ["Documents", "src", "photos"] {
            assert!(!should_skip_subtree(dir), "{dir} must not be skipped");
        }
    }

    #[test]
    fn every_skippable_directory_also_classifies_as_at_least_metadata_only() {
        // If a walk skips a subtree but the classifier would have allowed reading it, then the
        // two disagree and behaviour depends on which code path reached the file first.
        for dir in BULKY_DIRS.iter().chain(SENSITIVE_DIRS.iter()) {
            let path = PathBuf::from(format!("/home/user/{dir}/somefile.txt"));
            assert_ne!(
                classify(&path, &[]),
                Verdict::Allow,
                "{dir} is skipped during walks but classify() would allow reading it"
            );
        }
    }
}
