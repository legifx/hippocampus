//! Where a person starts, as opposed to where the filesystem starts.
//!
//! Opening a file manager at `/` is what happens when nobody decides where to open it. The
//! first screen is then `bin`, `boot`, `dev`, `etc`, `lib`, `lost+found`, `opt`, `proc`,
//! `run`, `sbin`, `srv`, `sys`, `tmp`, `usr`, `var` — sixteen directories, of which one is
//! ever wanted, and it is `home`. Everything a person actually opens is four levels down.
//!
//! So this answers a different question: not "what is in this directory" but "where would you
//! like to go". Four sections, and the technical half of the machine is one row in the last of
//! them:
//!
//! 1. **Persönlich** — the home directory and the XDG user directories that exist.
//! 2. **Lesezeichen** — whatever the person bookmarked, read from the same file Nautilus and
//!    Thunar use, so a bookmark made in either shows up in all three.
//! 3. **Laufwerke** — mounted volumes, with a label and how full they are.
//! 4. **System** — the filesystem root. One row. Everything above is still reachable through it,
//!    and nothing above is in the way of somebody looking for their Downloads.
//!
//! ## Why the daemon builds this and not the interface
//!
//! D4: the frontend does no data processing. Deciding which mounts are volumes and which are
//! kernel plumbing, which directories exist, and what to call them, is exactly the kind of
//! judgement that would otherwise end up implemented twice — once here for the CLI and once in
//! TypeScript for the window — and the two would drift.
//!
//! ## Everything here is read from files, on purpose
//!
//! No udisks2, no D-Bus, no `libblkid`. `/proc/mounts` says what is mounted,
//! `/dev/disk/by-label` says what the volumes are called, `~/.config/user-dirs.dirs` says where
//! Downloads is, and `statvfs` says how full a volume is. The daemon promises no network access
//! and starts in milliseconds; a D-Bus round trip on the path to the first screen would cost
//! both. Mounting and unmounting *do* need udisks2, and that is a separate piece of work.

use cortex_core::tools::{Capacity, Place, PlaceKind, PlaceSection};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Where the mount table lives. A parameter so the tests can supply one.
const PROC_MOUNTS: &str = "/proc/mounts";

/// Where the kernel publishes filesystem labels as symlinks.
const BY_LABEL: &str = "/dev/disk/by-label";

/// Mount points below which a mount is plumbing rather than a place.
///
/// `/boot` holds the bootloader, `/var` holds snap and flatpak loop mounts by the dozen, `/snap`
/// holds more of the same, and none of them is somewhere a person navigates to. They stay
/// reachable through the filesystem root; they are simply not offered.
const NOT_PLACES: &[&str] = &["/boot", "/var", "/snap", "/proc", "/sys", "/dev", "/run/user"];

/// Mount points below which a mount is *always* a place, whatever it is mounted from.
///
/// This is where removable media lands, and a stick formatted as exFAT with a FUSE driver
/// behind it is as much a drive as an internal SSD.
const REMOVABLE_ROOTS: &[&str] = &["/media", "/run/media", "/mnt"];

/// One mounted filesystem, as `/proc/mounts` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub device: String,
    pub point: PathBuf,
    pub fstype: String,
}

/// The home directory, or `/` if the environment does not say.
///
/// Falling back to the root rather than failing: a session with no `HOME` is broken in ways this
/// cannot fix, and one section that says "Dateisystem" is still a usable answer.
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Build the whole list.
///
/// `home` is passed in rather than read from the environment so that this is a function of its
/// inputs and can be tested against a directory that was made for the purpose.
pub fn sections(home: &Path) -> Vec<PlaceSection> {
    let mounts = read_mounts(Path::new(PROC_MOUNTS));
    let labels = read_labels(Path::new(BY_LABEL));
    build(home, &mounts, &labels, &capacity_of)
}

