//! The drives that are *not* mounted, and the three things one can do to a drive.
//!
//! [`crate::places`] lists what `/proc/mounts` says is mounted. A stick that has just been
//! plugged in is not in that file, and for a file manager that is the interesting moment: the
//! whole reason somebody looks at the places screen after plugging something in is to open it.
//!
//! ## Reading is files; acting is `udisksctl`
//!
//! Finding the volumes needs no privileges and no D-Bus. `/sys/class/block` says what block
//! devices exist and how big they are, the disk above a partition says whether it is removable,
//! and udev's own database in `/run/udev/data/b<major>:<minor>` — world-readable, one
//! `E:KEY=value` per line — says what filesystem is on it and what it is called. That is the
//! same shape as everything in [`crate::places`]: read the files the system already keeps, and
//! stay out of the first screen's latency budget.
//!
//! **Mounting is different, and it is the one thing here that is not a file.** It needs
//! privileges this daemon does not have and must not have. `udisksctl` is the part of udisks2
//! that already knows how to ask: it talks to the system service over D-Bus, the service asks
//! polkit, and polkit says yes for a removable drive belonging to the person at the keyboard.
//! Shelling out to it rather than linking a D-Bus crate keeps the daemon's dependency list and
//! its startup time where they are, and it is the same program every desktop uses.
//!
//! `--no-user-interaction` on every call, without exception: a daemon must never sit waiting on
//! an authentication dialog that nobody can see. If the operation needs authorisation that is
//! not already granted, it fails immediately and says so, which is an answer a client can show.

use crate::places::Mount;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Where the kernel describes block devices.
pub const SYS_BLOCK: &str = "/sys/class/block";
/// Where udev keeps what it worked out about each of them.
pub const UDEV_DATA: &str = "/run/udev/data";

/// Filesystems worth offering to mount.
///
/// An allow-list rather than a deny-list, for the same reason the write policy is one: a
/// filesystem nobody has thought about is not offered, instead of being offered and failing.
const MOUNTABLE: &[&str] = &[
    "ext2", "ext3", "ext4", "xfs", "btrfs", "f2fs", "jfs", "reiserfs", "vfat", "exfat", "ntfs",
    "ntfs3", "hfs", "hfsplus", "iso9660", "udf",
];

/// Partition types that are machinery rather than places.
///
/// The EFI system partition and Microsoft's reserved partition both hold a filesystem and
/// neither is somewhere a person goes. Offering them is how a places screen fills up with rows
/// that do nothing but confuse — and mounting an ESP by accident is a way to damage a boot.
const NOT_PLACES: &[&str] = &[
    // GPT
    "c12a7328-f81f-11d2-ba4b-00a0c93ec93b", // EFI system
    "e3c9e316-b5c2-4bd4-a6d2-6d6d6d6d6d6d", // Microsoft reserved
    "21686148-6449-6e6f-744e-656564454649", // BIOS boot
    "de94bba4-06d1-4d40-a16a-bfd50179d6ac", // Windows recovery
    // MBR
    "0xef", // EFI system
    "0x82", // Linux swap
    "0x27", // Windows recovery
];

/// What the list above cannot do, said plainly: on an MBR disk a Windows system partition and a
/// Windows data partition are both type `0x7`, so "System-reserviert" is offered like any other
/// NTFS volume. Its label is the only thing that says otherwise, and inventing a rule from a
/// German label is worse than showing one row too many. Nautilus shows it too.

/// Device names that are never a place: the kernel's own pseudo-block-devices.
const NOT_DEVICES: &[&str] = &["loop", "ram", "zram", "md", "dm-"];

/// One filesystem on one block device, mounted or not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    /// `/dev/sdb1`. What every operation below names, and what udisks2 understands.
    pub device: PathBuf,
    /// What the filesystem calls itself, when it says.
    pub label: Option<String>,
    pub fstype: String,
    pub size_bytes: u64,
    /// Whether the *disk* this lives on can be unplugged. A partition is not removable; the
    /// stick it is on is, and that is the property that decides whether ejecting means
    /// anything.
    pub removable: bool,
    /// Where it is mounted, if it is.
    pub mount_point: Option<PathBuf>,
}

