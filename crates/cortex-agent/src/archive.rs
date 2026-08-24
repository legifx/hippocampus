//! Unpacking an archive, with everything the surface owes something that writes files.
//!
//! An extraction is a transfer that reads its sources out of one file instead of off the disk:
//! it takes as long, it wants the same progress bar, it has to stop when asked, and it must
//! leave nothing behind when it does. So it reuses [`crate::transfer`]'s handle, measure and
//! progress rather than growing a second vocabulary for the same ideas, and the daemon reports
//! it on the same two notifications.
//!
//! What is *not* shared is the trust model. Every path in a transfer came from the filesystem
//! and was checked by the policy before anything moved. Every path in an archive came from
//! whoever built the archive, which may be nobody the person unpacking it has ever met. Three
//! things follow, and they are the reason this module exists rather than a call to `bsdtar`:
//!
//! 1. **An entry may not leave the destination.** Absolute paths, `..` and anything else that
//!    resolves outside are refused — by the plan, so the refusal happens before a byte is
//!    written rather than in the middle of a directory tree.
//! 2. **Links are refused, not followed.** A symbolic link inside an archive is a path chosen
//!    by the archive pointing anywhere it likes; a hard link is worse. Neither is unpacked, and
//!    the entry is reported as refused rather than silently skipped.
//! 3. **A header is a claim, not a fact.** Both formats state an uncompressed size. It is used
//!    for the progress bar, and enforced while writing: an entry that produces more than it
//!    declared is stopped at the line it crosses, which is what a decompression bomb looks like
//!    from in here.

use crate::fs_tools::{InverseStep, Surface, ToolError, ToolResult};
use crate::transfer::{Handle, Measure, OnConflict, Progress};
use cortex_core::model::Action;
use cortex_core::tools::{
    ArchiveEntry, ArchiveFormat, ConflictChoice, FsKind, PlanAction, PlanStep, TransferFailure,
};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// How many entries a dry run lists.
///
/// A listing is for a person to look at before they press the button. An archive of a hundred
/// thousand files answered with a hundred thousand rows would cost the daemon a large JSON
/// document to build and the window a long list nobody reads to render — while the totals,
/// which are what the question is actually about, are already there.
pub const LISTING_CAP: usize = 500;

/// What a dry run answers, and what the extraction is then handed.
#[derive(Debug)]
pub struct ExtractPlan {
    pub archive: PathBuf,
    pub format: ArchiveFormat,
    /// Where the entries land, with the conflict choice already applied.
    pub destination: PathBuf,
    /// Whether something was already at the destination the caller asked for.
    pub occupied: bool,
    pub total: Measure,
    pub entries: Vec<ArchiveEntry>,
    /// True when `entries` is a prefix of the archive rather than all of it.
    pub truncated: bool,
}

/// What an extraction did.
#[derive(Debug, Default)]
pub struct ExtractOutcome {
    pub destination: PathBuf,
    pub files: u64,
    pub bytes: u64,
    /// Entries that were not unpacked, each with the reason. A link, a device node or an entry
    /// that would have escaped is reported here rather than dropped: an archive that quietly
    /// unpacks to fewer files than it contains is worse than one that says why.
    pub refused: Vec<TransferFailure>,
    pub cancelled: bool,
    /// The journal entry that removes what was written. `None` when nothing was.
    pub undo_id: Option<i64>,
}

/// What the file says it is.
///
/// The magic bytes decide, and the extension is only consulted when there are none to read —
/// a `.zip` that is really a tarball is a mislabelled file, not a reason to fail.
pub fn format_of(path: &Path) -> Option<ArchiveFormat> {
    if let Some(format) = format_from_magic(path) {
        return Some(format);
    }
    let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    if name.ends_with(".zip") {
        Some(ArchiveFormat::Zip)
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        Some(ArchiveFormat::TarGz)
    } else if name.ends_with(".tar") {
        Some(ArchiveFormat::Tar)
    } else {
        None
    }
}

