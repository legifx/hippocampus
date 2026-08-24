//! What is mounted, and which of it is somewhere else.
//!
//! This lives in `cortex-core` because two crates now need the same answer for different
//! reasons, and two parsers of `/proc/mounts` would be two opinions about what a network
//! filesystem is:
//!
//! - The **places** screen offers network locations as a section of their own, so a share
//!   mounted by the desktop is somewhere a person can go (M6).
//! - The **indexer** must not walk into one. A share is somebody else's disk reached over
//!   somebody else's network: crawling it turns "index my home directory" into a week of
//!   traffic against a server nobody asked about, and every `stat` in it can block for as long
//!   as the network takes to admit it is down.
//!
//! Read from `/proc/mounts` rather than asked of a daemon, like everything else on this path:
//! the first screen must not wait for D-Bus.

use std::path::{Path, PathBuf};

/// Where the kernel publishes the mount table.
pub const PROC_MOUNTS: &str = "/proc/mounts";

/// One mounted filesystem, as `/proc/mounts` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub device: String,
    pub point: PathBuf,
    pub fstype: String,
}

/// Filesystems whose contents are on another machine.
///
/// By type rather than by name, and listed rather than pattern-matched: `fuse.sshfs` and
/// `fuse.gvfsd-fuse` are network filesystems while `fuse.exfat` on a stick in the machine's own
/// USB port is not, and a rule like "anything with fuse in it" would refuse to index a memory
/// card. `overlay` and `squashfs` are local and belong nowhere near this list.
const NETWORK_FSTYPES: &[&str] = &[
    "cifs",
    "smb3",
    "smbfs",
    "nfs",
    "nfs4",
    "afs",
    "afp",
    "9p",
    "ceph",
    "glusterfs",
    "davfs",
    "fuse.davfs",
    "fuse.sshfs",
    "sshfs",
    "fuse.rclone",
    "fuse.s3fs",
    "fuse.gvfsd-fuse",
    "fuse.gvfs-fuse-daemon",
    "fuse.curlftpfs",
];

/// Is what is behind this mount on another machine?
pub fn is_network(fstype: &str) -> bool {
    NETWORK_FSTYPES.contains(&fstype)
}

/// The mount a path is on, if it is on a network one.
///
/// The longest matching mount point wins, which is what nesting means: a share mounted inside a
/// directory that is itself a mount is on the inner one.
pub fn network_mount_for<'a>(path: &Path, mounts: &'a [Mount]) -> Option<&'a Mount> {
    let mut best: Option<&Mount> = None;
    for mount in mounts {
        if !is_network(&mount.fstype) || !path.starts_with(&mount.point) {
            continue;
        }
        let longer = match best {
            None => true,
            Some(current) => mount.point.as_os_str().len() > current.point.as_os_str().len(),
        };
        if longer {
            best = Some(mount);
        }
    }
    best
}

/// The network mount points a walk from `root` must not cross.
///
/// A root that is *itself* on a share is not fenced in: configuring one is saying so on purpose,
/// and a tool that refused to index what its owner pointed it at would be overruling them. The
/// barrier is for the shares nobody chose — the one mounted inside a home directory that is
/// being indexed because it is a home directory.
pub fn barriers_for(root: &Path, mounts: &[Mount]) -> Vec<PathBuf> {
    let mut barriers = Vec::new();
    for point in network_points(mounts) {
        if root.starts_with(&point) {
            continue;
        }
        barriers.push(point);
    }
    barriers
}

/// Every network mount point, for a walk that has to stop at them.
pub fn network_points(mounts: &[Mount]) -> Vec<PathBuf> {
    let mut points = Vec::new();
    for mount in mounts {
        if is_network(&mount.fstype) {
            points.push(mount.point.clone());
        }
    }
    points
}

/// How long a network path gets to answer before it is called unreachable.
///
/// Two seconds is long enough for a share on a local network that is merely busy, and short
/// enough that a person who unplugged the cable gets an answer rather than a frozen window.
pub const PROBE: std::time::Duration = std::time::Duration::from_millis(2000);