impl Volume {
    /// What to call it in a list. The label if it has one, otherwise its size and kind — never
    /// the device node, which is what a places screen exists to replace.
    pub fn name(&self) -> String {
        match &self.label {
            Some(label) if !label.is_empty() => label.clone(),
            _ => format!("{} {}", human_size(self.size_bytes), self.fstype),
        }
    }
}

/// Every volume the machine has, mounted or not, in a stable order.
pub fn scan(sys_block: &Path, udev_data: &Path, mounts: &[Mount]) -> Vec<Volume> {
    let Ok(entries) = std::fs::read_dir(sys_block) else {
        return Vec::new();
    };

    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    // The directory order is the kernel's and changes between boots; a places screen that
    // reorders itself for no reason is one nobody can build muscle memory for.
    names.sort();

    let mut volumes = Vec::new();
    for name in names {
        if NOT_DEVICES.iter().any(|prefix| name.starts_with(prefix)) {
            continue;
        }
        if let Some(volume) = read_volume(sys_block, udev_data, &name, mounts) {
            volumes.push(volume);
        }
    }
    volumes
}

/// The volumes that are not mounted, which is what [`crate::places`] cannot see for itself.
pub fn unmounted(sys_block: &Path, udev_data: &Path, mounts: &[Mount]) -> Vec<Volume> {
    let mut found: Vec<Volume> = scan(sys_block, udev_data, mounts)
        .into_iter()
        .filter(|v| v.mount_point.is_none())
        .collect();
    // Removable first: a stick that was just plugged in is what somebody is looking for, and an
    // internal Windows partition has been there for a year.
    found.sort_by_key(|v| (!v.removable, v.name()));
    found
}

fn read_volume(
    sys_block: &Path,
    udev_data: &Path,
    name: &str,
    mounts: &[Mount],
) -> Option<Volume> {
    let dir = sys_block.join(name);
    let dev = read_trimmed(&dir.join("dev"))?;
    let properties = read_udev(udev_data, &dev);

    if properties.get("ID_FS_USAGE").map(String::as_str) != Some("filesystem") {
        return None;
    }
    let fstype = properties.get("ID_FS_TYPE")?.clone();
    if !MOUNTABLE.contains(&fstype.as_str()) {
        return None;
    }
    if let Some(part_type) = properties.get("ID_PART_ENTRY_TYPE") {
        if NOT_PLACES.contains(&part_type.to_ascii_lowercase().as_str()) {
            return None;
        }
    }

    // `size` is in 512-byte sectors whatever the device's own block size is — a kernel interface
    // old enough that the unit is part of it.
    let size_bytes = read_trimmed(&dir.join("size"))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
        * 512;

    // A partition's own `removable` is always 0; the property belongs to the disk above it.
    let removable = read_trimmed(&dir.join("../removable"))
        .or_else(|| read_trimmed(&dir.join("removable")))
        .map(|v| v == "1")
        .unwrap_or(false)
        || properties.get("ID_BUS").map(String::as_str) == Some("usb");

    let device = PathBuf::from(format!("/dev/{name}"));
    let mount_point = mounts
        .iter()
        .find(|m| m.device == device.to_string_lossy())
        .map(|m| m.point.clone());

    Some(Volume {
        label: properties.get("ID_FS_LABEL").cloned(),
        device,
        fstype,
        size_bytes,
        removable,
        mount_point,
    })
}

/// udev's own record for a device, as `E:KEY=value` lines.
///
/// Read rather than asked for over a socket: this is the file `udevadm info` prints, and reading
/// it costs an open and a few hundred bytes.
fn read_udev(udev_data: &Path, major_minor: &str) -> HashMap<String, String> {
    let mut properties = HashMap::new();
    let Ok(text) = std::fs::read_to_string(udev_data.join(format!("b{major_minor}"))) else {
        return properties;
    };
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("E:") else {
            continue;
        };
        if let Some((key, value)) = rest.split_once('=') {
            properties.insert(key.to_string(), unescape(value));
        }
    }
    properties
}