fn format_from_magic(path: &Path) -> Option<ArchiveFormat> {
    use std::io::{Read as _, Seek as _, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 4];
    if file.read_exact(&mut head).is_err() {
        return None;
    }
    if head == [0x50, 0x4b, 0x03, 0x04] || head == [0x50, 0x4b, 0x05, 0x06] {
        return Some(ArchiveFormat::Zip);
    }
    if head[0] == 0x1f && head[1] == 0x8b {
        return Some(ArchiveFormat::TarGz);
    }
    // A tar has no header of its own; the format is announced 257 bytes in, inside the first
    // entry's header, which is why an empty file cannot be told from a truncated one here.
    let mut magic = [0u8; 5];
    file.seek(SeekFrom::Start(257)).ok()?;
    if file.read_exact(&mut magic).is_ok() && &magic == b"ustar" {
        return Some(ArchiveFormat::Tar);
    }
    None
}

/// The directory an archive unpacks into when nobody named one: a new one beside it, called
/// what the archive is called.
///
/// Unpacking into the current directory is how a "tar bomb" scatters forty files over somebody's
/// home directory, and even a well-made archive is a surprise when it does it. The default is
/// the safe one; a caller that wants the other behaviour asks for it by name.
pub fn default_destination(archive: &Path) -> PathBuf {
    let parent = archive.parent().unwrap_or(Path::new("/"));
    let name = archive.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let stem = strip_archive_suffix(&name);
    // A name with nothing to strip would otherwise unpack the archive into itself — which is
    // not hypothetical: an archive identified by its magic bytes need not be called `.zip` at
    // all, and the first one that was not is how this was found.
    if stem.is_empty() || stem == name {
        return parent.join(format!("{name}-extracted"));
    }
    parent.join(stem)
}

/// Strip the compound suffix rather than the last one: `notes.tar.gz` unpacks into `notes`, not
/// into `notes.tar`. An extension this list does not know is still stripped — `bundle.bin`
/// unpacks into `bundle` — because the magic bytes have already decided what the file is.
fn strip_archive_suffix(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    for suffix in [".tar.gz", ".tar.bz2", ".tar.xz", ".tgz", ".tar", ".zip"] {
        if lower.ends_with(suffix) {
            return name[..name.len() - suffix.len()].to_string();
        }
    }
    match name.rfind('.') {
        // A leading dot is the whole name of a hidden file, not an extension.
        Some(at) if at > 0 => name[..at].to_string(),
        _ => name.to_string(),
    }
}

