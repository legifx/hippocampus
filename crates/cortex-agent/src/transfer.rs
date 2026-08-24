//! Copying and moving, with progress and a way out.
//!
//! M6 §4 is the whole specification of this module: *"a copy of 10 GB across filesystems shows
//! accurate progress, can be cancelled, and leaves no partial file behind on cancellation."*
//! Each clause is a design constraint rather than a feature:
//!
//! - **Accurate progress** means the total is known before the first byte moves, so a tree is
//!   measured first. That walk costs a stat per file and is the only way a percentage is not a
//!   guess.
//! - **Can be cancelled** means the check is inside the byte loop, not between files. A single
//!   ten-gigabyte file must stop within a chunk, not after it.
//! - **Leaves no partial file** means writing to a temporary name in the destination directory
//!   and renaming when the bytes are all there. A rename within a directory is atomic, so an
//!   interrupted copy is either absent or complete — never a plausible-looking half.
//!
//! The same-filesystem move is a rename and reports one instant step, because pretending to
//! copy a hundred gigabytes that the kernel moved by rewriting a directory entry would be a
//! progress bar that lies in the other direction.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// How much is read at a time. Large enough that the syscall overhead disappears, small enough
/// that cancelling a copy feels immediate.
const CHUNK: usize = 256 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("{path}: {reason}")]
    Io { path: PathBuf, reason: String },

    #[error("cancelled")]
    Cancelled,

    #[error("{path} already exists")]
    Exists { path: PathBuf },

    #[error("{from} cannot be copied into itself ({to})")]
    IntoItself { from: PathBuf, to: PathBuf },
}

type Result<T> = std::result::Result<T, TransferError>;

fn io(path: &Path, e: std::io::Error) -> TransferError {
    TransferError::Io {
        path: path.to_path_buf(),
        reason: e.to_string(),
    }
}

/// What a transfer is going to move, known before it starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Measure {
    pub files: u64,
    pub directories: u64,
    pub bytes: u64,
}

/// How far along it is.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Progress {
    pub total: Measure,
    pub files_done: u64,
    pub bytes_done: u64,
    /// What is being worked on right now, so a person can see it is not stuck.
    pub current: Option<PathBuf>,
}

impl Progress {
    /// Zero to one. Bytes rather than file count: a thousand small files and one large one are
    /// not the same job, and counting files makes the bar sprint and then stall.
    pub fn fraction(&self) -> f64 {
        if self.total.bytes == 0 {
            // A tree of empty files still finishes; measure it by files so it does not sit at
            // zero and then jump to one.
            if self.total.files == 0 {
                return 1.0;
            }
            return self.files_done as f64 / self.total.files as f64;
        }
        (self.bytes_done as f64 / self.total.bytes as f64).min(1.0)
    }
}

/// The handle a caller keeps: the counters to read, and the switch to stop it.
#[derive(Debug, Clone, Default)]
pub struct Handle {
    cancelled: Arc<AtomicBool>,
    files_done: Arc<AtomicU64>,
    bytes_done: Arc<AtomicU64>,
}

impl Handle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the transfer to stop. It stops inside the current chunk and cleans up after itself.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    pub fn files_done(&self) -> u64 {
        self.files_done.load(Ordering::Relaxed)
    }

    pub fn bytes_done(&self) -> u64 {
        self.bytes_done.load(Ordering::Relaxed)
    }
}

/// What to do when the destination is taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnConflict {
    /// Stop and say so. The default, because the alternatives both lose something.
    Fail,
    /// Replace it. The caller is expected to have asked somebody first.
    Replace,
    /// Copy alongside as `name (2).ext`, which is what a file manager offers when a person
    /// wants both.
    Keep,
}

impl Default for OnConflict {
    fn default() -> Self {
        OnConflict::Fail
    }
}

/// Walk a tree and add it up, before anything is moved.
///
/// Symbolic links are counted as themselves and never followed: a link into a parent directory
/// would otherwise make this walk forever, and copying a link's target instead of the link is a
/// different operation from the one that was asked for.
pub fn measure(path: &Path) -> Result<Measure> {
    let mut total = Measure::default();
    let mut stack = vec![path.to_path_buf()];

    while let Some(current) = stack.pop() {
        let md = std::fs::symlink_metadata(&current).map_err(|e| io(&current, e))?;
        if md.is_symlink() {
            total.files += 1;
        } else if md.is_dir() {
            total.directories += 1;
            let entries = std::fs::read_dir(&current).map_err(|e| io(&current, e))?;
            for entry in entries.flatten() {
                stack.push(entry.path());
            }
        } else {
            total.files += 1;
            total.bytes += md.len();
        }
    }
    Ok(total)
}

