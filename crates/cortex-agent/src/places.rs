//! Where a person starts, as opposed to where the filesystem starts.
//!
//! Opening a file manager at `/` is what happens when nobody decides where to open it. The
//! first screen is then `bin`, `boot`, `dev`, `etc`, `lib`, `lost+found`, `opt`, `proc`,
//! `run`, `sbin`, `srv`, `sys`, `tmp`, `usr`, `var` — sixteen directories, of which one is
//! ever wanted, and it is `home`. Everything a person actually opens is four levels down.
//!
//! So this answers a different question: not "what is in this directory" but "where would you
//! like to go". Five sections, and the technical half of the machine is one row in the last of
//! them:
//!
//! 1. **Persönlich** — the home directory and the XDG user directories that exist.
//! 2. **Lesezeichen** — whatever the person bookmarked, read from the same file Nautilus and
//!    Thunar use, so a bookmark made in either shows up in all three.
//! 3. **Laufwerke** — mounted volumes, with a label and how full they are.
//! 4. **Netzwerk** — shares on other machines that something has already mounted. Nothing here
//!    mounts one; see [`network_places`] for where that line is drawn and why.
//! 5. **System** — the filesystem root. One row. Everything above is still reachable through it,
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

/// One mounted filesystem. The parser and the network-filesystem list live in `cortex-core`,
/// because the indexer needs the same answer for the opposite reason — see [`cortex_core::mounts`].
pub use cortex_core::mounts::Mount;

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
    // What `/proc/mounts` cannot say: the stick that is plugged in and not mounted yet, which
    // is the thing somebody is looking for when they plug one in.
    let idle = crate::volumes::unmounted(
        Path::new(crate::volumes::SYS_BLOCK),
        Path::new(crate::volumes::UDEV_DATA),
        &mounts,
    );
    build(home, &mounts, &labels, &idle, &capacity_of)
}

/// The part with no I/O of its own, so that every decision in it is testable.
///
/// `measure` is how a volume's capacity is found; the real one calls `statvfs` and the tests
/// supply numbers.
pub fn build(
    home: &Path,
    mounts: &[Mount],
    labels: &HashMap<String, String>,
    idle: &[crate::volumes::Volume],
    measure: &dyn Fn(&Path) -> Option<Capacity>,
) -> Vec<PlaceSection> {
    build_with(home, mounts, labels, idle, measure, &list_dir)
}

/// The same, with the directory listing injected.
///
/// gvfs publishes one mount whose *children* are the shares, so the only way to name them is to
/// read that directory — and a test that had to create `smb-share:server=nas,share=fotos` on a
/// real filesystem would be testing the tempdir rather than the naming.
pub fn build_with(
    home: &Path,
    mounts: &[Mount],
    labels: &HashMap<String, String>,
    idle: &[crate::volumes::Volume],
    measure: &dyn Fn(&Path) -> Option<Capacity>,
    read_dir: &dyn Fn(&Path) -> Vec<String>,
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

    let drives = drive_places(mounts, labels, idle, measure);
    if !drives.is_empty() {
        sections.push(PlaceSection {
            title: "Laufwerke".to_string(),
            places: drives,
        });
    }

    let network = network_places(mounts, read_dir);
    if !network.is_empty() {
        sections.push(PlaceSection {
            title: "Netzwerk".to_string(),
            places: network,
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
            volume: None,
            source: None,
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
        volume: None,
        source: None,
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
                volume: None,
            source: None,
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
            volume: None,
            source: None,
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
            volume: None,
            source: None,
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
    idle: &[crate::volumes::Volume],
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
            volume: volume_of(idle, &mount.device),
            source: None,
        });
    }

    // Then the ones that are plugged in and not mounted. After the mounted ones, because a
    // drive somebody is already using is more likely to be the one they want than a drive that
    // is merely present — and a row that has to be mounted before it can be opened is a row
    // with a step in it.
    for volume in idle {
        places.push(Place {
            name: volume.name(),
            // A device node rather than a directory, because there is no directory yet. What
            // makes that legible to a client is `volume.mounted` being false, and what makes it
            // safe is that nothing tries to list a place without looking at that first.
            path: volume.device.clone(),
            kind: PlaceKind::Drive,
            capacity: None,
            device: Some(volume.device.display().to_string()),
            volume: Some(cortex_core::tools::VolumeInfo {
                mounted: false,
                fstype: volume.fstype.clone(),
                size_bytes: volume.size_bytes,
                removable: volume.removable,
            }),
            source: None,
        });
    }

    places
}