/// Where an entry may land, or `None` if it may not land anywhere.
///
/// Rejected: absolute paths, anything containing `..`, a Windows prefix, and the empty path.
/// `.` components are dropped rather than refused — they are noise, not an escape.
pub fn safe_target(base: &Path, name: &Path) -> Option<PathBuf> {
    let mut out = base.to_path_buf();
    let mut any = false;
    for component in name.components() {
        match component {
            Component::Normal(part) => {
                out.push(part);
                any = true;
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    any.then_some(out)
}

/// One entry, as both the listing and the extraction see it.
struct Item {
    path: PathBuf,
    kind: FsKind,
    size: u64,
    /// Set when the entry is something this build will not write. The listing shows it; the
    /// extraction reports it and moves on.
    refusal: Option<String>,
}

impl Item {
    fn refused(name: String, reason: &str) -> Self {
        Item {
            path: PathBuf::from(name),
            kind: FsKind::File,
            size: 0,
            refusal: Some(reason.to_string()),
        }
    }
}

/// Read an archive's table of contents without unpacking it.
///
/// For a zip this reads the central directory and costs nothing. For a tarball there is no
/// central directory to read: the whole stream is walked, and for a `.tar.gz` that means
/// decompressing it once to count and once to unpack. That is the price of a progress bar with
/// a denominator, and it is paid deliberately — a spinner that cannot say how far along it is
/// was the thing this was supposed to replace.
fn read_entries(archive: &Path, format: ArchiveFormat) -> std::io::Result<Vec<Item>> {
    match format {
        ArchiveFormat::Zip => read_zip_entries(archive),
        ArchiveFormat::Tar | ArchiveFormat::TarGz => read_tar_entries(archive, format),
    }
}

fn read_zip_entries(archive: &Path) -> std::io::Result<Vec<Item>> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file).map_err(zip_io)?;
    let mut items = Vec::with_capacity(zip.len());

    for index in 0..zip.len() {
        let entry = zip.by_index(index).map_err(zip_io)?;
        let raw = entry.name().to_string();

        // The unix mode says what the entry is. A zip written on a system without one has no
        // mode at all, in which case "is it a directory" is decided by the trailing slash,
        // which is the convention the format actually has.
        if let Some(mode) = entry.unix_mode() {
            const KIND: u32 = 0o170000;
            const SYMLINK: u32 = 0o120000;
            const REGULAR: u32 = 0o100000;
            const DIRECTORY: u32 = 0o040000;
            let kind = mode & KIND;
            if kind == SYMLINK {
                items.push(Item::refused(raw, "is a symbolic link"));
                continue;
            }
            if kind != 0 && kind != REGULAR && kind != DIRECTORY {
                items.push(Item::refused(raw, "is not a file or a directory"));
                continue;
            }
        }

        let Some(path) = entry.enclosed_name() else {
            items.push(Item::refused(raw, "would be written outside the destination"));
            continue;
        };

        items.push(Item {
            path,
            kind: if entry.is_dir() { FsKind::Dir } else { FsKind::File },
            size: if entry.is_dir() { 0 } else { entry.size() },
            refusal: None,
        });
    }
    Ok(items)
}

fn read_tar_entries(archive: &Path, format: ArchiveFormat) -> std::io::Result<Vec<Item>> {
    let mut items = Vec::new();
    with_tar(archive, format, |mut tar| {
        for entry in tar.entries()? {
            let entry = entry?;
            let raw = entry.path()?.display().to_string();
            let header = entry.header();

            let kind = match header.entry_type() {
                tar::EntryType::Regular | tar::EntryType::Continuous => FsKind::File,
                tar::EntryType::Directory => FsKind::Dir,
                tar::EntryType::Symlink => {
                    items.push(Item::refused(raw, "is a symbolic link"));
                    continue;
                }
                tar::EntryType::Link => {
                    items.push(Item::refused(raw, "is a hard link"));
                    continue;
                }
                // Long-name and extended-header entries are the format talking to itself; the
                // `tar` crate has already folded them into the entry that follows.
                tar::EntryType::GNULongName
                | tar::EntryType::GNULongLink
                | tar::EntryType::XHeader
                | tar::EntryType::XGlobalHeader => continue,
                _ => {
                    items.push(Item::refused(raw, "is not a file or a directory"));
                    continue;
                }
            };

            let path = entry.path()?.to_path_buf();
            if safe_target(Path::new("/base"), &path).is_none() {
                items.push(Item::refused(raw, "would be written outside the destination"));
                continue;
            }

            items.push(Item {
                path,
                kind,
                size: if kind == FsKind::Dir { 0 } else { header.size().unwrap_or(0) },
                refusal: None,
            });
        }
        Ok(())
    })?;
    Ok(items)
}

/// Open a tarball, decompressed if it needs to be, and hand it to `f`.
///
/// Written as a callback because the two cases have different types all the way down and a
/// boxed `dyn Read` would be a trait object in the inner loop of every extraction.
fn with_tar<F>(archive: &Path, format: ArchiveFormat, mut f: F) -> std::io::Result<()>
where
    F: FnMut(tar::Archive<Box<dyn Read>>) -> std::io::Result<()>,
{
    let file = std::fs::File::open(archive)?;
    let reader: Box<dyn Read> = match format {
        ArchiveFormat::TarGz => Box::new(flate2::read::GzDecoder::new(file)),
        _ => Box::new(file),
    };
    f(tar::Archive::new(reader))
}

fn zip_io(e: zip::result::ZipError) -> std::io::Error {
    match e {
        zip::result::ZipError::Io(io) => io,
        other => std::io::Error::new(std::io::ErrorKind::InvalidData, other.to_string()),
    }
}

fn measure_of(items: &[Item]) -> Measure {
    let mut total = Measure::default();
    for item in items.iter().filter(|i| i.refusal.is_none()) {
        match item.kind {
            FsKind::Dir => total.directories += 1,
            _ => {
                total.files += 1;
                total.bytes += item.size;
            }
        }
    }
    total
}

impl Surface<'_> {
    /// Check an archive before any of it is written, and say what is inside.
    ///
    /// Separate from doing it for the same two reasons a transfer's plan is: the progress bar
    /// needs its denominator before the first byte, and a refusal on policy grounds has to
    /// happen while nothing has been written yet.
    pub fn plan_extract(
        &self,
        archive: &Path,
        destination: Option<&Path>,
        conflict: ConflictChoice,
    ) -> ToolResult<ExtractPlan> {
        let tool = "fs.extract";
        let args = serde_json::json!({
            "archive": archive.display().to_string(),
            "destination": destination.map(|d| d.display().to_string()),
        });
        self.gate_args(tool, &args)?;

        // Reading the archive is a read; writing its contents is a write. Both are checked,
        // because an archive an agent may not read is not made readable by unpacking it.
        let archive = match self.policy.check_read(archive) {
            Ok(path) => path,
            Err(r) => return self.refuse_public(tool, &args, r),
        };

        let Some(format) = format_of(&archive) else {
            return Err(ToolError::Io {
                path: archive,
                message: "is not a zip or a tar this build can unpack".into(),
            });
        };

        let wanted = destination
            .map(|d| d.to_path_buf())
            .unwrap_or_else(|| default_destination(&archive));
        let wanted = match self.policy.check_write(&wanted, &self.actor) {
            Ok(path) => path,
            Err(r) => return self.refuse_public(tool, &args, r),
        };

        let occupied = std::fs::symlink_metadata(&wanted).is_ok();
        let destination = resolve_destination(&wanted, conflict).map_err(|message| ToolError::Io {
            path: wanted.clone(),
            message,
        })?;

        let items = read_entries(&archive, format).map_err(|e| ToolError::io(&archive, e))?;
        let total = measure_of(&items);
        let truncated = items.len() > LISTING_CAP;
        let entries = items
            .iter()
            .take(LISTING_CAP)
            .map(|item| ArchiveEntry {
                path: item.path.clone(),
                kind: item.kind,
                size: item.size,
            })
            .collect();

        Ok(ExtractPlan {
            archive,
            format,
            destination,
            occupied,
            total,
            entries,
            truncated,
        })
    }

    /// Unpack an archive that `plan_extract` has already checked.
    ///
    /// Blocking, like a transfer, and for the same reason: this is `read` and `write` syscalls
    /// for as long as it takes.
    pub fn run_extract(
        &self,
        archive: &Path,
        destination: &Path,
        format: ArchiveFormat,
        total: Measure,
        handle: &Handle,
        mut on_progress: impl FnMut(&Progress),
    ) -> ExtractOutcome {
        let tool = "fs.extract";
        let args = serde_json::json!({
            "archive": archive.display().to_string(),
            "destination": destination.display().to_string(),
        });

        let mut written = Written::default();
        let mut outcome = ExtractOutcome {
            destination: destination.to_path_buf(),
            ..ExtractOutcome::default()
        };
        let mut progress = Progress {
            total,
            files_done: 0,
            bytes_done: 0,
            current: None,
        };

        let result = self.unpack(
            archive,
            destination,
            format,
            handle,
            &mut written,
            &mut progress,
            &mut on_progress,
            &mut outcome,
        );

        match result {
            Ok(()) if !outcome.cancelled => {
                outcome.files = progress.files_done;
                outcome.bytes = progress.bytes_done;
                // The journal entry is written after the fact rather than before, unlike every
                // single-path tool here. A single-path tool knows what it will touch; an
                // extraction finds out as it goes, and an entry written first would describe an
                // undo for files that a cancellation means never existed.
                outcome.undo_id = self.journal_extract(archive, destination, &written).ok();
                let undone = outcome.undo_id.unwrap_or(-1);
                let _ = self.record_episode(
                    Action::Create,
                    destination,
                    serde_json::json!({
                        "undo": undone,
                        "archive": archive.display().to_string(),
                        "files": outcome.files,
                    }),
                );
                self.auditor.allowed(
                    self.now,
                    tool,
                    &args,
                    &format!(
                        "{} files, {} refused, undo {undone}",
                        outcome.files,
                        outcome.refused.len()
                    ),
                );
            }
            Ok(()) => {
                // Cancelled. What was written is not what was asked for, and half a tree that
                // looks like a finished one is the failure mode this removes.
                written.remove_all();
                self.auditor
                    .allowed(self.now, tool, &args, "cancelled, nothing kept");
            }
            Err(message) => {
                written.remove_all();
                self.auditor
                    .allowed(self.now, tool, &args, &format!("failed: {message}"));
                outcome.refused.push(TransferFailure {
                    source: archive.to_path_buf(),
                    reason: message,
                });
            }
        }
        outcome
    }

    #[allow(clippy::too_many_arguments)]
    fn unpack(
        &self,
        archive: &Path,
        destination: &Path,
        format: ArchiveFormat,
        handle: &Handle,
        written: &mut Written,
        progress: &mut Progress,
        on_progress: &mut impl FnMut(&Progress),
        outcome: &mut ExtractOutcome,
    ) -> Result<(), String> {
        written.create_dir(destination).map_err(|e| e.to_string())?;

        match format {
            ArchiveFormat::Zip => {
                let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
                let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
                let items = read_zip_entries(archive).map_err(|e| e.to_string())?;

                for (index, item) in items.iter().enumerate() {
                    if handle.is_cancelled() {
                        outcome.cancelled = true;
                        return Ok(());
                    }
                    if let Some(reason) = &item.refusal {
                        outcome.refused.push(TransferFailure {
                            source: item.path.clone(),
                            reason: reason.clone(),
                        });
                        continue;
                    }
                    let Some(target) = safe_target(destination, &item.path) else {
                        outcome.refused.push(TransferFailure {
                            source: item.path.clone(),
                            reason: "would be written outside the destination".into(),
                        });
                        continue;
                    };
                    if item.kind == FsKind::Dir {
                        written.create_dir(&target).map_err(|e| e.to_string())?;
                        continue;
                    }
                    let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
                    written.parents_of(&target).map_err(|e| e.to_string())?;
                    match written.write_file(&target, &mut entry, item.size, handle, progress, on_progress) {
                        Ok(true) => {}
                        Ok(false) => {
                            outcome.cancelled = true;
                            return Ok(());
                        }
                        Err(e) => {
                            outcome.refused.push(TransferFailure {
                                source: item.path.clone(),
                                reason: e.to_string(),
                            });
                        }
                    }
                }
                Ok(())
            }
            ArchiveFormat::Tar | ArchiveFormat::TarGz => {
                let mut failure: Option<String> = None;
                let result = with_tar(archive, format, |mut tar| {
                    for entry in tar.entries()? {
                        let mut entry = entry?;
                        if handle.is_cancelled() {
                            outcome.cancelled = true;
                            return Ok(());
                        }
                        let raw = entry.path()?.to_path_buf();
                        let size = entry.header().size().unwrap_or(0);
                        let kind = match entry.header().entry_type() {
                            tar::EntryType::Regular | tar::EntryType::Continuous => FsKind::File,
                            tar::EntryType::Directory => FsKind::Dir,
                            tar::EntryType::GNULongName
                            | tar::EntryType::GNULongLink
                            | tar::EntryType::XHeader
                            | tar::EntryType::XGlobalHeader => continue,
                            tar::EntryType::Symlink => {
                                outcome.refused.push(TransferFailure {
                                    source: raw,
                                    reason: "is a symbolic link".into(),
                                });
                                continue;
                            }
                            tar::EntryType::Link => {
                                outcome.refused.push(TransferFailure {
                                    source: raw,
                                    reason: "is a hard link".into(),
                                });
                                continue;
                            }
                            _ => {
                                outcome.refused.push(TransferFailure {
                                    source: raw,
                                    reason: "is not a file or a directory".into(),
                                });
                                continue;
                            }
                        };

                        let Some(target) = safe_target(destination, &raw) else {
                            outcome.refused.push(TransferFailure {
                                source: raw,
                                reason: "would be written outside the destination".into(),
                            });
                            continue;
                        };

                        if kind == FsKind::Dir {
                            written.create_dir(&target)?;
                            continue;
                        }
                        written.parents_of(&target)?;
                        match written.write_file(&target, &mut entry, size, handle, progress, on_progress) {
                            Ok(true) => {}
                            Ok(false) => {
                                outcome.cancelled = true;
                                return Ok(());
                            }
                            Err(e) => outcome.refused.push(TransferFailure {
                                source: raw,
                                reason: e.to_string(),
                            }),
                        }
                    }
                    Ok(())
                });
                if let Err(e) = result {
                    failure = Some(e.to_string());
                }
                match failure {
                    Some(message) => Err(message),
                    None => Ok(()),
                }
            }
        }
    }

    /// Record how to remove what an extraction wrote.
    ///
    /// One journal entry per extraction, holding one inverse step per path created. That makes
    /// the row large for a large archive — a thousand files is roughly a hundred kilobytes of
    /// JSON — and the alternative, a single "delete the destination recursively" step, was
    /// rejected: it would remove whatever the person had put in that directory in the meantime.
    /// `Delete` skips a file that is already gone and `RemoveDir` refuses a directory that is no
    /// longer empty, so an undo takes back exactly what this wrote and nothing else.
    fn journal_extract(
        &self,
        archive: &Path,
        destination: &Path,
        written: &Written,
    ) -> ToolResult<i64> {
        let steps = vec![PlanStep {
            action: PlanAction::Create,
            path: destination.to_path_buf(),
            from: Some(archive.to_path_buf()),
            replaces: None,
        }];

        // Forward order: the directories as they were made, then the files. `apply_inverse`
        // walks it backwards, so the files go first and the directories unwind deepest last.
        let mut inverse: Vec<InverseStep> = written
            .dirs
            .iter()
            .map(|path| InverseStep::RemoveDir { path: path.clone() })
            .collect();
        inverse.extend(
            written
                .files
                .iter()
                .map(|path| InverseStep::Delete { path: path.clone() }),
        );

        self.journal("fs.extract", &steps, &inverse)
    }
}