/// Copy `from` to `to`, reporting progress and stopping when asked.
///
/// `on_progress` is called per chunk and per file. It is a callback rather than a channel so
/// that this module does not decide what progress *is for* — the daemon turns it into
/// notifications, a test turns it into a list.
pub fn copy(
    from: &Path,
    to: &Path,
    conflict: OnConflict,
    handle: &Handle,
    total: Measure,
    mut on_progress: impl FnMut(&Progress),
) -> Result<PathBuf> {
    if to.starts_with(from) {
        return Err(TransferError::IntoItself {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
        });
    }

    let destination = resolve_conflict(to, conflict)?;

    // Seeded from the handle rather than from zero. A batch calls this once per source, and a
    // progress struct that restarted each time would drive a bar that resets on every file —
    // and, worse, a caller watching `files_done` would never see it pass one. That is exactly
    // how the batch cancellation test failed before this line existed.
    let mut progress = Progress {
        total,
        files_done: handle.files_done(),
        bytes_done: handle.bytes_done(),
        current: None,
    };

    let outcome = copy_into(from, &destination, handle, &mut progress, &mut on_progress);

    if outcome.is_err() {
        // Whatever was written is not what was asked for. A cancelled copy that left half a
        // tree behind would be indistinguishable from a finished one at a glance.
        remove_whatever_it_is(&destination);
    }
    outcome.map(|_| destination)
}

fn copy_into(
    from: &Path,
    to: &Path,
    handle: &Handle,
    progress: &mut Progress,
    on_progress: &mut impl FnMut(&Progress),
) -> Result<()> {
    if handle.is_cancelled() {
        return Err(TransferError::Cancelled);
    }

    let md = std::fs::symlink_metadata(from).map_err(|e| io(from, e))?;

    if md.is_symlink() {
        let target = std::fs::read_link(from).map_err(|e| io(from, e))?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, to).map_err(|e| io(to, e))?;
        progress.files_done += 1;
        handle.files_done.fetch_add(1, Ordering::Relaxed);
        on_progress(progress);
        return Ok(());
    }

    if md.is_dir() {
        std::fs::create_dir_all(to).map_err(|e| io(to, e))?;
        let entries = std::fs::read_dir(from).map_err(|e| io(from, e))?;
        for entry in entries.flatten() {
            let name = entry.file_name();
            copy_into(&entry.path(), &to.join(name), handle, progress, on_progress)?;
        }
        // The mode last, so a directory being written into is not read-only while it is.
        copy_permissions(&md, to);
        return Ok(());
    }

    copy_file(from, to, &md, handle, progress, on_progress)
}

fn copy_file(
    from: &Path,
    to: &Path,
    md: &std::fs::Metadata,
    handle: &Handle,
    progress: &mut Progress,
    on_progress: &mut impl FnMut(&Progress),
) -> Result<()> {
    progress.current = Some(from.to_path_buf());

    // A temporary in the destination's own directory: a rename within one directory is atomic,
    // so the file appears complete or not at all. A temporary in `/tmp` would make the final
    // step a copy across filesystems, which is the thing being avoided.
    let temporary = to.with_file_name(format!(
        ".{}.hippocampus-part",
        to.file_name().and_then(|n| n.to_str()).unwrap_or("file")
    ));

    let result = (|| -> Result<()> {
        let mut source = std::fs::File::open(from).map_err(|e| io(from, e))?;
        let mut sink = std::fs::File::create(&temporary).map_err(|e| io(&temporary, e))?;
        let mut buffer = vec![0u8; CHUNK];

        loop {
            // Inside the loop, not between files: a single ten-gigabyte file has to stop within
            // a chunk rather than at the end of itself.
            if handle.is_cancelled() {
                return Err(TransferError::Cancelled);
            }
            let read = source.read(&mut buffer).map_err(|e| io(from, e))?;
            if read == 0 {
                break;
            }
            sink.write_all(&buffer[..read]).map_err(|e| io(&temporary, e))?;

            progress.bytes_done += read as u64;
            handle.bytes_done.fetch_add(read as u64, Ordering::Relaxed);
            on_progress(progress);
        }

        sink.flush().map_err(|e| io(&temporary, e))?;
        Ok(())
    })();

    if let Err(e) = result {
        let _ = std::fs::remove_file(&temporary);
        return Err(e);
    }

    copy_permissions(md, &temporary);
    std::fs::rename(&temporary, to).map_err(|e| io(to, e))?;

    progress.files_done += 1;
    handle.files_done.fetch_add(1, Ordering::Relaxed);
    on_progress(progress);
    Ok(())
}