/// The part with no I/O of its own, so that every decision in it is testable.
///
/// `measure` is how a volume's capacity is found; the real one calls `statvfs` and the tests
/// supply numbers.
pub fn build(
    home: &Path,
    mounts: &[Mount],
    labels: &HashMap<String, String>,
    measure: &dyn Fn(&Path) -> Option<Capacity>,
) -> Vec<PlaceSection> {
    let mut sections = Vec::new();

    let personal = personal_places(home);
    if !personal.is_empty() {
        sections.push(PlaceSection {
            title: "Persönlich".to_string(),
            places: personal,
        });
    }

    let marked = bookmarks(home);
    if !marked.is_empty() {
        sections.push(PlaceSection {
            title: "Lesezeichen".to_string(),
            places: marked,
        });
    }

    let drives = drive_places(mounts, labels, measure);
    if !drives.is_empty() {
        sections.push(PlaceSection {
            title: "Laufwerke".to_string(),
            places: drives,
        });
    }

    // Always present, always last, always one row. This is the whole answer to "the first screen
    // is sixteen directories of which one matters".
    sections.push(PlaceSection {
        title: "System".to_string(),
        places: vec![Place {
            path: PathBuf::from("/"),
            name: "Dateisystem".to_string(),
            kind: PlaceKind::Filesystem,
            capacity: measure(Path::new("/")),
            device: root_device(mounts),
        }],
    });

    sections
}

// ---------------------------------------------------------------------------
// Personal directories.
// ---------------------------------------------------------------------------

/// The XDG user directories, in the order a person reaches for them.
///
/// Downloads before Documents: it is where a browser puts things, which is where most of what
/// somebody is looking for arrived twenty minutes ago.
const USER_DIRS: &[(&str, &str, PlaceKind)] = &[
    ("XDG_DOWNLOAD_DIR", "Downloads", PlaceKind::Downloads),
    ("XDG_DOCUMENTS_DIR", "Documents", PlaceKind::Documents),
    ("XDG_DESKTOP_DIR", "Desktop", PlaceKind::Desktop),
    ("XDG_PICTURES_DIR", "Pictures", PlaceKind::Pictures),
    ("XDG_MUSIC_DIR", "Music", PlaceKind::Music),
    ("XDG_VIDEOS_DIR", "Videos", PlaceKind::Videos),
];

fn personal_places(home: &Path) -> Vec<Place> {
    let mut places = vec![Place {
        path: home.to_path_buf(),
        name: "Persönlicher Ordner".to_string(),
        kind: PlaceKind::Home,
        capacity: None,
        device: None,
    }];

    let configured = user_dirs(home);
    for (key, fallback, kind) in USER_DIRS {
        let path = match configured.get(*key) {
            Some(path) => path.clone(),
            None => home.join(fallback),
        };
        // Only what exists. Offering a Videos folder to somebody who has never had one is the
        // kind of clutter this screen is meant to remove, not add.
        if path.is_dir() {
            places.push(Place {
                name: folder_name(&path).unwrap_or_else(|| (*fallback).to_string()),
                path,
                kind: *kind,
                capacity: None,
                device: None,
            });
        }
    }

    let trash = home.join(".local/share/Trash/files");
    if trash.is_dir() {
        places.push(Place {
            path: trash,
            name: "Papierkorb".to_string(),
            kind: PlaceKind::Trash,
            capacity: None,
            device: None,
        });
    }

    places
}

/// Read `~/.config/user-dirs.dirs`, which is where the desktop records that Downloads is
/// actually called `Herunterladen` and lives somewhere else.
///
/// The format is shell: `XDG_DOWNLOAD_DIR="$HOME/Downloads"`. It is *sourced* by the tools that
/// use it, which this does not do — a config file is not a reason to run a shell. `$HOME` is the
/// only expansion the specification allows, so it is the only one performed.
fn user_dirs(home: &Path) -> HashMap<String, PathBuf> {
    let mut dirs = HashMap::new();
    let Ok(text) = std::fs::read_to_string(home.join(".config/user-dirs.dirs")) else {
        return dirs;
    };

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        let path = if let Some(rest) = value.strip_prefix("$HOME/") {
            home.join(rest)
        } else if value == "$HOME" {
            home.to_path_buf()
        } else if value.starts_with('/') {
            PathBuf::from(value)
        } else {
            // A relative path here is a broken file, and guessing what it is relative to would
            // be inventing an answer.
            continue;
        };
        dirs.insert(key.trim().to_string(), path);
    }
    dirs
}

