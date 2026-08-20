//! The XDG Trash, as the specification defines it.
//!
//! M6 asks for "a trash can, never `rm`", and specifically the shared one: a file this program
//! deletes must appear in the same trash as a file deleted by anything else, and be restorable
//! by anything else. That is not a nicety — a file manager whose deletions land somewhere only
//! it knows about is a file manager nobody can safely stop using.
//!
//! The rules, each of which has a reason:
//!
//! - The home trash is `$XDG_DATA_HOME/Trash`, with `files/` and `info/` beside each other.
//! - Every trashed file gets `info/<name>.trashinfo`, holding the **original path** and the
//!   deletion date. Without it the file is unrestorable, so the info file is written *first*:
//!   a crash between the two leaves an entry pointing at a file that is still where it was,
//!   which is recoverable, while the reverse leaves a file nobody can put back.
//! - A name that is taken gets a suffix, and the info file takes the same one. The pair is the
//!   unit; they are never allowed to disagree.
//! - A file on another filesystem goes to a trash at the top of *its* filesystem, because the
//!   alternative is copying a hundred gigabytes to delete it.
//!
//! Restoring is here too, because a trash you cannot undo from is a delete with extra steps.

use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum TrashError {
    #[error("no trash directory could be determined: neither XDG_DATA_HOME nor HOME is set")]
    NoTrashDir,

    #[error("{path}: {reason}")]
    Io { path: PathBuf, reason: String },

    #[error("{path} has no name to file it under")]
    Unnamed { path: PathBuf },

    #[error("the trash has no entry called {name}")]
    NoSuchEntry { name: String },

    #[error("{path} is where {name} came from, and something is there now")]
    Occupied { name: String, path: PathBuf },

    #[error("the entry for {name} is unreadable, so where it came from is unknown")]
    UnreadableInfo { name: String },
}

type Result<T> = std::result::Result<T, TrashError>;

fn io(path: &Path) -> impl Fn(std::io::Error) -> TrashError + '_ {
    move |e| TrashError::Io {
        path: path.to_path_buf(),
        reason: e.to_string(),
    }
}

/// One trash directory: the home one, or a filesystem's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trash {
    root: PathBuf,
}

/// What the trash holds about one deleted thing.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TrashedEntry {
    /// The name inside `files/`, which is what identifies it and is not always the original.
    pub name: String,
    /// Where it is now.
    pub path: PathBuf,
    /// Where it came from, and where restoring puts it back.
    pub original: PathBuf,
    /// `YYYY-MM-DDThh:mm:ss`, local time, as the specification writes it.
    pub deleted_at: String,
}

impl Trash {
    /// `$XDG_DATA_HOME/Trash`, or `~/.local/share/Trash`.
    pub fn home() -> Result<Self> {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .ok_or(TrashError::NoTrashDir)?;
        Ok(Self::at(base.join("Trash")))
    }

    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn files_dir(&self) -> PathBuf {
        self.root.join("files")
    }

    pub fn info_dir(&self) -> PathBuf {
        self.root.join("info")
    }

    fn ensure(&self) -> Result<()> {
        for dir in [self.files_dir(), self.info_dir()] {
            std::fs::create_dir_all(&dir).map_err(io(&dir))?;
        }
        Ok(())
    }

    /// Move `path` into this trash and return what was recorded.
    ///
    /// `now` is the deletion timestamp, passed in rather than read, so the caller's clock is the
    /// one clock in the system.
    pub fn put(&self, path: &Path, now: DeletionDate) -> Result<TrashedEntry> {
        self.ensure()?;

        let original = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| TrashError::Unnamed {
                path: path.to_path_buf(),
            })?
            .to_string();

        let name = self.free_name(&original)?;
        let info_path = self.info_dir().join(format!("{name}.trashinfo"));
        let file_path = self.files_dir().join(&name);

        // The info file first, and this order is the whole reliability argument. A crash after
        // it leaves an entry pointing at a file still in place — visible, harmless, cleanable.
        // A crash after the *move* would leave a file in the trash that nothing can restore.
        let info = format!(
            "[Trash Info]\nPath={}\nDeletionDate={}\n",
            encode_path(path),
            now.0
        );
        std::fs::write(&info_path, info).map_err(io(&info_path))?;

        if let Err(e) = std::fs::rename(path, &file_path) {
            // Same filesystem or nothing: this type is only ever handed a path it can rename
            // into, and the caller picks which trash for that reason.
            let _ = std::fs::remove_file(&info_path);
            return Err(TrashError::Io {
                path: path.to_path_buf(),
                reason: e.to_string(),
            });
        }