/// udev escapes bytes it does not like as `\x20`. A label with a space in it is common enough
/// that leaving them in would show `My\x20Stick` on the first screen.
fn unescape(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = String::with_capacity(value.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() && bytes[i + 1] == b'x' {
            if let Ok(byte) = u8::from_str_radix(&value[i + 2..i + 4], 16) {
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

fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// Sizes as a person reads them. Only used for naming a volume that has no label.
fn human_size(bytes: u64) -> String {
    const UNITS: [(&str, u64); 4] = [
        ("TB", 1_000_000_000_000),
        ("GB", 1_000_000_000),
        ("MB", 1_000_000),
        ("kB", 1_000),
    ];
    for (unit, scale) in UNITS {
        if bytes >= scale {
            return format!("{:.0} {unit}", bytes as f64 / scale as f64);
        }
    }
    format!("{bytes} B")
}

// ---------------------------------------------------------------------------
// Acting on a drive
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum VolumeError {
    #[error("{0} is not a block device on this machine")]
    NoSuchDevice(PathBuf),
    #[error("udisks2 is not installed, so mounting is not available here")]
    NoUdisks,
    #[error("{operation} failed: {message}")]
    Refused { operation: &'static str, message: String },
    #[error("{0}")]
    Io(String),
}

/// What an unmount request comes to, for this volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unmounting {
    /// It is not mounted and nothing was asked beyond that.
    Nothing,
    Unmount,
    /// Unmount if it is mounted, then cut the power so it can be pulled out.
    Eject,
}

/// Decide what to do, or refuse — separately from doing it, so the refusal has a test.
///
/// **The rule this exists for: `power-off` on a disk that cannot be unplugged.** It is one
/// argument away from the ordinary case and it takes an internal disk off the bus, which on a
/// running system means a filesystem that disappears underneath everything using it. Whether a
/// disk can be unplugged is read from the kernel and never inferred from the mount point, and
/// the check is here rather than inside the doing so that deleting it fails a named test.
pub fn plan_unmount(volume: &Volume, eject: bool) -> Result<Unmounting, VolumeError> {
    if eject && !volume.removable {
        return Err(VolumeError::Refused {
            operation: "eject",
            message: format!(
                "{} is on a disk that cannot be unplugged, so it will not be powered off",
                volume.device.display()
            ),
        });
    }
    Ok(match (eject, volume.mount_point.is_some()) {
        (true, _) => Unmounting::Eject,
        (false, true) => Unmounting::Unmount,
        (false, false) => Unmounting::Nothing,
    })
}

/// The part of udisks2 that already knows how to ask for privileges.
///
/// A struct rather than three free functions so that the tests can point it at a script instead
/// of at the real thing — the alternative is a suite that only passes on a machine with a stick
/// plugged into it.
pub struct Udisks {
    program: String,
}

impl Default for Udisks {
    fn default() -> Self {
        Self {
            program: "udisksctl".to_string(),
        }
    }
}

impl Udisks {
    pub fn using(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
        }
    }

    /// Mount it, and answer with where it landed.
    ///
    /// The mount point is udisks2's to choose — `/run/media/<user>/<label>` by convention — and
    /// asking for a particular one is how a file manager ends up disagreeing with every other
    /// program about where the stick is.
    pub fn mount(&self, device: &Path) -> Result<PathBuf, VolumeError> {
        let output = self.run("mount", &["-b", &device.to_string_lossy()])?;
        parse_mount_point(&output).ok_or(VolumeError::Refused {
            operation: "mount",
            message: format!("udisks2 mounted it but did not say where: {output}"),
        })
    }

    pub fn unmount(&self, device: &Path) -> Result<(), VolumeError> {
        self.run("unmount", &["-b", &device.to_string_lossy()])?;
        Ok(())
    }

    /// Unmount, then cut the power to the drive it is on, so it can be unplugged safely.
    ///
    /// Two steps and one word, because that is what a person means by "eject": a drive that is
    /// unmounted but still spinning is not what the safe-to-remove indicator on their desk says.
    /// The power-off applies to the whole disk, which is why it is refused for anything that
    /// cannot be unplugged — powering off an internal disk is a way to lose a machine.
    pub fn eject(&self, device: &Path, disk: &Path, mounted: bool) -> Result<(), VolumeError> {
        if mounted {
            self.unmount(device)?;
        }
        self.run("power-off", &["-b", &disk.to_string_lossy()])?;
        Ok(())
    }

    fn run(&self, operation: &'static str, args: &[&str]) -> Result<String, VolumeError> {
        let mut command = std::process::Command::new(&self.program);
        command.arg(operation).args(args);
        // Never, on any path. A daemon waiting on a dialog nobody can see is a daemon that has
        // stopped answering.
        command.arg("--no-user-interaction");

        let output = match command.output() {
            Ok(output) => output,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(VolumeError::NoUdisks)
            }
            Err(e) => return Err(VolumeError::Io(e.to_string())),
        };

        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).to_string());
        }
        // udisks2's own words. They are written for a person — "Object /org/... is not a mountable
        // filesystem", "Not authorized to perform operation" — and rewriting them into something
        // of our own would lose the only description of what actually went wrong.
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(VolumeError::Refused {
            operation,
            message: if message.is_empty() {
                format!("{} exited with {}", self.program, output.status)
            } else {
                message
            },
        })
    }
}