/// Move, which is a rename when it can be and a copy-then-delete when it cannot.
///
/// The source is removed only after every byte has arrived. A move that deleted first and
/// failed second would be the worst outcome this module can produce.
pub fn move_to(
    from: &Path,
    to: &Path,
    conflict: OnConflict,
    handle: &Handle,
    total: Measure,
    on_progress: impl FnMut(&Progress),
) -> Result<PathBuf> {
    let destination = resolve_conflict(to, conflict)?;

    // The cheap path. A rename moves a directory tree of any size by rewriting one entry, and
    // reporting byte progress for it would be a bar that lies in the other direction.
    if std::fs::rename(from, &destination).is_ok() {
        return Ok(destination);
    }

    let copied = copy(from, &destination, OnConflict::Fail, handle, total, on_progress)?;
    remove_whatever_it_is(from);
    Ok(copied)
}

fn resolve_conflict(to: &Path, conflict: OnConflict) -> Result<PathBuf> {
    if std::fs::symlink_metadata(to).is_err() {
        return Ok(to.to_path_buf());
    }
    match conflict {
        OnConflict::Fail => Err(TransferError::Exists {
            path: to.to_path_buf(),
        }),
        OnConflict::Replace => {
            remove_whatever_it_is(to);
            Ok(to.to_path_buf())
        }
        OnConflict::Keep => Ok(free_name(to)),
    }
}

/// `notes.md` becomes `notes (2).md` — the form a person recognises from every file manager,
/// with the number before the extension so the file still opens with the right program.
pub(crate) fn free_name(taken: &Path) -> PathBuf {
    let stem = taken.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let extension = taken.extension().and_then(|e| e.to_str());

    for n in 2..10_000u32 {
        let name = match extension {
            Some(ext) => format!("{stem} ({n}).{ext}"),
            None => format!("{stem} ({n})"),
        };
        let candidate = taken.with_file_name(name);
        if std::fs::symlink_metadata(&candidate).is_err() {
            return candidate;
        }
    }
    taken.to_path_buf()
}

fn remove_whatever_it_is(path: &Path) {
    match std::fs::symlink_metadata(path) {
        Ok(md) if md.is_dir() && !md.is_symlink() => {
            let _ = std::fs::remove_dir_all(path);
        }
        Ok(_) => {
            let _ = std::fs::remove_file(path);
        }
        Err(_) => {}
    }
}

#[cfg(unix)]
fn copy_permissions(from: &std::fs::Metadata, to: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(to, std::fs::Permissions::from_mode(from.permissions().mode()));
}