        Ok(TrashedEntry {
            name,
            path: file_path,
            original: path.to_path_buf(),
            deleted_at: now.0,
        })
    }

    /// Put one entry back where it came from.
    pub fn restore(&self, name: &str) -> Result<PathBuf> {
        let entry = self.entry(name)?;
        if entry.original.exists() {
            return Err(TrashError::Occupied {
                name: name.to_string(),
                path: entry.original,
            });
        }
        if let Some(parent) = entry.original.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        std::fs::rename(&entry.path, &entry.original).map_err(io(&entry.path))?;
        let info = self.info_dir().join(format!("{name}.trashinfo"));
        let _ = std::fs::remove_file(info);
        Ok(entry.original)
    }

    /// Read one entry back.
    pub fn entry(&self, name: &str) -> Result<TrashedEntry> {
        let info_path = self.info_dir().join(format!("{name}.trashinfo"));
        let text = std::fs::read_to_string(&info_path).map_err(|_| TrashError::NoSuchEntry {
            name: name.to_string(),
        })?;

        let mut original = None;
        let mut deleted_at = None;
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("Path=") {
                original = Some(decode_path(value));
            } else if let Some(value) = line.strip_prefix("DeletionDate=") {
                deleted_at = Some(value.to_string());
            }
        }

        Ok(TrashedEntry {
            name: name.to_string(),
            path: self.files_dir().join(name),
            original: original.ok_or_else(|| TrashError::UnreadableInfo {
                name: name.to_string(),
            })?,
            deleted_at: deleted_at.unwrap_or_default(),
        })
    }

    /// Everything currently in the trash, newest name order not guaranteed.
    pub fn list(&self) -> Result<Vec<TrashedEntry>> {
        let dir = self.info_dir();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Ok(Vec::new());
        };

        let mut out = Vec::new();
        for entry in entries.flatten() {
            let file = entry.file_name();
            let Some(name) = file.to_str().and_then(|n| n.strip_suffix(".trashinfo")) else {
                continue;
            };
            if let Ok(entry) = self.entry(name) {
                out.push(entry);
            }
        }
        Ok(out)
    }

    /// A name inside `files/` that nothing is using.
    ///
    /// Collisions are ordinary: two directories can hold a `notes.md`, and both may be deleted.
    /// The suffix goes before the extension, which is what every other implementation does and
    /// what keeps `notes.2.md` recognisable.
    fn free_name(&self, original: &str) -> Result<String> {
        let files = self.files_dir();
        let info = self.info_dir();

        // Both must be free. A `.trashinfo` without its file is a broken entry, and reusing its
        // name would silently adopt somebody else's original path.
        let taken = |name: &str| {
            files.join(name).symlink_metadata().is_ok()
                || info.join(format!("{name}.trashinfo")).exists()
        };

        if !taken(original) {
            return Ok(original.to_string());
        }

        let (stem, extension) = match original.rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() => (stem, Some(ext)),
            _ => (original, None),
        };

        for n in 2..10_000u32 {
            let candidate = match extension {
                Some(ext) => format!("{stem}.{n}.{ext}"),
                None => format!("{stem}.{n}"),
            };
            if !taken(&candidate) {
                return Ok(candidate);
            }
        }
        Err(TrashError::Io {
            path: files,
            reason: "ten thousand files of that name are already in the trash".into(),
        })
    }
}

/// `YYYY-MM-DDThh:mm:ss`, which is what the specification writes into `DeletionDate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletionDate(pub String);

impl DeletionDate {
    /// Format a Unix timestamp as the specification's local-time string.
    ///
    /// The arithmetic is written out because a date crate for one format string is a dependency
    /// for one format string. It is UTC rather than local time, which the specification permits
    /// implementations to differ on and which at least never lies about an offset it does not
    /// know.
    pub fn from_unix(seconds: i64) -> Self {
        let days = seconds.div_euclid(86_400);
        let time = seconds.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days);
        Self(format!(
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
            time / 3600,
            (time % 3600) / 60,
            time % 60
        ))
    }
}