/// Where `udisksctl mount` says it put it.
///
/// The line is `Mounted /dev/sdb1 at /run/media/user/LABEL.` — with the full stop, and with a
/// mount point that may contain spaces, so it is taken as everything between " at " and the end
/// rather than by splitting on whitespace.
pub fn parse_mount_point(output: &str) -> Option<PathBuf> {
    for line in output.lines() {
        let line = line.trim();
        let Some((_, at)) = line.split_once(" at ") else {
            continue;
        };
        let path = at.trim_end_matches('.').trim();
        if path.starts_with('/') {
            return Some(PathBuf::from(path));
        }
    }
    None
}

/// The disk a partition lives on: `/dev/sdb1` → `/dev/sdb`, `/dev/nvme0n1p2` → `/dev/nvme0n1`.
///
/// Read from `/sys` rather than by stripping digits, because stripping digits turns
/// `nvme0n1p2` into `nvme0n1p` and then into nothing at all.
pub fn disk_of(sys_block: &Path, device: &Path) -> Option<PathBuf> {
    let name = device.file_name()?.to_string_lossy().to_string();
    let dir = sys_block.join(&name);
    if !dir.join("partition").exists() {
        // Not a partition: the device is its own disk.
        return Some(device.to_path_buf());
    }
    let resolved = std::fs::canonicalize(&dir).ok()?;
    let parent = resolved.parent()?.file_name()?.to_string_lossy().to_string();
    Some(PathBuf::from(format!("/dev/{parent}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A machine's block devices, invented.
    ///
    /// The real `/sys` and `/run/udev` were used to *find* the rules — a stick, three Windows
    /// partitions, an EFI partition and six loop devices, on the machine this was written on —
    /// and every one of those cases is reproduced here, because a suite that reads the real ones
    /// asserts what happens to be plugged in today.
    struct Machine {
        root: tempfile::TempDir,
    }

    impl Machine {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(root.path().join("sys")).unwrap();
            std::fs::create_dir_all(root.path().join("udev")).unwrap();
            Self { root }
        }

        fn sys(&self) -> PathBuf {
            self.root.path().join("sys")
        }

        fn udev(&self) -> PathBuf {
            self.root.path().join("udev")
        }

        /// A whole disk. `removable` is the property the kernel puts here and nowhere else.
        fn disk(&self, name: &str, removable: bool) -> &Self {
            let dir = self.sys().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("removable"), if removable { "1\n" } else { "0\n" }).unwrap();
            std::fs::write(dir.join("dev"), "8:0\n").unwrap();
            self
        }

        /// A partition on it, with whatever udev worked out about the filesystem.
        ///
        /// Laid out the way `/sys/class/block` really is — the partition directory *inside* the
        /// disk's — because the code reads `..` to find out whether the disk can be unplugged,
        /// and a flat fixture would let that go wrong unnoticed.
        fn part(&self, disk: &str, name: &str, dev: &str, sectors: u64, udev: &str) -> &Self {
            let dir = self.sys().join(disk).join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("dev"), format!("{dev}\n")).unwrap();
            std::fs::write(dir.join("size"), format!("{sectors}\n")).unwrap();
            std::fs::write(dir.join("partition"), "1\n").unwrap();
            // `/sys/class/block/sdb1` is a link to the partition inside the disk.
            std::os::unix::fs::symlink(&dir, self.sys().join(name)).unwrap();
            std::fs::write(self.udev().join(format!("b{dev}")), udev).unwrap();
            self
        }

        fn found(&self, mounts: &[Mount]) -> Vec<Volume> {
            scan(&self.sys(), &self.udev(), mounts)
        }
    }

    fn mount(device: &str, point: &str) -> Mount {
        Mount {
            device: device.to_string(),
            point: PathBuf::from(point),
            fstype: "ext4".to_string(),
        }
    }

    const STICK: &str = "E:ID_FS_USAGE=filesystem\nE:ID_FS_TYPE=vfat\nE:ID_FS_LABEL=ESD-USB\nE:ID_PART_ENTRY_TYPE=0xc\nE:ID_BUS=usb\n";
    const WINDOWS: &str = "E:ID_FS_USAGE=filesystem\nE:ID_FS_TYPE=ntfs\nE:ID_FS_LABEL=Videos\nE:ID_PART_ENTRY_TYPE=0x7\n";

    #[test]
    fn a_stick_that_is_not_mounted_is_still_found() {
        // The whole point of the module: `/proc/mounts` cannot see this, and it is exactly what
        // somebody who has just plugged something in is looking for.
        let m = Machine::new();
        m.disk("sdb", true).part("sdb", "sdb1", "8:17", 61_437_952, STICK);

        let found = m.found(&[]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].device, PathBuf::from("/dev/sdb1"));
        assert_eq!(found[0].label.as_deref(), Some("ESD-USB"));
        assert_eq!(found[0].fstype, "vfat");
        assert_eq!(found[0].size_bytes, 61_437_952 * 512);
        assert!(found[0].removable, "a partition on a removable disk is removable");
        assert_eq!(found[0].mount_point, None);
    }

    #[test]
    fn a_mounted_volume_carries_where_it_is() {
        let m = Machine::new();
        m.disk("sdb", true).part("sdb", "sdb1", "8:17", 1000, STICK);
        let found = m.found(&[mount("/dev/sdb1", "/run/media/user/ESD-USB")]);
        assert_eq!(found[0].mount_point, Some(PathBuf::from("/run/media/user/ESD-USB")));
        assert!(unmounted(&m.sys(), &m.udev(), &[mount("/dev/sdb1", "/x")]).is_empty());
    }

    #[test]
    fn an_internal_partition_is_offered_and_is_not_removable() {
        // A Windows partition somebody has not mounted is a place they may well want to open —
        // Nautilus offers it, and so does this.
        let m = Machine::new();
        m.disk("sda", false).part("sda", "sda2", "8:2", 976_000_000, WINDOWS);
        let found = m.found(&[]);
        assert_eq!(found[0].label.as_deref(), Some("Videos"));
        assert!(!found[0].removable, "an internal disk cannot be ejected, and must not offer to be");
    }

    #[test]
    fn the_machinery_is_not_offered() {
        let m = Machine::new();
        m.disk("nvme0n1", false)
            .part("nvme0n1", "nvme0n1p1", "259:1", 200_000,
                  "E:ID_FS_USAGE=filesystem\nE:ID_FS_TYPE=vfat\nE:ID_PART_ENTRY_TYPE=c12a7328-f81f-11d2-ba4b-00a0c93ec93b\n")
            .part("nvme0n1", "nvme0n1p2", "259:2", 200_000,
                  "E:ID_FS_USAGE=filesystem\nE:ID_FS_TYPE=ntfs\nE:ID_PART_ENTRY_TYPE=de94bba4-06d1-4d40-a16a-bfd50179d6ac\n")
            .part("nvme0n1", "nvme0n1p3", "259:3", 200_000,
                  "E:ID_FS_USAGE=other\nE:ID_FS_TYPE=swap\n")
            .part("nvme0n1", "nvme0n1p4", "259:4", 200_000,
                  "E:ID_FS_USAGE=crypto\nE:ID_FS_TYPE=crypto_LUKS\n");
        assert!(m.found(&[]).is_empty(), "an ESP, a recovery partition, swap and a locked volume are not places");
    }

    #[test]
    fn the_kernels_own_block_devices_are_not_drives() {
        // Six snap loop devices on the machine this was written on, every one of them carrying a
        // squashfs that udev reports as a filesystem. Without this rule the first screen would
        // open with them.
        let m = Machine::new();
        m.disk("loop0", false).part("loop0", "loop0p1", "7:0", 8,
            "E:ID_FS_USAGE=filesystem\nE:ID_FS_TYPE=squashfs\n");
        std::fs::create_dir_all(m.sys().join("loop1")).unwrap();
        std::fs::write(m.sys().join("loop1/dev"), "7:1\n").unwrap();
        std::fs::write(m.udev().join("b7:1"), "E:ID_FS_USAGE=filesystem\nE:ID_FS_TYPE=squashfs\n").unwrap();
        assert!(m.found(&[]).is_empty());
    }

    #[test]
    fn a_filesystem_nobody_has_thought_about_is_not_offered() {
        let m = Machine::new();
        m.disk("sdc", true).part("sdc", "sdc1", "8:33",
            1000, "E:ID_FS_USAGE=filesystem\nE:ID_FS_TYPE=zfs_member\n");
        assert!(m.found(&[]).is_empty(), "the list of mountable filesystems is an allow-list");
    }

    #[test]
    fn a_label_with_a_space_in_it_is_readable() {
        // udev escapes the space; showing `My\x20Stick` on the first screen is the failure this
        // catches, and it is invisible to anybody testing with a one-word label.
        let m = Machine::new();
        m.disk("sdb", true).part("sdb", "sdb1", "8:17", 1000,
            "E:ID_FS_USAGE=filesystem\nE:ID_FS_TYPE=vfat\nE:ID_FS_LABEL=My\\x20Stick\n");
        assert_eq!(m.found(&[])[0].label.as_deref(), Some("My Stick"));
    }

    #[test]
    fn a_volume_with_no_label_is_named_by_what_it_is() {
        // Never the device node: `/dev/sdc1` is the thing a places screen exists to replace.
        let m = Machine::new();
        m.disk("sdc", true).part("sdc", "sdc1", "8:33", 61_437_952,
            "E:ID_FS_USAGE=filesystem\nE:ID_FS_TYPE=exfat\n");
        assert_eq!(m.found(&[])[0].name(), "31 GB exfat");
    }

    #[test]
    fn removable_drives_come_first() {
        let m = Machine::new();
        m.disk("sda", false).part("sda", "sda2", "8:2", 1000, WINDOWS);
        m.disk("sdb", true).part("sdb", "sdb1", "8:17", 1000, STICK);
        let order: Vec<String> = unmounted(&m.sys(), &m.udev(), &[])
            .into_iter()
            .map(|v| v.name())
            .collect();
        assert_eq!(order, ["ESD-USB", "Videos"], "the stick just plugged in is what is being looked for");
    }

    #[test]
    fn the_order_does_not_depend_on_the_order_the_kernel_lists_them() {
        // `read_dir` order is the filesystem's and changes between boots. A places screen that
        // reshuffles itself is one nobody can learn.
        let m = Machine::new();
        m.disk("sdb", true)
            .part("sdb", "sdb1", "8:17", 1000, STICK)
            .part("sdb", "sdb2", "8:18", 1000, "E:ID_FS_USAGE=filesystem\nE:ID_FS_TYPE=ext4\nE:ID_FS_LABEL=Zweites\n");
        let once: Vec<PathBuf> = m.found(&[]).into_iter().map(|v| v.device).collect();
        let twice: Vec<PathBuf> = m.found(&[]).into_iter().map(|v| v.device).collect();
        assert_eq!(once, twice);
        assert_eq!(once, [PathBuf::from("/dev/sdb1"), PathBuf::from("/dev/sdb2")]);
    }

    // -----------------------------------------------------------------------
    // Acting
    // -----------------------------------------------------------------------

    #[test]
    fn an_internal_disk_is_never_powered_off() {
        // The rule with the worst failure: `power-off` on an internal disk takes it off the bus
        // while a running system is using it. One argument separates this from the ordinary
        // case, so it is a decision with a test rather than a line inside the doing.
        let internal = Volume {
            device: PathBuf::from("/dev/sda2"),
            label: Some("Videos".into()),
            fstype: "ntfs".into(),
            size_bytes: 1,
            removable: false,
            mount_point: Some(PathBuf::from("/mnt/videos")),
        };
        let error = plan_unmount(&internal, true).unwrap_err();
        assert!(error.to_string().contains("cannot be unplugged"), "{error}");

        // Unmounting it is fine and reversible; only the power-off is refused.
        assert_eq!(plan_unmount(&internal, false).unwrap(), Unmounting::Unmount);
    }

    #[test]
    fn a_removable_drive_can_be_ejected_whether_or_not_it_is_mounted() {
        // A stick that was unmounted a minute ago still has power, and "eject" means the
        // indicator on the desk, not the mount table.
        let mut stick = Volume {
            device: PathBuf::from("/dev/sdb1"),
            label: None,
            fstype: "vfat".into(),
            size_bytes: 1,
            removable: true,
            mount_point: None,
        };
        assert_eq!(plan_unmount(&stick, true).unwrap(), Unmounting::Eject);
        assert_eq!(plan_unmount(&stick, false).unwrap(), Unmounting::Nothing);

        stick.mount_point = Some(PathBuf::from("/run/media/user/x"));
        assert_eq!(plan_unmount(&stick, false).unwrap(), Unmounting::Unmount);
    }

    #[test]
    fn the_mount_point_is_read_out_of_what_udisks_says() {
        assert_eq!(
            parse_mount_point("Mounted /dev/sdb1 at /run/media/user/ESD-USB.\n"),
            Some(PathBuf::from("/run/media/user/ESD-USB"))
        );
        // A label with a space in it makes a mount point with a space in it, and splitting on
        // whitespace would return `/run/media/user/My`.
        assert_eq!(
            parse_mount_point("Mounted /dev/sdb1 at /run/media/user/My Stick.\n"),
            Some(PathBuf::from("/run/media/user/My Stick"))
        );
        assert_eq!(parse_mount_point("nothing useful\n"), None);
    }

    /// A fake `udisksctl`, so the behaviour around it can be asserted without a drive.
    ///
    /// The wait at the end is not superstition. Tests in one binary run on many threads, and a
    /// thread that forks while another is still holding a freshly written script open makes the
    /// child inherit that write handle — the exec then fails with `ETXTBSY`, in a test that has
    /// nothing to do with the one that wrote the file. So the fixture proves its own script is
    /// runnable before handing it over, and a failure afterwards is about the code under test.
    fn fake_udisks(dir: &Path, script: &str) -> Udisks {
        let path = dir.join("udisksctl");
        std::fs::write(&path, script).unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
        std::fs::set_permissions(&path, permissions).unwrap();

        for attempt in 0..200 {
            match std::process::Command::new(&path).arg("--ready").output() {
                Ok(_) => break,
                Err(e) if e.raw_os_error() == Some(26) && attempt < 199 => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(e) => panic!("the fixture's own script will not run: {e}"),
            }
        }
        Udisks::using(path.to_string_lossy().to_string())
    }

    #[test]
    fn a_mount_answers_with_where_it_landed() {
        let dir = tempfile::tempdir().unwrap();
        let udisks = fake_udisks(
            dir.path(),
            "#!/bin/sh\necho \"Mounted $3 at /run/media/user/ESD-USB.\"\n",
        );
        assert_eq!(
            udisks.mount(Path::new("/dev/sdb1")).unwrap(),
            PathBuf::from("/run/media/user/ESD-USB")
        );
    }

    #[test]
    fn every_call_says_that_nobody_can_answer_a_dialog() {
        // A daemon waiting on a polkit prompt nobody can see is a daemon that has stopped
        // answering. The flag is asserted rather than trusted because it is invisible when it
        // works and fatal when it is missing.
        let dir = tempfile::tempdir().unwrap();
        let seen = dir.path().join("argv");
        let udisks = fake_udisks(
            dir.path(),
            // The readiness probe above calls this too; it names itself so the assertions can
            // ignore it rather than counting it as an operation.
            &format!(
                "#!/bin/sh\n[ \"$1\" = --ready ] && exit 0\necho \"$@\" >> {}\necho \"Mounted /dev/sdb1 at /mnt/x.\"\n",
                seen.display()
            ),
        );
        udisks.mount(Path::new("/dev/sdb1")).unwrap();
        udisks.unmount(Path::new("/dev/sdb1")).unwrap();
        udisks.eject(Path::new("/dev/sdb1"), Path::new("/dev/sdb"), true).unwrap();

        let argv = std::fs::read_to_string(&seen).unwrap();
        assert_eq!(argv.lines().count(), 4, "mount, unmount, and eject's two steps");
        for line in argv.lines() {
            assert!(line.contains("--no-user-interaction"), "{line}");
        }
        assert!(argv.contains("power-off -b /dev/sdb"), "eject powers off the disk, not the partition");
    }

    #[test]
    fn ejecting_something_that_is_not_mounted_does_not_unmount_it_first() {
        let dir = tempfile::tempdir().unwrap();
        let seen = dir.path().join("argv");
        let udisks = fake_udisks(
            dir.path(),
            &format!(
                "#!/bin/sh\n[ \"$1\" = --ready ] && exit 0\necho \"$@\" >> {}\n",
                seen.display()
            ),
        );
        udisks.eject(Path::new("/dev/sdb1"), Path::new("/dev/sdb"), false).unwrap();
        let argv = std::fs::read_to_string(&seen).unwrap();
        assert_eq!(argv.lines().count(), 1);
        assert!(argv.contains("power-off"));
    }

    #[test]
    fn what_udisks_says_when_it_refuses_is_what_the_caller_is_told() {
        // "Not authorized to perform operation" is the only description of what went wrong that
        // anybody has. Replacing it with a message of our own loses it.
        let dir = tempfile::tempdir().unwrap();
        let udisks = fake_udisks(
            dir.path(),
            "#!/bin/sh\necho 'Error mounting /dev/sdb1: Not authorized to perform operation' >&2\nexit 1\n",
        );
        let error = udisks.mount(Path::new("/dev/sdb1")).unwrap_err();
        assert!(error.to_string().contains("Not authorized"), "{error}");
    }

    #[test]
    fn a_machine_without_udisks2_says_so_rather_than_failing_obscurely() {
        let udisks = Udisks::using("/nonexistent/udisksctl");
        assert!(matches!(
            udisks.mount(Path::new("/dev/sdb1")).unwrap_err(),
            VolumeError::NoUdisks
        ));
    }

    #[test]
    fn a_mount_that_says_nothing_is_an_error_rather_than_a_guess() {
        // Reporting success with no mount point would send the window to a directory that is
        // not there.
        let dir = tempfile::tempdir().unwrap();
        let udisks = fake_udisks(dir.path(), "#!/bin/sh\nexit 0\n");
        assert!(udisks.mount(Path::new("/dev/sdb1")).is_err());
    }

    #[test]
    fn the_disk_above_a_partition_is_read_and_not_guessed() {
        let m = Machine::new();
        m.disk("nvme0n1", false).part("nvme0n1", "nvme0n1p2", "259:2", 1000, WINDOWS);
        assert_eq!(
            disk_of(&m.sys(), Path::new("/dev/nvme0n1p2")),
            Some(PathBuf::from("/dev/nvme0n1")),
            "stripping digits would give nvme0n1p, and then nothing"
        );

        // A whole disk with a filesystem straight on it is its own disk.
        m.disk("sdc", true);
        std::fs::write(m.sys().join("sdc/size"), "1000\n").unwrap();
        assert_eq!(disk_of(&m.sys(), Path::new("/dev/sdc")), Some(PathBuf::from("/dev/sdc")));
    }
}