// ---------------------------------------------------------------------------
// Network locations.
// ---------------------------------------------------------------------------

/// Shares on other machines, as places.
///
/// **Nothing here is mounted by this program.** A share appears because the desktop mounted it
/// (gvfs, from Nautilus or the portal) or because `/etc/fstab` did. That is the staged half M6
/// allows, and the line is drawn where the credentials are: mounting an SMB share means asking
/// for a password, storing it, and answering a re-authentication prompt hours later, and a
/// headless daemon that did any of that would be a second, worse keyring. What this does is
/// make what is already mounted reachable, browsable and searchable.
///
/// **No capacity is measured.** `statvfs` on a share whose server has gone is one of the calls
/// that blocks, and the first screen is not a place to find that out. A drive shows how full it
/// is; a share shows where it is.
fn network_places(mounts: &[Mount], read_dir: &dyn Fn(&Path) -> Vec<String>) -> Vec<Place> {
    let mut places = Vec::new();

    for mount in mounts {
        if !cortex_core::mounts::is_network(&mount.fstype) {
            continue;
        }

        // gvfs is one mount with the shares underneath it. The mount itself is plumbing — a row
        // called "gvfs" is a row nobody wants — so its children stand in for it.
        if is_gvfs(&mount.fstype) {
            for name in read_dir(&mount.point) {
                places.push(Place {
                    name: gvfs_name(&name),
                    path: mount.point.join(&name),
                    kind: PlaceKind::Network,
                    capacity: None,
                    // The address rather than the encoding: `smb://nas/fotos` is what somebody
                    // typed to mount it, and `smb-share:server=nas,share=fotos` is how gvfs
                    // chose to spell it in a directory name.
                    device: Some(gvfs_address(&name)),
                    volume: None,
                    source: None,
                });
            }
            continue;
        }

        places.push(Place {
            name: network_name(&mount.device, &mount.point),
            path: mount.point.clone(),
            kind: PlaceKind::Network,
            capacity: None,
            device: Some(mount.device.clone()),
            volume: None,
            source: None,
        });
    }

    places
}

fn is_gvfs(fstype: &str) -> bool {
    fstype == "fuse.gvfsd-fuse" || fstype == "fuse.gvfs-fuse-daemon"
}

/// The names of the entries in a directory, or nothing if it will not answer.
///
/// The deadline is here rather than around the whole screen because this is the one directory in
/// it that is served by a daemon: gvfs answers out of its own state, which is fast until gvfsd
/// is wedged, and a places screen that hangs because of that would be the first screen hanging.
fn list_dir(path: &Path) -> Vec<String> {
    let owned = path.to_path_buf();
    let listed = cortex_core::mounts::answers_within(
        std::time::Duration::from_millis(300),
        move || {
            let mut names = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&owned) {
                for entry in entries.flatten() {
                    names.push(entry.file_name().to_string_lossy().to_string());
                }
            }
            names
        },
    );
    let mut names = listed.unwrap_or_default();
    names.sort();
    names
}

/// What to call a share that was mounted the traditional way.
///
/// The device string is the only thing that says where it came from, and each protocol writes it
/// differently: `//server/photos` for SMB, `server:/export` for NFS, `me@server:/srv` for sshfs.
/// All three answer the same question — *what*, on *which machine* — so all three are read into
/// the same sentence.
fn network_name(device: &str, point: &Path) -> String {
    let (host, share) = split_device(device);
    match (host, share) {
        (Some(host), Some(share)) if !share.is_empty() => format!("{share} auf {host}"),
        (Some(host), _) => host.to_string(),
        _ => folder_name(point).unwrap_or_else(|| device.to_string()),
    }
}