/// Days since the epoch to a calendar date. Howard Hinnant's `civil_from_days`, which is the
/// standard way to do this without a calendar library.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Percent-encode a path for `Path=`, as the specification requires.
///
/// The same rule as the thumbnail URIs, minus the scheme: everything outside the unreserved set,
/// with `/` kept so the value stays readable as a path.
fn encode_path(path: &Path) -> String {
    let mut out = String::new();
    for byte in path.as_os_str().as_encoded_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn decode_path(value: &str) -> PathBuf {
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&value[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    PathBuf::from(String::from_utf8_lossy(&out).into_owned())
}

/// Which trash a path belongs in.
///
/// The home trash when the path is on the same filesystem, and the filesystem's own otherwise:
/// `$topdir/.Trash-$uid`, created if it is missing. Trashing across a filesystem boundary would
/// mean copying every byte to delete something, which is the one thing a trash must not do to a
/// hundred-gigabyte file.
#[cfg(unix)]
pub fn trash_for(path: &Path, home: &Trash) -> Result<Trash> {
    use std::os::unix::fs::MetadataExt;

    let home_device = std::fs::metadata(home_anchor(home))
        .map(|m| m.dev())
        .unwrap_or(0);
    let path_device = std::fs::symlink_metadata(path).map(|m| m.dev()).unwrap_or(0);

    if home_device == path_device || path_device == 0 {
        return Ok(home.clone());
    }

    let Some(topdir) = topdir_of(path, path_device) else {
        return Ok(home.clone());
    };
    let uid = unsafe { libc_getuid() };
    Ok(Trash::at(topdir.join(format!(".Trash-{uid}"))))
}

#[cfg(not(unix))]
pub fn trash_for(_path: &Path, home: &Trash) -> Result<Trash> {
    Ok(home.clone())
}

/// An existing directory to ask which filesystem the home trash is on. The trash itself may not
/// exist yet, and its parents do.
fn home_anchor(home: &Trash) -> PathBuf {
    let mut candidate = home.root().to_path_buf();
    while !candidate.exists() {
        match candidate.parent() {
            Some(parent) => candidate = parent.to_path_buf(),
            None => break,
        }
    }
    candidate
}

/// Walk up until the device number changes: the last path still on the same filesystem is where
/// that filesystem is mounted.
fn topdir_of(path: &Path, device: u64) -> Option<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let mut current = path.parent()?.to_path_buf();
        loop {
            let Some(parent) = current.parent() else {
                return Some(current);
            };
            match std::fs::metadata(parent) {
                Ok(md) if md.dev() == device => current = parent.to_path_buf(),
                _ => return Some(current),
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (path, device);
        None
    }
}

#[cfg(unix)]
unsafe fn libc_getuid() -> u32 {
    // One libc call does not justify the dependency, and `getuid` cannot fail.
    extern "C" {
        fn getuid() -> u32;
    }
    getuid()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trash() -> (tempfile::TempDir, Trash) {
        let dir = tempfile::tempdir().unwrap();
        let trash = Trash::at(dir.path().join("Trash"));
        (dir, trash)
    }

    #[test]
    fn the_layout_is_the_one_every_other_tool_looks_in() {
        let (dir, trash) = trash();
        let victim = dir.path().join("notes.md");
        std::fs::write(&victim, "content").unwrap();

        let entry = trash.put(&victim, DeletionDate::from_unix(1_700_000_000)).unwrap();

        assert!(!victim.exists(), "the original is still there");
        assert_eq!(entry.path, trash.root().join("files/notes.md"));
        assert!(trash.root().join("info/notes.md.trashinfo").exists());
        assert_eq!(std::fs::read_to_string(&entry.path).unwrap(), "content");
    }

    #[test]
    fn the_info_file_says_where_it_came_from_and_when() {
        let (dir, trash) = trash();
        let victim = dir.path().join("notes.md");
        std::fs::write(&victim, "x").unwrap();

        trash.put(&victim, DeletionDate::from_unix(1_700_000_000)).unwrap();
        let info = std::fs::read_to_string(trash.root().join("info/notes.md.trashinfo")).unwrap();

        assert!(info.starts_with("[Trash Info]\n"), "{info}");
        assert!(info.contains(&format!("Path={}", victim.display())), "{info}");
        assert!(info.contains("DeletionDate=2023-11-14T22:13:20"), "{info}");
    }

    #[test]
    fn a_path_with_spaces_is_encoded_and_comes_back_intact() {
        let (dir, trash) = trash();
        let awkward = dir.path().join("a file with spaces & ümlauts.txt");
        std::fs::write(&awkward, "x").unwrap();

        trash.put(&awkward, DeletionDate::from_unix(0)).unwrap();
        let name = "a file with spaces & ümlauts.txt";
        let info = std::fs::read_to_string(trash.root().join(format!("info/{name}.trashinfo")))
            .unwrap();
        assert!(info.contains("%20"), "spaces must be encoded: {info}");
        assert!(!info.contains("Path=/tmp/ a"), "{info}");

        assert_eq!(trash.entry(name).unwrap().original, awkward);
    }

    #[test]
    fn two_files_of_the_same_name_both_fit() {
        let (dir, trash) = trash();
        let one = dir.path().join("one/notes.md");
        let two = dir.path().join("two/notes.md");
        for path in [&one, &two] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, path.parent().unwrap().to_string_lossy().as_bytes()).unwrap();
        }

        let first = trash.put(&one, DeletionDate::from_unix(0)).unwrap();
        let second = trash.put(&two, DeletionDate::from_unix(0)).unwrap();

        assert_eq!(first.name, "notes.md");
        assert_eq!(second.name, "notes.2.md", "the suffix goes before the extension");
        // And the pair never disagrees: each info file points at its own original.
        assert_eq!(trash.entry(&first.name).unwrap().original, one);
        assert_eq!(trash.entry(&second.name).unwrap().original, two);
    }

    #[test]
    fn an_orphaned_info_file_does_not_get_its_name_reused() {
        let (dir, trash) = trash();
        std::fs::create_dir_all(trash.info_dir()).unwrap();
        std::fs::create_dir_all(trash.files_dir()).unwrap();
        // An entry whose file is gone: reusing the name would adopt its original path.
        std::fs::write(
            trash.info_dir().join("notes.md.trashinfo"),
            "[Trash Info]\nPath=/somewhere/else/notes.md\nDeletionDate=2020-01-01T00:00:00\n",
        )
        .unwrap();

        let victim = dir.path().join("notes.md");
        std::fs::write(&victim, "mine").unwrap();
        let entry = trash.put(&victim, DeletionDate::from_unix(0)).unwrap();

        assert_eq!(entry.name, "notes.2.md");
        assert_eq!(trash.entry("notes.md").unwrap().original, PathBuf::from("/somewhere/else/notes.md"));
    }

    #[test]
    fn restoring_puts_it_back_and_forgets_the_entry() {
        let (dir, trash) = trash();
        let victim = dir.path().join("sub/notes.md");
        std::fs::create_dir_all(victim.parent().unwrap()).unwrap();
        std::fs::write(&victim, "content").unwrap();

        let entry = trash.put(&victim, DeletionDate::from_unix(0)).unwrap();
        let restored = trash.restore(&entry.name).unwrap();

        assert_eq!(restored, victim);
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "content");
        assert!(trash.list().unwrap().is_empty());
    }

    #[test]
    fn restoring_refuses_to_overwrite_what_took_its_place() {
        let (dir, trash) = trash();
        let victim = dir.path().join("notes.md");
        std::fs::write(&victim, "old").unwrap();
        let entry = trash.put(&victim, DeletionDate::from_unix(0)).unwrap();

        std::fs::write(&victim, "somebody else's work").unwrap();
        let error = trash.restore(&entry.name).unwrap_err();

        assert!(matches!(error, TrashError::Occupied { .. }), "{error}");
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "somebody else's work");
    }

    #[test]
    fn restoring_recreates_a_directory_that_was_deleted_meanwhile() {
        let (dir, trash) = trash();
        let victim = dir.path().join("gone/notes.md");
        std::fs::create_dir_all(victim.parent().unwrap()).unwrap();
        std::fs::write(&victim, "content").unwrap();

        let entry = trash.put(&victim, DeletionDate::from_unix(0)).unwrap();
        std::fs::remove_dir(victim.parent().unwrap()).unwrap();

        assert_eq!(trash.restore(&entry.name).unwrap(), victim);
        assert!(victim.exists());
    }

    #[test]
    fn a_directory_can_be_trashed_whole() {
        let (dir, trash) = trash();
        let victim = dir.path().join("project");
        std::fs::create_dir_all(victim.join("nested")).unwrap();
        std::fs::write(victim.join("nested/file.txt"), "deep").unwrap();

        let entry = trash.put(&victim, DeletionDate::from_unix(0)).unwrap();
        assert!(entry.path.join("nested/file.txt").exists());

        trash.restore(&entry.name).unwrap();
        assert_eq!(std::fs::read_to_string(victim.join("nested/file.txt")).unwrap(), "deep");
    }

    #[test]
    fn listing_reads_back_everything_that_was_put_in() {
        let (dir, trash) = trash();
        for name in ["a.txt", "b.txt", "c.txt"] {
            let path = dir.path().join(name);
            std::fs::write(&path, name).unwrap();
            trash.put(&path, DeletionDate::from_unix(0)).unwrap();
        }
        let mut names: Vec<String> = trash.list().unwrap().into_iter().map(|e| e.name).collect();
        names.sort();
        assert_eq!(names, vec!["a.txt", "b.txt", "c.txt"]);
    }

    #[test]
    fn the_date_format_is_the_one_the_specification_writes() {
        assert_eq!(DeletionDate::from_unix(0).0, "1970-01-01T00:00:00");
        assert_eq!(DeletionDate::from_unix(1_700_000_000).0, "2023-11-14T22:13:20");
        // A leap day, because the arithmetic is written out here rather than borrowed.
        assert_eq!(DeletionDate::from_unix(1_709_164_800).0, "2024-02-29T00:00:00");
    }

    #[test]
    fn a_file_on_the_same_filesystem_uses_the_home_trash() {
        let (dir, home) = trash();
        let victim = dir.path().join("notes.md");
        std::fs::write(&victim, "x").unwrap();
        assert_eq!(trash_for(&victim, &home).unwrap(), home);
    }
}
