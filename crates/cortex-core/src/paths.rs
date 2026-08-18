//! Where the daemon keeps its things.
//!
//! Rule R4 from `docs/05-HARD-RULES.md`: the index lives in `$XDG_STATE_HOME`, the directory is
//! `0700` and the files are `0600`. Never in a document folder, never in `/tmp`.

use crate::error::{Error, Result};
use etcetera::BaseStrategy;
use std::path::{Path, PathBuf};

/// Directory name used under every XDG base directory.
pub const APP_DIR: &str = "hippocampus";

/// Resolved locations for one instance of the daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    config_dir: PathBuf,
    state_dir: PathBuf,
    runtime_dir: PathBuf,
}

impl Paths {
    /// Resolve from the environment, honouring `XDG_*` variables.
    pub fn from_env() -> Result<Self> {
        let strategy = etcetera::choose_base_strategy().map_err(|_| Error::NoHome)?;

        let config_dir = strategy.config_dir().join(APP_DIR);
        let state_dir = strategy.state_dir().unwrap_or_else(|| strategy.data_dir()).join(APP_DIR);

        // XDG_RUNTIME_DIR is the correct home for a socket: it is user-private, on tmpfs, and
        // cleaned up at logout. When it is missing (a plain SSH session, a container) we fall
        // back to the state directory rather than to /tmp, which is world-writable.
        let runtime_dir = match std::env::var_os("XDG_RUNTIME_DIR") {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir).join(APP_DIR),
            _ => state_dir.join("run"),
        };

        Ok(Self {
            config_dir,
            state_dir,
            runtime_dir,
        })
    }

    /// Build explicitly. Used by tests and by `--state-dir` style overrides.
    pub fn new(config_dir: PathBuf, state_dir: PathBuf, runtime_dir: PathBuf) -> Self {
        Self {
            config_dir,
            state_dir,
            runtime_dir,
        }
    }

    /// Put every directory under one root. Only for tests and for throwaway instances.
    pub fn rooted_at(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref();
        Self {
            config_dir: root.join("config"),
            state_dir: root.join("state"),
            runtime_dir: root.join("run"),
        }
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn plugins_dir(&self) -> PathBuf {
        self.config_dir.join("plugins")
    }

    /// The single SQLite file that holds everything authoritative.
    pub fn database(&self) -> PathBuf {
        self.state_dir.join("cortex.db")
    }

    /// The ANN index. Rebuildable from the database, so it is safe to delete.
    pub fn vector_index(&self) -> PathBuf {
        self.state_dir.join("vectors.usearch")
    }

    pub fn models_dir(&self) -> PathBuf {
        self.state_dir.join("models")
    }

    pub fn audit_log(&self) -> PathBuf {
        self.state_dir.join("audit.log")
    }

    /// Where content displaced by an agent waits.
    ///
    /// A trashed file is not a copy in the index — rule R10 forbids the index holding content —
    /// it is the original, moved. That is what makes an overwrite undoable without the database
    /// growing by the size of every file an agent touches.
    pub fn trash_dir(&self) -> PathBuf {
        self.state_dir.join("trash")
    }

    pub fn socket(&self) -> PathBuf {
        self.runtime_dir.join("cortex.sock")
    }

    pub fn lock_file(&self) -> PathBuf {
        self.runtime_dir.join("cortex.lock")
    }

    /// Create the directories the daemon owns, with restrictive permissions.
    ///
    /// Existing directories are left in place but their mode is tightened — a state directory
    /// that became group-readable at some point is a finding, not something to work around.
    pub fn ensure_dirs(&self) -> Result<()> {
        for dir in [&self.config_dir, &self.state_dir, &self.runtime_dir] {
            create_private_dir(dir)?;
        }
        create_private_dir(&self.models_dir())?;
        create_private_dir(&self.trash_dir())?;
        Ok(())
    }
}

fn create_private_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path).map_err(|e| Error::io(path, e))?;
    set_mode(path, 0o700)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(mode);
    std::fs::set_permissions(path, perms).map_err(|e| Error::io(path, e))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

/// Tighten a file that the daemon created to `0600`.
///
/// SQLite creates its database and WAL files itself, so this is applied after the fact rather
/// than through the creating call.
pub fn restrict_file(path: &Path) -> Result<()> {
    if path.exists() {
        set_mode(path, 0o600)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn ensure_dirs_creates_private_directories() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::rooted_at(tmp.path());
        paths.ensure_dirs().unwrap();

        for dir in [paths.config_dir(), paths.state_dir(), paths.runtime_dir()] {
            assert!(dir.is_dir(), "{} was not created", dir.display());
            let mode = std::fs::metadata(dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "{} has mode {mode:o}, expected 700", dir.display());
        }
    }

    #[cfg(unix)]
    #[test]
    fn ensure_dirs_tightens_a_directory_that_was_too_open() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::rooted_at(tmp.path());
        std::fs::create_dir_all(paths.state_dir()).unwrap();
        std::fs::set_permissions(paths.state_dir(), std::fs::Permissions::from_mode(0o755)).unwrap();

        paths.ensure_dirs().unwrap();

        let mode = std::fs::metadata(paths.state_dir()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "an over-permissive state directory must be tightened");
    }

    #[test]
    fn ensure_dirs_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::rooted_at(tmp.path());
        paths.ensure_dirs().unwrap();
        paths.ensure_dirs().expect("running twice must not fail");
    }

    #[test]
    fn every_artefact_lives_under_a_directory_we_own() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::rooted_at(tmp.path());
        for artefact in [
            paths.database(),
            paths.vector_index(),
            paths.audit_log(),
            paths.socket(),
            paths.lock_file(),
            paths.config_file(),
        ] {
            assert!(
                artefact.starts_with(tmp.path()),
                "{} escaped the instance root",
                artefact.display()
            );
        }
    }
}