/// `//server/photos` → (server, photos); `me@server:/srv/data` → (server, data).
///
/// Only the two forms `/proc/mounts` actually writes on Linux. A Windows UNC path with
/// backslashes never appears in it, and a branch for one would be a branch no test could reach.
fn split_device(device: &str) -> (Option<&str>, Option<&str>) {
    if let Some(rest) = device.strip_prefix("//") {
        let mut parts = rest.splitn(2, '/');
        return (parts.next(), parts.next());
    }
    if let Some((left, path)) = device.split_once(':') {
        // `user@host` — the user is authentication, not identity, and a row that says
        // "data auf me@server" is a row with a login in it.
        let host = left.rsplit('@').next().unwrap_or(left);
        let last = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
        return (Some(host), Some(last));
    }
    (None, None)
}

/// Decode one gvfs directory name.
///
/// gvfs encodes the whole mount into the name: `smb-share:server=nas,share=fotos`,
/// `sftp:host=server,user=someone`, `dav:host=cloud,ssl=true`. It is a scheme, then
/// comma-separated pairs — so it is read as one, rather than matched per protocol, and a scheme
/// this build has never heard of still produces a row with a host in it.
fn gvfs_name(entry: &str) -> String {
    let (scheme, rest) = entry.split_once(':').unwrap_or((entry, ""));
    let mut host = None;
    let mut share = None;
    for pair in rest.split(',') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        match key {
            "server" | "host" => host = Some(value),
            "share" => share = Some(value),
            _ => {}
        }
    }

    let protocol = match scheme {
        "smb-share" | "smb" => "SMB",
        "sftp" | "ssh" => "SFTP",
        "dav" | "davs" => "WebDAV",
        "ftp" | "ftps" => "FTP",
        "nfs" => "NFS",
        other => other,
    };

    match (host, share) {
        (Some(host), Some(share)) => format!("{share} auf {host}"),
        (Some(host), None) => format!("{host} ({protocol})"),
        // Not a shape this build knows. The raw name is ugly and it is *true*, which is the
        // right trade for a row somebody has to click on.
        _ => entry.to_string(),
    }
}

/// The address a gvfs mount was made from, rebuilt from the name it was given.
///
/// `smb-share:server=nas,share=fotos` → `smb://nas/fotos`. Not round-tripped through anything
/// that mounts: nothing here mounts. It is the line under the name in the sidebar, for telling
/// two shares called `fotos` apart.
fn gvfs_address(entry: &str) -> String {
    let (scheme, rest) = entry.split_once(':').unwrap_or((entry, ""));
    let mut host = None;
    let mut share = None;
    for pair in rest.split(',') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        match key {
            "server" | "host" => host = Some(value),
            "share" => share = Some(value),
            _ => {}
        }
    }
    let scheme = if scheme == "smb-share" { "smb" } else { scheme };
    match (host, share) {
        (Some(host), Some(share)) => format!("{scheme}://{host}/{share}"),
        (Some(host), None) => format!("{scheme}://{host}"),
        _ => entry.to_string(),
    }
}

/// What is known about the device behind a mount, when it is one of the volumes scanned.
///
/// A mounted drive carries the same facts as an unmounted one — how big it is, whether it can be
/// unplugged — because "can I eject this" is a question about a mounted drive.
fn volume_of(
    volumes: &[crate::volumes::Volume],
    device: &str,
) -> Option<cortex_core::tools::VolumeInfo> {
    let found = volumes
        .iter()
        .find(|v| v.device.to_string_lossy() == device)?;
    Some(cortex_core::tools::VolumeInfo {
        mounted: true,
        fstype: found.fstype.clone(),
        size_bytes: found.size_bytes,
        removable: found.removable,
    })
}