/// Everything an extraction has created, in the order it created it.
///
/// Two jobs: it is the cleanup list when the extraction is cancelled or fails, and it is the
/// inverse when it succeeds. Keeping one list for both is what makes those two agree.
#[derive(Debug, Default)]
struct Written {
    dirs: Vec<PathBuf>,
    files: Vec<PathBuf>,
}

impl Written {
    /// Create a directory, remembering it only if it was not already there. A directory that
    /// existed before is not ours to remove.
    fn create_dir(&mut self, path: &Path) -> std::io::Result<()> {
        if path.is_dir() {
            return Ok(());
        }
        std::fs::create_dir_all(path)?;
        self.dirs.push(path.to_path_buf());
        Ok(())
    }

    fn parents_of(&mut self, target: &Path) -> std::io::Result<()> {
        if let Some(parent) = target.parent() {
            self.create_dir(parent)?;
        }
        Ok(())
    }

    /// Write one entry. `Ok(false)` means the handle was cancelled part-way through.
    fn write_file(
        &mut self,
        target: &Path,
        source: &mut impl Read,
        declared: u64,
        handle: &Handle,
        progress: &mut Progress,
        on_progress: &mut impl FnMut(&Progress),
    ) -> std::io::Result<bool> {
        use std::io::Write;

        let existed = target.exists();
        let mut file = std::fs::File::create(target)?;
        if !existed {
            self.files.push(target.to_path_buf());
        }
        progress.current = Some(target.to_path_buf());

        let mut buffer = vec![0u8; 64 * 1024];
        let mut done: u64 = 0;
        loop {
            if handle.is_cancelled() {
                return Ok(false);
            }
            let read = source.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            done += read as u64;
            // The header said how big this is. More than that means the header lied, and the
            // difference between a mislabelled file and a decompression bomb is only how much
            // more — so it stops at the line rather than at some multiple of it.
            if declared > 0 && done > declared {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("unpacks to more than the {declared} bytes it declares"),
                ));
            }
            file.write_all(&buffer[..read])?;
            progress.bytes_done += read as u64;
            on_progress(progress);
        }
        file.flush()?;
        progress.files_done += 1;
        on_progress(progress);
        Ok(true)
    }

    /// Undo everything, for a cancellation or a failure. Files first, then directories from the
    /// deepest — the same order `apply_inverse` runs in, for the same reason.
    fn remove_all(&self) {
        for path in self.files.iter().rev() {
            let _ = std::fs::remove_file(path);
        }
        for path in self.dirs.iter().rev() {
            let _ = std::fs::remove_dir(path);
        }
    }
}