/// The same bookmarks file Nautilus, Nemo and Thunar use.
///
/// Reading it rather than keeping our own means a bookmark made in any of them appears in all of
/// them, which is the whole point of a shared convention. Writing to it is not done here: this
/// program is new on the machine and the file belongs to whatever has been managing it.
///
/// Lines are `file:///path` with an optional display name after a space.
fn bookmarks(home: &Path) -> Vec<Place> {
    let Ok(text) = std::fs::read_to_string(home.join(".config/gtk-3.0/bookmarks")) else {
        return Vec::new();
    };

    let mut places = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (uri, label) = match line.split_once(' ') {
            Some((uri, label)) => (uri, Some(label.trim().to_string())),
            None => (line, None),
        };
        // Anything that is not a local file — `sftp://`, `smb://`, `trash://` — is a network
        // location or a virtual one, and neither is something this can open yet.
        let Some(path) = local_path(uri) else { continue };
        if !path.is_dir() {
            continue;
        }
        places.push(Place {
            name: label
                .or_else(|| folder_name(&path))
                .unwrap_or_else(|| path.display().to_string()),
            path,
            kind: PlaceKind::Bookmark,
            capacity: None,
            device: None,
        });
    }
    places
}

/// `file:///a/b` → `/a/b`, percent-decoded. Anything else is not a place on this machine.
fn local_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    if !rest.starts_with('/') {
        return None;
    }
    Some(PathBuf::from(percent_decode(rest)))
}

// ---------------------------------------------------------------------------
// Volumes.
// ---------------------------------------------------------------------------

fn drive_places(
    mounts: &[Mount],
    labels: &HashMap<String, String>,
    measure: &dyn Fn(&Path) -> Option<Capacity>,
) -> Vec<Place> {
    let mut places = Vec::new();
    let mut seen = Vec::new();

    for mount in mounts {
        if !is_place(mount) {
            continue;
        }
        // The same device mounted twice — a bind mount, or a btrfs subvolume — is one drive.
        // Listing it twice would make the section look like a diagnostic dump.
        if seen.contains(&mount.point) {
            continue;
        }
        seen.push(mount.point.clone());

        places.push(Place {
            name: volume_name(mount, labels),
            path: mount.point.clone(),
            kind: PlaceKind::Drive,
            capacity: measure(&mount.point),
            device: Some(mount.device.clone()),
        });
    }
    places
}

/// Is this mount somewhere a person would go?
fn is_place(mount: &Mount) -> bool {
    // The root is a place, but it is *the* place, and it gets its own section.
    if mount.point == Path::new("/") {
        return false;
    }
    if under_any(&mount.point, REMOVABLE_ROOTS) {
        return true;
    }
    if under_any(&mount.point, NOT_PLACES) {
        return false;
    }
    // Otherwise: a real block device, mounted somewhere ordinary. A separate `/home` partition
    // is the usual case, and it belongs here.
    mount.device.starts_with("/dev/") && !mount.device.starts_with("/dev/loop")
}

fn under_any(path: &Path, roots: &[&str]) -> bool {
    for root in roots {
        let root = Path::new(root);
        if path == root || path.starts_with(root) {
            return true;
        }
    }
    false
}

/// What to call a volume.
///
/// The filesystem label first, because that is the name its owner gave it. Then the last part
/// of the mount point, which for `/run/media/<user>/<label>` is the label anyway and for `/home`
/// is `home`. The device only as a last resort — `/dev/sdb1` is an answer, but not a good one.
fn volume_name(mount: &Mount, labels: &HashMap<String, String>) -> String {
    if let Some(label) = labels.get(&mount.device) {
        return label.clone();
    }
    folder_name(&mount.point).unwrap_or_else(|| mount.device.clone())
}

fn root_device(mounts: &[Mount]) -> Option<String> {
    for mount in mounts {
        if mount.point == Path::new("/") {
            return Some(mount.device.clone());
        }
    }
    None
}

/// Parse `/proc/mounts`.
///
/// Five fields, space-separated, with spaces and tabs in paths written as octal escapes — which
/// is why a stick called `My Stick` mounts at `/media/user/My\040Stick` and a naive split gives
/// six fields and a wrong answer.
pub fn read_mounts(path: &Path) -> Vec<Mount> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_mounts(&text)
}