/// Is this mount somewhere a person would go?
fn is_place(mount: &Mount) -> bool {
    // The root is a place, but it is *the* place, and it gets its own section.
    if mount.point == Path::new("/") {
        return false;
    }
    // A share has its own section. Mounting one under `/mnt` is ordinary — the removable-media
    // rule below would otherwise offer it a second time, as a drive, with a capacity nobody
    // should be measuring.
    if cortex_core::mounts::is_network(&mount.fstype) {
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
    cortex_core::mounts::read(path)
}

pub fn parse_mounts(text: &str) -> Vec<Mount> {
    cortex_core::mounts::parse(text)
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
        let sections = build(home.path(), &[], &HashMap::new(), &[], &nothing);

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
        let sections = build(home.path(), &[], &HashMap::new(), &[], &nothing);
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
        let sections = build(home.path(), &[], &HashMap::new(), &[], &nothing);
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
        let sections = build(home.path(), &[], &HashMap::new(), &[], &nothing);
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

        let sections = build(home.path(), &[], &HashMap::new(), &[], &nothing);
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

        let sections = build(home.path(), &[], &HashMap::new(), &[], &nothing);
        let places = &section(&sections, "Persönlich").places;
        let downloads = places.iter().find(|p| p.kind == PlaceKind::Downloads).unwrap();
        assert_eq!(downloads.path, home.path().join("Downloads"), "the default");
    }

    #[test]
    fn the_trash_is_offered_when_there_is_one() {
        let home = a_home();
        let sections = build(home.path(), &[], &HashMap::new(), &[], &nothing);
        assert!(
            !section(&sections, "Persönlich")
                .places
                .iter()
                .any(|p| p.kind == PlaceKind::Trash),
            "no trash directory yet"
        );

        std::fs::create_dir_all(home.path().join(".local/share/Trash/files")).unwrap();
        let sections = build(home.path(), &[], &HashMap::new(), &[], &nothing);
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

        let sections = build(home.path(), &[], &HashMap::new(), &[], &nothing);
        let marked = &section(&sections, "Lesezeichen").places;
        assert_eq!(marked.len(), 2, "the sftp one is not a place on this machine");
        assert_eq!(marked[0].name, "Arbeit", "the label wins over the folder name");
        assert_eq!(marked[1].name, "Pictures", "and the folder name is the fallback");
    }

    #[test]
    fn there_is_no_bookmarks_section_when_there_are_no_bookmarks() {
        let home = a_home();
        let sections = build(home.path(), &[], &HashMap::new(), &[], &nothing);
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
        let sections = build(home.path(), &[], &HashMap::new(), &[], &nothing);
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

        let sections = build(home.path(), &mounts, &HashMap::new(), &[], &nothing);
        let drives = &section(&sections, "Laufwerke").places;
        let points: Vec<_> = drives.iter().map(|d| d.path.display().to_string()).collect();

        assert_eq!(points, vec!["/media/user/Stick", "/home"]);
    }

    /// A volume the scan found, for the tests below.
    fn volume(device: &str, label: &str, removable: bool) -> crate::volumes::Volume {
        crate::volumes::Volume {
            device: PathBuf::from(device),
            label: Some(label.to_string()),
            fstype: "vfat".to_string(),
            size_bytes: 31_000_000_000,
            removable,
            mount_point: None,
        }
    }

    #[test]
    fn a_stick_that_is_plugged_in_and_not_mounted_is_still_offered() {
        // The moment this exists for: something was just plugged in, `/proc/mounts` knows
        // nothing about it, and it is the only reason the person opened this screen.
        let home = a_home();
        let mounts = vec![mount("/dev/nvme0n1p2", "/")];
        let idle = vec![volume("/dev/sdb1", "ESD-USB", true)];

        let sections = build(home.path(), &mounts, &HashMap::new(), &idle, &nothing);
        let drives = &section(&sections, "Laufwerke").places;
        assert_eq!(drives.len(), 1);
        assert_eq!(drives[0].name, "ESD-USB");

        let info = drives[0].volume.as_ref().expect("a drive carries what it is");
        assert!(!info.mounted, "and says that it is not mounted");
        assert!(info.removable);
        assert_eq!(drives[0].path, PathBuf::from("/dev/sdb1"), "the device, because there is no directory yet");
        assert_eq!(drives[0].capacity, None, "how full it is cannot be known before it is mounted");
    }

    #[test]
    fn mounted_drives_come_before_ones_that_would_have_to_be_mounted() {
        // A row that opens is worth more than a row with a step in it.
        let home = a_home();
        let mounts = vec![mount("/dev/nvme0n1p2", "/"), mount("/dev/sdc1", "/media/user/Arbeit")];
        let idle = vec![volume("/dev/sdb1", "ESD-USB", true)];
        let sections = build(home.path(), &mounts, &HashMap::new(), &idle, &nothing);
        let names: Vec<&str> = section(&sections, "Laufwerke")
            .places
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(names, ["Arbeit", "ESD-USB"]);
    }

    #[test]
    fn a_mounted_drive_says_whether_it_can_be_unplugged() {
        // "Can I eject this" is a question about a mounted drive, and the answer is a property
        // of the disk rather than of the mount — so the scan's facts are carried across.
        let home = a_home();
        let mounts = vec![mount("/dev/nvme0n1p2", "/"), mount("/dev/sdb1", "/media/user/Stick")];
        let mut found = volume("/dev/sdb1", "ESD-USB", true);
        found.mount_point = Some(PathBuf::from("/media/user/Stick"));

        let sections = build(home.path(), &mounts, &HashMap::new(), &[found], &nothing);
        let drive = &section(&sections, "Laufwerke").places[0];
        let info = drive.volume.as_ref().unwrap();
        assert!(info.mounted);
        assert!(info.removable);
        assert_eq!(info.fstype, "vfat");
    }

    #[test]
    fn the_root_is_not_listed_twice() {
        // It has its own section. A `/` in the drive list as well would be the same place under
        // two names, which is how a list stops being trustworthy.
        let home = a_home();
        let mounts = vec![mount("/dev/nvme0n1p2", "/")];
        let sections = build(home.path(), &mounts, &HashMap::new(), &[], &nothing);
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
        let sections = build(home.path(), &mounts, &HashMap::new(), &[], &nothing);
        assert_eq!(section(&sections, "Laufwerke").places.len(), 1);
    }

    #[test]
    fn a_label_is_what_a_drive_is_called() {
        let home = a_home();
        let mounts = vec![mount("/dev/sdb1", "/media/user/sdb1")];
        let mut labels = HashMap::new();
        labels.insert("/dev/sdb1".to_string(), "Sicherung".to_string());

        let sections = build(home.path(), &mounts, &labels, &[], &nothing);
        assert_eq!(section(&sections, "Laufwerke").places[0].name, "Sicherung");

        // Without a label, the mount point's own name — which is usually the label anyway.
        let sections = build(home.path(), &mounts, &HashMap::new(), &[], &nothing);
        assert_eq!(section(&sections, "Laufwerke").places[0].name, "sdb1");
    }

    #[test]
    fn the_same_mount_point_twice_is_one_drive() {
        let home = a_home();
        let mounts = vec![
            mount("/dev/sdb1", "/media/user/Stick"),
            mount("/dev/sdb1", "/media/user/Stick"),
        ];
        let sections = build(home.path(), &mounts, &HashMap::new(), &[], &nothing);
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

        let sections = build(home.path(), &mounts, &HashMap::new(), &[], &measured);
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

#[cfg(test)]
mod network_tests {
    use super::*;

    fn mount(device: &str, point: &str, fstype: &str) -> Mount {
        Mount {
            device: device.to_string(),
            point: PathBuf::from(point),
            fstype: fstype.to_string(),
        }
    }

    fn nothing(_: &Path) -> Option<Capacity> {
        None
    }

    #[test]
    fn a_share_says_what_it_is_and_which_machine_it_is_on() {
        assert_eq!(
            network_name("//nas/fotos", Path::new("/mnt/fotos")),
            "fotos auf nas"
        );
        assert_eq!(
            network_name("server:/export/backup", Path::new("/mnt/backup")),
            "backup auf server"
        );
        // A login is authentication, not identity: nobody wants a row called "data auf me@nas".
        assert_eq!(
            network_name("me@nas:/srv/data", Path::new("/mnt/data")),
            "data auf nas"
        );
        // Nothing recognisable: the mount point's own name, which is at least true.
        assert_eq!(
            network_name("something-odd", Path::new("/mnt/irgendwas")),
            "irgendwas"
        );
    }

    #[test]
    fn a_gvfs_mount_is_read_as_a_scheme_and_its_pairs() {
        assert_eq!(gvfs_name("smb-share:server=nas,share=fotos"), "fotos auf nas");
        assert_eq!(gvfs_name("sftp:host=server,user=someone"), "server (SFTP)");
        assert_eq!(gvfs_name("dav:host=cloud,ssl=true"), "cloud (WebDAV)");
        // A scheme this build has never seen still produces a row with a machine in it.
        assert_eq!(gvfs_name("wombat:host=elsewhere"), "elsewhere (wombat)");
        // And something that is not the shape at all keeps its name rather than being guessed at.
        assert_eq!(gvfs_name("mtp-volume"), "mtp-volume");
    }

    #[test]
    fn gvfs_shares_are_rows_and_the_gvfs_mount_itself_is_not() {
        let mounts = vec![
            mount("gvfsd-fuse", "/run/user/1000/gvfs", "fuse.gvfsd-fuse"),
            mount("//nas/fotos", "/mnt/fotos", "cifs"),
        ];
        let listing = |path: &Path| -> Vec<String> {
            if path == Path::new("/run/user/1000/gvfs") {
                vec![
                    "smb-share:server=nas,share=dokumente".to_string(),
                    "sftp:host=server,user=someone".to_string(),
                ]
            } else {
                Vec::new()
            }
        };

        let places = network_places(&mounts, &listing);
        let names: Vec<&str> = places.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["dokumente auf nas", "server (SFTP)", "fotos auf nas"],
            "the gvfs mount itself must not be a row called gvfs"
        );

        // Every one of them: no capacity, and a path a client can actually open.
        for place in &places {
            assert_eq!(place.kind, PlaceKind::Network);
            assert!(
                place.capacity.is_none(),
                "statvfs on a share is one of the calls that blocks; the first screen must not \
                 make it"
            );
            assert!(place.path.is_absolute());
        }
        assert_eq!(
            places[0].path,
            PathBuf::from("/run/user/1000/gvfs/smb-share:server=nas,share=dokumente")
        );
        // The address under the name is the one somebody would recognise, not gvfs's spelling.
        assert_eq!(places[0].device.as_deref(), Some("smb://nas/dokumente"));
        assert_eq!(places[1].device.as_deref(), Some("sftp://server"));
    }

    #[test]
    fn a_share_under_mnt_is_not_also_offered_as_a_drive() {
        let home = tempfile::tempdir().unwrap();
        let mounts = vec![
            mount("/dev/nvme0n1p2", "/", "ext4"),
            // Mounting a share under /mnt is ordinary, and /mnt is where removable media is
            // *also* offered from — which is exactly how it appeared twice.
            mount("//nas/fotos", "/mnt/fotos", "cifs"),
        ];

        let sections = build(home.path(), &mounts, &HashMap::new(), &[], &nothing);
        let network = sections
            .iter()
            .find(|s| s.title == "Netzwerk")
            .expect("a share is a network location");
        assert_eq!(network.places.len(), 1);
        assert!(
            sections.iter().all(|s| s.title != "Laufwerke"),
            "the share was offered as a drive as well"
        );
    }

    /// The listing behind the gvfs rows: sorted, and it is the one call on this screen with a
    /// deadline on it (`answers_within`, whose own test covers what a stuck call costs).
    #[test]
    fn the_shares_are_listed_in_a_stable_order() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["sftp:host=zeta", "smb-share:server=alpha,share=x"] {
            std::fs::create_dir(dir.path().join(name)).unwrap();
        }
        assert_eq!(
            list_dir(dir.path()),
            vec![
                "sftp:host=zeta".to_string(),
                "smb-share:server=alpha,share=x".to_string()
            ],
            "read_dir returns whatever order the filesystem feels like; two runs of one screen              must not disagree"
        );
        // A directory that is not there is an empty screen rather than an error.
        assert!(list_dir(&dir.path().join("nope")).is_empty());
    }
}