/// Apply the conflict choice to the destination directory.
///
/// `Replace` unpacks *into* the directory that is there, overwriting only what collides. A
/// transfer's `Replace` removes the thing it replaces; doing that here would delete whatever
/// else the person had put in that directory, which is not what replacing an archive's contents
/// means to anybody.
///
/// `Skip` is refused rather than reinterpreted. For a transfer it means "leave the one that is
/// there and count it as done", which for an extraction would be a call that unpacks nothing and
/// reports success — the one answer a person cannot act on.
fn resolve_destination(wanted: &Path, conflict: ConflictChoice) -> Result<PathBuf, String> {
    if std::fs::symlink_metadata(wanted).is_err() {
        return Ok(wanted.to_path_buf());
    }
    match OnConflict::from(conflict) {
        OnConflict::Fail if conflict == ConflictChoice::Skip => {
            Err("skip has no meaning for an extraction: fail, replace or keep".into())
        }
        OnConflict::Fail => Err("already exists".into()),
        OnConflict::Replace => Ok(wanted.to_path_buf()),
        OnConflict::Keep => Ok(crate::transfer::free_name(wanted)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_compound_suffix_is_stripped_whole() {
        assert_eq!(
            default_destination(Path::new("/tmp/notes.tar.gz")),
            PathBuf::from("/tmp/notes")
        );
        assert_eq!(
            default_destination(Path::new("/tmp/notes.zip")),
            PathBuf::from("/tmp/notes")
        );
        assert_eq!(
            default_destination(Path::new("/tmp/report.2026.tar")),
            PathBuf::from("/tmp/report.2026")
        );
        // Identified by its bytes rather than by its name: the extension is still stripped.
        assert_eq!(
            default_destination(Path::new("/tmp/bundle.bin")),
            PathBuf::from("/tmp/bundle")
        );
        // And a name with nothing to strip must not unpack the archive into itself.
        assert_eq!(
            default_destination(Path::new("/tmp/bundle")),
            PathBuf::from("/tmp/bundle-extracted")
        );
    }

    #[test]
    fn nothing_lands_outside_the_destination() {
        let base = Path::new("/tmp/out");
        assert_eq!(
            safe_target(base, Path::new("a/b.txt")),
            Some(PathBuf::from("/tmp/out/a/b.txt"))
        );
        assert_eq!(safe_target(base, Path::new("../escape")), None);
        assert_eq!(safe_target(base, Path::new("a/../../escape")), None);
        assert_eq!(safe_target(base, Path::new("/etc/passwd")), None);
        assert_eq!(safe_target(base, Path::new("")), None);
        // `.` is noise rather than an escape.
        assert_eq!(
            safe_target(base, Path::new("./a.txt")),
            Some(PathBuf::from("/tmp/out/a.txt"))
        );
    }

    #[test]
    fn the_magic_bytes_decide_before_the_extension() {
        let dir = tempfile::tempdir().unwrap();
        let lying = dir.path().join("actually-a-zip.tar");
        std::fs::write(&lying, [0x50, 0x4b, 0x03, 0x04, 0, 0, 0, 0]).unwrap();
        assert_eq!(format_of(&lying), Some(ArchiveFormat::Zip));

        let nothing = dir.path().join("notes.txt");
        std::fs::write(&nothing, b"not an archive").unwrap();
        assert_eq!(format_of(&nothing), None);
    }
}