pub fn parse_mounts(text: &str) -> Vec<Mount> {
    let mut mounts = Vec::new();
    for line in text.lines() {
        let mut fields = line.split(' ');
        let (Some(device), Some(point), Some(fstype)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        mounts.push(Mount {
            device: unescape_octal(device),
            point: PathBuf::from(unescape_octal(point)),
            fstype: fstype.to_string(),
        });
    }
    mounts
}

/// `\040` → a space. The kernel escapes space, tab, newline and backslash this way.
fn unescape_octal(text: &str) -> String {
    if !text.contains('\\') {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() {
            let digits = &text[i + 1..i + 4];
            if let Ok(byte) = u8::from_str_radix(digits, 8) {
                out.push(byte as char);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Read `/dev/disk/by-label`, which is a directory of symlinks from label to device.
///
/// Returns device → label, which is the direction a mount table needs. A label with a space in
/// it appears as `My\x20Stick` in the file name.
pub fn read_labels(dir: &Path) -> HashMap<String, String> {
    let mut labels = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return labels;
    };

    for entry in entries.flatten() {
        let Ok(device) = std::fs::canonicalize(entry.path()) else {
            continue;
        };
        let label = unescape_hex(&entry.file_name().to_string_lossy());
        labels.insert(device.display().to_string(), label);
    }
    labels
}

/// `My\x20Stick` → `My Stick`.
fn unescape_hex(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() && bytes[i + 1] == b'x' {
            if let Ok(byte) = u8::from_str_radix(&text[i + 2..i + 4], 16) {
                out.push(byte as char);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// How full a filesystem is.
///
/// `statvfs`, declared by hand rather than through a dependency. The alternative is `libc` for
/// one call, and the fields this needs — block size, total blocks, blocks available to a
/// non-root user — are the three that have been in the same place since 4.4BSD.
fn capacity_of(path: &Path) -> Option<Capacity> {
    use std::os::unix::ffi::OsStrExt;

    // The layout of `struct statvfs` on Linux: eight `unsigned long`s, then `fsid`, flags,
    // namemax, and padding. Over-allocating and reading only the leading fields is what makes
    // this safe against the padding differing between architectures.
    #[repr(C)]
    #[derive(Default)]
    struct StatVfs {
        bsize: libc_ulong,
        frsize: libc_ulong,
        blocks: u64,
        bfree: u64,
        bavail: u64,
        files: u64,
        ffree: u64,
        favail: u64,
        fsid: libc_ulong,
        flag: libc_ulong,
        namemax: libc_ulong,
        spare: [u32; 6],
    }

    extern "C" {
        #[link_name = "statvfs"]
        fn c_statvfs(path: *const std::ffi::c_char, buf: *mut StatVfs) -> i32;
    }

    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat = StatVfs::default();
    // SAFETY: `c_path` is a valid NUL-terminated string for the length of the call, and `stat`
    // is a `#[repr(C)]` struct at least as large as the kernel's.
    let ok = unsafe { c_statvfs(c_path.as_ptr(), &mut stat) } == 0;
    if !ok {
        return None;
    }

    // `frsize` is the fragment size and the one the block counts are in; `bsize` is the
    // preferred I/O size and is a different number on some filesystems.
    let unit = if stat.frsize > 0 { stat.frsize } else { stat.bsize } as u64;
    Some(Capacity {
        total_bytes: stat.blocks.saturating_mul(unit),
        // `bavail`, not `bfree`: the difference is the reserve only root may use, and reporting
        // space a person cannot fill is reporting the wrong number.
        free_bytes: stat.bavail.saturating_mul(unit),
    })
}

#[allow(non_camel_case_types)]
type libc_ulong = std::ffi::c_ulong;

// ---------------------------------------------------------------------------
// Small shared pieces.
// ---------------------------------------------------------------------------

fn folder_name(path: &Path) -> Option<String> {
    path.file_name().map(|n| n.to_string_lossy().to_string())
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&text[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mount(device: &str, point: &str) -> Mount {
        Mount {
            device: device.to_string(),
            point: PathBuf::from(point),
            fstype: "ext4".to_string(),
        }
    }

    fn nothing(_: &Path) -> Option<Capacity> {
        None
    }

    fn titles(sections: &[PlaceSection]) -> Vec<&str> {
        sections.iter().map(|s| s.title.as_str()).collect()
    }

    fn section<'a>(sections: &'a [PlaceSection], title: &str) -> &'a PlaceSection {
        for candidate in sections {
            if candidate.title == title {
                return candidate;
            }
        }
        panic!("no section called {title}");
    }

    /// A home directory with the folders a person has.
    fn a_home() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for folder in ["Downloads", "Documents", "Pictures", ".config"] {
            std::fs::create_dir_all(dir.path().join(folder)).unwrap();
        }
        dir
    }

    #[test]
    fn the_system_is_one_row_at_the_end() {
        // The whole point. Opening at `/` puts sixteen directories in front of somebody who
        // wanted their Downloads; this puts one, last, called what it is.
        let home = a_home();
        let sections = build(home.path(), &[], &HashMap::new(), &nothing);

        let system = section(&sections, "System");
        assert_eq!(system.places.len(), 1);
        assert_eq!(system.places[0].path, PathBuf::from("/"));
        assert_eq!(system.places[0].name, "Dateisystem");
        assert_eq!(system.places[0].kind, PlaceKind::Filesystem);
        assert_eq!(titles(&sections).last(), Some(&"System"));
    }

    #[test]
    fn the_personal_section_comes_first_and_starts_with_home() {
        let home = a_home();
        let sections = build(home.path(), &[], &HashMap::new(), &nothing);
        assert_eq!(titles(&sections)[0], "Persönlich");

        let personal = section(&sections, "Persönlich");
        assert_eq!(personal.places[0].kind, PlaceKind::Home);
        assert_eq!(personal.places[0].path, home.path());
    }

    #[test]
    fn only_the_folders_that_exist_are_offered() {
        // Offering a Videos folder to somebody who has never had one is the clutter this screen
        // exists to remove.
        let home = a_home();
        let sections = build(home.path(), &[], &HashMap::new(), &nothing);
        let kinds: Vec<_> = section(&sections, "Persönlich")
            .places
            .iter()
            .map(|p| p.kind)
            .collect();

        assert!(kinds.contains(&PlaceKind::Downloads));
        assert!(kinds.contains(&PlaceKind::Documents));
        assert!(kinds.contains(&PlaceKind::Pictures));
        assert!(!kinds.contains(&PlaceKind::Videos), "there is no Videos folder");
        assert!(!kinds.contains(&PlaceKind::Music));
    }

    #[test]
    fn downloads_comes_before_documents() {
        // Where a browser puts things, which is where most of what somebody is looking for
        // arrived twenty minutes ago.
        let home = a_home();
        let sections = build(home.path(), &[], &HashMap::new(), &nothing);
        let places = &section(&sections, "Persönlich").places;
        let position = |kind: PlaceKind| places.iter().position(|p| p.kind == kind).unwrap();
        assert!(position(PlaceKind::Downloads) < position(PlaceKind::Documents));
    }

    #[test]
    fn a_translated_downloads_folder_is_found_and_called_by_its_name() {
        // `user-dirs.dirs` is how a German desktop records that Downloads is `Herunterladen`.
        // Looking only for the English name would offer a folder that is not there and miss the
        // one that is.
        let home = a_home();
        std::fs::create_dir_all(home.path().join("Herunterladen")).unwrap();
        std::fs::write(
            home.path().join(".config/user-dirs.dirs"),
            "# comment\nXDG_DOWNLOAD_DIR=\"$HOME/Herunterladen\"\n",
        )
        .unwrap();

        let sections = build(home.path(), &[], &HashMap::new(), &nothing);
        let places = &section(&sections, "Persönlich").places;
        let downloads = places.iter().find(|p| p.kind == PlaceKind::Downloads).unwrap();
        assert_eq!(downloads.path, home.path().join("Herunterladen"));
        assert_eq!(downloads.name, "Herunterladen");
    }

    #[test]
    fn a_user_dirs_file_that_is_nonsense_falls_back_rather_than_failing() {
        let home = a_home();
        std::fs::write(
            home.path().join(".config/user-dirs.dirs"),
            "XDG_DOWNLOAD_DIR=relative/path\nnot a line at all\nXDG_MUSIC_DIR=\n",
        )
        .unwrap();

        let sections = build(home.path(), &[], &HashMap::new(), &nothing);
        let places = &section(&sections, "Persönlich").places;
        let downloads = places.iter().find(|p| p.kind == PlaceKind::Downloads).unwrap();
        assert_eq!(downloads.path, home.path().join("Downloads"), "the default");
    }

    #[test]
    fn the_trash_is_offered_when_there_is_one() {
        let home = a_home();
        let sections = build(home.path(), &[], &HashMap::new(), &nothing);
        assert!(
            !section(&sections, "Persönlich")
                .places
                .iter()
                .any(|p| p.kind == PlaceKind::Trash),
            "no trash directory yet"
        );

        std::fs::create_dir_all(home.path().join(".local/share/Trash/files")).unwrap();
        let sections = build(home.path(), &[], &HashMap::new(), &nothing);
        let trash = section(&sections, "Persönlich")
            .places
            .iter()
            .find(|p| p.kind == PlaceKind::Trash)
            .unwrap();
        assert_eq!(trash.name, "Papierkorb");
    }

    #[test]
    fn bookmarks_are_read_from_the_file_the_rest_of_the_desktop_uses() {
        let home = a_home();
        std::fs::create_dir_all(home.path().join(".config/gtk-3.0")).unwrap();
        let project = home.path().join("Documents");
        std::fs::write(
            home.path().join(".config/gtk-3.0/bookmarks"),
            format!(
                "file://{} Arbeit\nfile://{}\nsftp://example.invalid/x\n",
                project.display(),
                home.path().join("Pictures").display()
            ),
        )
        .unwrap();

        let sections = build(home.path(), &[], &HashMap::new(), &nothing);
        let marked = &section(&sections, "Lesezeichen").places;
        assert_eq!(marked.len(), 2, "the sftp one is not a place on this machine");
        assert_eq!(marked[0].name, "Arbeit", "the label wins over the folder name");
        assert_eq!(marked[1].name, "Pictures", "and the folder name is the fallback");
    }

    #[test]
    fn there_is_no_bookmarks_section_when_there_are_no_bookmarks() {
        let home = a_home();
        let sections = build(home.path(), &[], &HashMap::new(), &nothing);
        assert!(!titles(&sections).contains(&"Lesezeichen"));
    }

    #[test]
    fn a_bookmark_to_a_folder_that_is_gone_is_not_offered() {
        let home = a_home();
        std::fs::create_dir_all(home.path().join(".config/gtk-3.0")).unwrap();
        std::fs::write(
            home.path().join(".config/gtk-3.0/bookmarks"),
            "file:///nonexistent/gone Weg\n",
        )
        .unwrap();
        let sections = build(home.path(), &[], &HashMap::new(), &nothing);
        assert!(!titles(&sections).contains(&"Lesezeichen"));
    }

    // -----------------------------------------------------------------------
    // Volumes.
    // -----------------------------------------------------------------------

    #[test]
    fn a_stick_is_a_drive_and_the_kernels_plumbing_is_not() {
        let home = a_home();
        let mounts = vec![
            mount("/dev/nvme0n1p2", "/"),
            mount("/dev/sdb1", "/media/user/Stick"),
            mount("/dev/nvme0n1p1", "/boot/efi"),
            mount("/dev/loop3", "/snap/firefox/1234"),
            mount("proc", "/proc"),
            mount("tmpfs", "/run/user/1000"),
            mount("/dev/nvme0n1p3", "/home"),
        ];

        let sections = build(home.path(), &mounts, &HashMap::new(), &nothing);
        let drives = &section(&sections, "Laufwerke").places;
        let points: Vec<_> = drives.iter().map(|d| d.path.display().to_string()).collect();

        assert_eq!(points, vec!["/media/user/Stick", "/home"]);
    }

    #[test]
    fn the_root_is_not_listed_twice() {
        // It has its own section. A `/` in the drive list as well would be the same place under
        // two names, which is how a list stops being trustworthy.
        let home = a_home();
        let mounts = vec![mount("/dev/nvme0n1p2", "/")];
        let sections = build(home.path(), &mounts, &HashMap::new(), &nothing);
        assert!(!titles(&sections).contains(&"Laufwerke"));
        assert_eq!(section(&sections, "System").places[0].device.as_deref(), Some("/dev/nvme0n1p2"));
    }

    #[test]
    fn a_removable_mount_counts_even_when_it_is_not_a_block_device() {
        // An exFAT stick behind a FUSE driver is as much a drive as an internal SSD, and the
        // device it reports is not under `/dev`.
        let home = a_home();
        let mounts = vec![Mount {
            device: "exfat-fuse".to_string(),
            point: PathBuf::from("/run/media/user/CAMERA"),
            fstype: "fuseblk".to_string(),
        }];
        let sections = build(home.path(), &mounts, &HashMap::new(), &nothing);
        assert_eq!(section(&sections, "Laufwerke").places.len(), 1);
    }

    #[test]
    fn a_label_is_what_a_drive_is_called() {
        let home = a_home();
        let mounts = vec![mount("/dev/sdb1", "/media/user/sdb1")];
        let mut labels = HashMap::new();
        labels.insert("/dev/sdb1".to_string(), "Sicherung".to_string());

        let sections = build(home.path(), &mounts, &labels, &nothing);
        assert_eq!(section(&sections, "Laufwerke").places[0].name, "Sicherung");

        // Without a label, the mount point's own name — which is usually the label anyway.
        let sections = build(home.path(), &mounts, &HashMap::new(), &nothing);
        assert_eq!(section(&sections, "Laufwerke").places[0].name, "sdb1");
    }

    #[test]
    fn the_same_mount_point_twice_is_one_drive() {
        let home = a_home();
        let mounts = vec![
            mount("/dev/sdb1", "/media/user/Stick"),
            mount("/dev/sdb1", "/media/user/Stick"),
        ];
        let sections = build(home.path(), &mounts, &HashMap::new(), &nothing);
        assert_eq!(section(&sections, "Laufwerke").places.len(), 1);
    }

    #[test]
    fn capacity_comes_back_with_a_drive_and_not_with_a_folder() {
        let home = a_home();
        let mounts = vec![mount("/dev/sdb1", "/media/user/Stick")];
        let measured = |_: &Path| {
            Some(Capacity {
                total_bytes: 1000,
                free_bytes: 250,
            })
        };

        let sections = build(home.path(), &mounts, &HashMap::new(), &measured);
        let drive = &section(&sections, "Laufwerke").places[0];
        assert_eq!(drive.capacity.unwrap().used_fraction(), 0.75);
        assert!(section(&sections, "Persönlich").places[0].capacity.is_none());
    }

    #[test]
    fn a_capacity_of_nothing_does_not_divide_by_zero() {
        let empty = Capacity {
            total_bytes: 0,
            free_bytes: 0,
        };
        assert_eq!(empty.used_fraction(), 0.0);
    }

    // -----------------------------------------------------------------------
    // Parsing.
    // -----------------------------------------------------------------------

    #[test]
    fn a_mount_point_with_a_space_in_it_is_read_correctly() {
        // The kernel writes `\040`, and a naive split on spaces gives six fields and a stick
        // called `My`.
        let mounts = parse_mounts("/dev/sdb1 /media/user/My\\040Stick vfat rw,noatime 0 0\n");
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].point, PathBuf::from("/media/user/My Stick"));
        assert_eq!(mounts[0].fstype, "vfat");
    }

    #[test]
    fn the_real_mount_table_parses() {
        // The one assertion that touches the machine: a parser that works on a fixture and not
        // on `/proc/mounts` is a parser for a format nobody uses.
        let mounts = read_mounts(Path::new(PROC_MOUNTS));
        assert!(!mounts.is_empty(), "this machine has mounts");
        assert!(
            mounts.iter().any(|m| m.point == Path::new("/")),
            "one of them is the root"
        );
    }

    #[test]
    fn the_root_filesystem_has_a_size() {
        // And so does `statvfs`, hand-declared, which is the part worth checking against a real
        // kernel rather than a fixture.
        let capacity = capacity_of(Path::new("/")).expect("the root is measurable");
        assert!(capacity.total_bytes > 0);
        assert!(capacity.free_bytes <= capacity.total_bytes);
        assert!(capacity.used_fraction() > 0.0 && capacity.used_fraction() <= 1.0);
    }

    #[test]
    fn a_label_with_a_space_is_unescaped() {
        assert_eq!(unescape_hex("My\\x20Stick"), "My Stick");
        assert_eq!(unescape_hex("Plain"), "Plain");
    }

    #[test]
    fn a_bookmark_uri_is_percent_decoded() {
        assert_eq!(
            local_path("file:///home/user/two%20words"),
            Some(PathBuf::from("/home/user/two words"))
        );
        assert_eq!(local_path("smb://server/share"), None);
        assert_eq!(local_path("file://server/share"), None);
    }
}