/// Run something that might never come back, and give up on it.
///
/// A `stat` on a share whose server has gone is not slow — it is *stuck*, in the kernel, until
/// the mount's own timeout expires, which for a default NFS mount is minutes. Nothing in user
/// space can cancel it, so this does the only thing that is actually available: the call happens
/// on a thread of its own, the caller waits for a while, and if the answer has not arrived the
/// caller stops waiting.
///
/// **The thread is left behind**, deliberately and with its cost stated: it holds one stack and
/// one blocked syscall until the kernel returns, and then it ends. The alternative — waiting for
/// it — is the frozen file manager this exists to prevent. What must not happen is one of these
/// per row: callers probe a mount, not a file.
pub fn answers_within<T: Send + 'static>(
    timeout: std::time::Duration,
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // The receiver is gone when the caller has given up; sending then fails, and that is
        // the expected end of this thread rather than an error.
        let _ = tx.send(work());
    });
    rx.recv_timeout(timeout).ok()
}

pub fn read(path: &Path) -> Vec<Mount> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    parse(&text)
}

/// Parse `/proc/mounts`.
///
/// Five fields, space-separated, with spaces and tabs in paths written as octal escapes — which
/// is why a stick called `My Stick` mounts at `/media/user/My\040Stick` and a naive split gives
/// six fields and a wrong answer.
pub fn parse(text: &str) -> Vec<Mount> {
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

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
/dev/nvme0n1p2 / ext4 rw,relatime 0 0
//server/photos /home/user/Bilder\\040vom\\040Server cifs rw 0 0
tmpfs /run/user/1000 tmpfs rw 0 0
gvfsd-fuse /run/user/1000/gvfs fuse.gvfsd-fuse rw,user_id=1000 0 0
/dev/sdb1 /media/someone/stick fuse.exfat rw 0 0
";

    #[test]
    fn a_mount_point_with_a_space_in_it_is_one_field() {
        let mounts = parse(SAMPLE);
        let cifs = &mounts[1];
        assert_eq!(cifs.point, PathBuf::from("/home/user/Bilder vom Server"));
        assert_eq!(cifs.device, "//server/photos");
    }

    #[test]
    fn what_counts_as_network_is_the_type_and_not_the_word_fuse() {
        let mounts = parse(SAMPLE);
        let points = network_points(&mounts);
        assert_eq!(
            points,
            vec![
                PathBuf::from("/home/user/Bilder vom Server"),
                PathBuf::from("/run/user/1000/gvfs"),
            ],
            "a stick with a FUSE driver is not a network location, and a gvfs mount is"
        );
    }

    #[test]
    fn a_walk_stops_at_a_share_unless_the_walk_started_inside_one() {
        let mounts = parse(SAMPLE);
        let home = Path::new("/home/user");

        assert_eq!(
            barriers_for(home, &mounts),
            vec![
                PathBuf::from("/home/user/Bilder vom Server"),
                PathBuf::from("/run/user/1000/gvfs"),
            ],
            "a share inside an indexed directory is exactly the case this exists for"
        );

        // Somebody who configures a share as a root has asked for it to be indexed.
        assert!(
            barriers_for(Path::new("/home/user/Bilder vom Server/2026"), &mounts)
                .iter()
                .all(|b| !b.ends_with("Bilder vom Server")),
            "the root's own share was fenced off from itself"
        );
    }

    #[test]
    fn something_that_never_answers_is_given_up_on() {
        let quick = answers_within(std::time::Duration::from_millis(500), || 7);
        assert_eq!(quick, Some(7));

        let started = std::time::Instant::now();
        let stuck = answers_within(std::time::Duration::from_millis(80), || {
            std::thread::sleep(std::time::Duration::from_secs(30));
            7
        });
        assert_eq!(stuck, None);
        // The caller comes back on its own schedule rather than the stuck call's. Generous
        // against a loaded machine, and still two orders of magnitude below the thirty seconds
        // it would have waited.
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "the caller waited {:?} for something it had given up on",
            started.elapsed()
        );
    }

    #[test]
    fn a_path_finds_the_share_it_is_on_and_the_innermost_one() {
        let mut mounts = parse(SAMPLE);
        mounts.push(Mount {
            device: "sftp:host=elsewhere".into(),
            point: PathBuf::from("/run/user/1000/gvfs/sftp:host=elsewhere"),
            fstype: "fuse.sshfs".into(),
        });

        let inner = network_mount_for(
            Path::new("/run/user/1000/gvfs/sftp:host=elsewhere/notes.md"),
            &mounts,
        );
        assert_eq!(inner.map(|m| m.fstype.as_str()), Some("fuse.sshfs"));

        // And a path on the local disk is on no share at all.
        assert!(network_mount_for(Path::new("/home/user/notes.md"), &mounts).is_none());
    }
}