#[cfg(not(unix))]
fn copy_permissions(_from: &std::fs::Metadata, _to: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(root: &Path) {
        std::fs::create_dir_all(root.join("sub/deeper")).unwrap();
        std::fs::write(root.join("a.txt"), vec![b'a'; 1000]).unwrap();
        std::fs::write(root.join("sub/b.txt"), vec![b'b'; 2000]).unwrap();
        std::fs::write(root.join("sub/deeper/c.txt"), vec![b'c'; 3000]).unwrap();
    }

    #[test]
    fn a_tree_is_measured_before_anything_moves() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        tree(&source);

        let total = measure(&source).unwrap();
        assert_eq!(total.files, 3);
        assert_eq!(total.directories, 3, "source, sub and deeper");
        assert_eq!(total.bytes, 6000);
    }

    #[test]
    fn a_copy_arrives_complete_and_reports_as_it_goes() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let target = dir.path().join("target");
        tree(&source);

        let total = measure(&source).unwrap();
        let handle = Handle::new();
        let mut reports = Vec::new();
        copy(&source, &target, OnConflict::Fail, &handle, total, |p| {
            reports.push(p.fraction())
        })
        .unwrap();

        assert_eq!(std::fs::read(target.join("sub/deeper/c.txt")).unwrap().len(), 3000);
        assert!(!reports.is_empty());
        // Monotone and finishing at one: a bar that goes backwards is worse than none.
        assert!(reports.windows(2).all(|w| w[1] >= w[0]), "{reports:?}");
        assert_eq!(reports.last().copied(), Some(1.0));
        assert_eq!(handle.bytes_done(), 6000);
        assert_eq!(handle.files_done(), 3);
    }

    #[test]
    fn cancelling_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("big.bin");
        // Several chunks, so there is a middle to be cancelled in.
        std::fs::write(&source, vec![0u8; CHUNK * 8]).unwrap();
        let target = dir.path().join("copy.bin");

        let handle = Handle::new();
        let cancel_at = handle.clone();
        let total = measure(&source).unwrap();

        let error = copy(&source, &target, OnConflict::Fail, &handle, total, |p| {
            if p.bytes_done > CHUNK as u64 * 2 {
                cancel_at.cancel();
            }
        })
        .unwrap_err();

        assert!(matches!(error, TransferError::Cancelled), "{error}");
        assert!(!target.exists(), "a cancelled copy left a file behind");
        // And no temporary either — a `.part` file is still litter.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            !leftovers.iter().any(|n| n.contains("hippocampus-part")),
            "{leftovers:?}"
        );
    }

    #[test]
    fn cancelling_a_tree_removes_the_part_that_arrived() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        tree(&source);
        let target = dir.path().join("target");

        let handle = Handle::new();
        let cancel_at = handle.clone();
        let total = measure(&source).unwrap();
        let error = copy(&source, &target, OnConflict::Fail, &handle, total, |p| {
            if p.files_done >= 1 {
                cancel_at.cancel();
            }
        })
        .unwrap_err();

        assert!(matches!(error, TransferError::Cancelled));
        assert!(!target.exists(), "half a tree is worse than none: it looks finished");
    }

    #[test]
    fn an_existing_destination_is_refused_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("a.txt");
        let target = dir.path().join("b.txt");
        std::fs::write(&source, "new").unwrap();
        std::fs::write(&target, "old").unwrap();

        let error = copy(&source, &target, OnConflict::Fail, &Handle::new(), Measure::default(), |_| {})
            .unwrap_err();
        assert!(matches!(error, TransferError::Exists { .. }));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "old");
    }

    #[test]
    fn keeping_both_numbers_the_new_one_before_its_extension() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("notes.md");
        let target = dir.path().join("copy/notes.md");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&source, "new").unwrap();
        std::fs::write(&target, "old").unwrap();

        let total = measure(&source).unwrap();
        let written = copy(&source, &target, OnConflict::Keep, &Handle::new(), total, |_| {}).unwrap();

        assert_eq!(written.file_name().unwrap(), "notes (2).md");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(&written).unwrap(), "new");
    }

    #[test]
    fn replacing_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("a.txt");
        let target = dir.path().join("b.txt");
        std::fs::write(&source, "new").unwrap();
        std::fs::write(&target, "old").unwrap();

        let total = measure(&source).unwrap();
        copy(&source, &target, OnConflict::Replace, &Handle::new(), total, |_| {}).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
    }

    #[test]
    fn a_directory_cannot_be_copied_into_itself() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        tree(&source);

        let error = copy(
            &source,
            &source.join("sub/inside"),
            OnConflict::Fail,
            &Handle::new(),
            Measure::default(),
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(error, TransferError::IntoItself { .. }), "{error}");
    }

    #[test]
    fn a_symbolic_link_is_copied_as_a_link_rather_than_followed() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("real.txt"), "content").unwrap();
        std::os::unix::fs::symlink("real.txt", source.join("link.txt")).unwrap();

        let target = dir.path().join("target");
        let total = measure(&source).unwrap();
        copy(&source, &target, OnConflict::Fail, &Handle::new(), total, |_| {}).unwrap();

        let copied = target.join("link.txt");
        assert!(
            std::fs::symlink_metadata(&copied).unwrap().is_symlink(),
            "the link was followed and copied as a file"
        );
        assert_eq!(std::fs::read_link(&copied).unwrap(), PathBuf::from("real.txt"));
    }

    #[test]
    fn a_move_within_one_filesystem_is_a_rename_and_costs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        tree(&source);
        let target = dir.path().join("moved");

        let mut reports = 0;
        move_to(
            &source,
            &target,
            OnConflict::Fail,
            &Handle::new(),
            measure(&source).unwrap(),
            |_| reports += 1,
        )
        .unwrap();

        assert!(!source.exists());
        assert_eq!(std::fs::read(target.join("sub/deeper/c.txt")).unwrap().len(), 3000);
        assert_eq!(reports, 0, "a rename has no bytes to report");
    }

    #[test]
    fn progress_is_measured_in_bytes_rather_than_files() {
        // One large file and three tiny ones: counting files would show 75 % after the three
        // that took no time, and then sit there.
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("big.bin"), vec![0u8; 400_000]).unwrap();
        for name in ["a", "b", "c"] {
            std::fs::write(source.join(name), "x").unwrap();
        }

        let total = measure(&source).unwrap();
        let mut after_three_small_files = None;
        copy(
            &source,
            &dir.path().join("target"),
            OnConflict::Fail,
            &Handle::new(),
            total,
            |p| {
                if p.files_done == 3 && after_three_small_files.is_none() {
                    after_three_small_files = Some(p.fraction());
                }
            },
        )
        .unwrap();

        let fraction = after_three_small_files.expect("three files were copied");
        assert!(
            fraction < 0.99,
            "three files of one byte reported {fraction} of the work done"
        );
    }

    #[test]
    fn an_empty_tree_finishes_at_one_rather_than_at_zero() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("empty"), "").unwrap();

        let total = measure(&source).unwrap();
        let mut last = 0.0;
        copy(&source, &dir.path().join("target"), OnConflict::Fail, &Handle::new(), total, |p| {
            last = p.fraction()
        })
        .unwrap();
        assert_eq!(last, 1.0);
    }
}
