//! Single-instance enforcement.
//!
//! Two daemons against one database is not merely wasteful — with WAL and two independent
//! watchers it produces duplicated episodes and interleaved writes that look like corruption
//! long after the fact. The lock is held for the process lifetime and released by the kernel
//! when the process dies, so a crash never leaves a stale lock behind.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error("another cortexd is already running (lock held on {path}{})", match pid {
        Some(p) => format!(", pid {p}"),
        None => String::new(),
    })]
    AlreadyHeld { path: String, pid: Option<i32> },

    #[error("could not create the lock file at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// An advisory lock held for as long as this value lives.
#[derive(Debug)]
pub struct InstanceLock {
    _file: File,
    path: PathBuf,
}

impl InstanceLock {
    /// Acquire the lock, failing immediately if another process holds it.
    pub fn acquire(path: &Path) -> Result<Self, LockError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| LockError::Io {
                path: parent.display().to_string(),
                source,
            })?;
        }

        let mut file = File::options()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)
            .map_err(|source| LockError::Io {
                path: path.display().to_string(),
                source,
            })?;

        try_lock(&file).map_err(|_| LockError::AlreadyHeld {
            path: path.display().to_string(),
            pid: read_pid(path),
        })?;

        // Record our pid for diagnostics. This is informational only — the lock itself is what
        // enforces exclusivity, so a stale pid in the file is harmless.
        let _ = file.set_len(0);
        let _ = write!(file, "{}", std::process::id());
        let _ = file.flush();

        Ok(Self {
            _file: file,
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        // Best effort. If this does not run — a SIGKILL, say — the kernel still drops the
        // lock when the file descriptor closes, so the next start is not blocked.
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(unix)]
fn try_lock(file: &File) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;
    // SAFETY: `fd` is owned by `file` and valid for the duration of this call.
    let rc = unsafe { libc_flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn try_lock(_file: &File) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
const LOCK_EX: i32 = 2;
#[cfg(unix)]
const LOCK_NB: i32 = 4;

#[cfg(unix)]
extern "C" {
    #[link_name = "flock"]
    fn libc_flock(fd: i32, operation: i32) -> i32;
}

fn read_pid(path: &Path) -> Option<i32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lock_can_be_acquired_and_records_our_pid() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cortex.lock");
        let lock = InstanceLock::acquire(&path).unwrap();

        assert!(path.exists());
        assert_eq!(read_pid(&path), Some(std::process::id() as i32));
        drop(lock);
    }

    #[test]
    fn a_second_acquisition_in_the_same_process_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cortex.lock");
        let _held = InstanceLock::acquire(&path).unwrap();

        let err = InstanceLock::acquire(&path).unwrap_err();
        assert!(
            matches!(err, LockError::AlreadyHeld { .. }),
            "expected a refusal, got {err:?}"
        );
    }

    #[test]
    fn the_error_message_names_the_holder() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cortex.lock");
        let _held = InstanceLock::acquire(&path).unwrap();

        let message = InstanceLock::acquire(&path).unwrap_err().to_string();
        assert!(message.contains("already running"), "unhelpful message: {message}");
        assert!(
            message.contains(&std::process::id().to_string()),
            "the message should name the holding pid: {message}"
        );
    }

    #[test]
    fn releasing_the_lock_allows_a_new_one() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cortex.lock");

        drop(InstanceLock::acquire(&path).unwrap());
        InstanceLock::acquire(&path).expect("the lock must be reusable after release");
    }

    #[test]
    fn a_leftover_lock_file_from_a_dead_process_does_not_block_startup() {
        // A crash leaves the file behind but not the kernel lock. Refusing to start because a
        // file exists would mean every hard shutdown needs manual cleanup.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cortex.lock");
        std::fs::write(&path, "999999").unwrap();

        InstanceLock::acquire(&path).expect("a stale lock file must not block startup");
    }

    #[test]
    fn missing_parent_directories_are_created() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("run").join("nested").join("cortex.lock");
        let _lock = InstanceLock::acquire(&path).unwrap();
        assert!(path.exists());
    }
}
