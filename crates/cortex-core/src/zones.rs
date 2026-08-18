//! Spatial zones: the major areas of the tree, and their colours.
//!
//! Implements decision **D15**. The reasoning is worth restating, because "add some colours"
//! is exactly the kind of feature that decays into noise:
//!
//! - **Colour is derived, never random.** The hue comes from a stable hash of the zone's
//!   identity, so `~/Projects` is the same colour today, next week and on a rebuilt index.
//!   A colour that changes between sessions is noise; a colour that does not becomes a spatial
//!   memory aid, which is the entire point of this project.
//! - **Saturation and lightness are fixed by the theme, so only the hue varies.** No zone can
//!   end up louder than another, and no unlucky hash can produce something that hurts to look at.
//! - **Colour is never the only signal.** Every zone also has a name and an icon. A user who
//!   cannot distinguish two hues loses nothing that matters.
//!
//! This module is pure: no filesystem, no database. Detection lives in `cortex-ingest`.

use std::path::{Path, PathBuf};

/// Hues are snapped to a wheel of this many steps.
///
/// Twelve is chosen so that neighbouring zones are visibly distinct at the low saturation the
/// theme uses. A continuous hue space sounds better and looks worse: two zones landing three
/// degrees apart read as the same colour, which is worse than sharing one outright, because the
/// user cannot tell whether the difference is meaningful.
pub const HUE_STEPS: u32 = 12;

/// A zone as the daemon knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zone {
    pub id: Option<crate::ids::ZoneId>,
    /// The directory this zone covers. A node belongs to the deepest zone containing it.
    pub root_path: PathBuf,
    pub name: String,
    pub color_hue: u16,
    pub icon: Option<String>,
    /// True when the user picked the colour by hand; regeneration must then leave it alone.
    pub user_override: bool,
}

impl Zone {
    /// Build a zone with its colour derived from its own identity.
    pub fn derived(root_path: impl Into<PathBuf>, name: impl Into<String>, icon: Option<String>) -> Self {
        let root_path = root_path.into();
        let name = name.into();
        Self {
            id: None,
            color_hue: hue_for(&root_path),
            root_path,
            name,
            icon,
            user_override: false,
        }
    }
}

/// The hue for a zone root, in degrees.
///
/// Keyed on the path rather than on the database id: an id is an accident of insertion order,
/// so keying on it would repaint the whole tree after a rebuild — precisely the instability
/// this design exists to avoid.
pub fn hue_for(root_path: &Path) -> u16 {
    let step = fnv1a(root_path.as_os_str().as_encoded_bytes()) % HUE_STEPS as u64;
    (step * (360 / HUE_STEPS) as u64) as u16
}

/// FNV-1a, 64-bit.
///
/// Hand-rolled rather than `DefaultHasher` on purpose: `DefaultHasher` is explicitly documented
/// as unstable across Rust releases, which would silently repaint every zone on a toolchain
/// upgrade. This one is fixed forever.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Pick the zone a path belongs to: the deepest root that contains it.
///
/// Depth wins so that a project inside `~/Documents` reads as its own area rather than
/// dissolving into the surrounding one.
pub fn zone_for<'a>(path: &Path, zones: &'a [Zone]) -> Option<&'a Zone> {
    zones
        .iter()
        .filter(|zone| path.starts_with(&zone.root_path))
        .max_by_key(|zone| zone.root_path.components().count())
}

/// A human-readable name for a directory, used when nothing better is known.
pub fn name_for(root_path: &Path) -> String {
    root_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root_path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(path: &str) -> Zone {
        Zone::derived(path, name_for(Path::new(path)), None)
    }

    #[test]
    fn a_hue_is_stable_for_the_same_path() {
        // The whole feature rests on this. If it ever fails, colours have stopped being a
        // memory aid and become decoration.
        let first = hue_for(Path::new("/home/user/Projects"));
        for _ in 0..100 {
            assert_eq!(hue_for(Path::new("/home/user/Projects")), first);
        }
    }

    #[test]
    fn hues_land_on_the_wheel() {
        let step = 360 / HUE_STEPS as u16;
        for path in [
            "/home/user/Documents",
            "/home/user/Projects",
            "/home/user/Pictures",
            "/srv/data",
            "/",
        ] {
            let hue = hue_for(Path::new(path));
            assert!(hue < 360, "{path} produced hue {hue}");
            assert_eq!(hue % step, 0, "{path} produced off-wheel hue {hue}");
        }
    }

    #[test]
    fn different_paths_generally_get_different_hues() {
        // Collisions are possible with twelve steps and are acceptable — the name and icon
        // still distinguish them. What would not be acceptable is everything colliding.
        let paths = [
            "/home/user/Documents",
            "/home/user/Projects",
            "/home/user/Pictures",
            "/home/user/Downloads",
            "/home/user/Music",
            "/home/user/Videos",
        ];
        let hues: std::collections::HashSet<u16> =
            paths.iter().map(|p| hue_for(Path::new(p))).collect();
        assert!(
            hues.len() >= 4,
            "six typical zones collapsed into {} colours: {hues:?}",
            hues.len()
        );
    }

    #[test]
    fn the_hash_is_pinned_so_a_toolchain_upgrade_cannot_repaint_the_tree() {
        // Fixed expected values. If this test fails, the hash changed and every user's zone
        // colours changed with it — which is a breaking change, not an implementation detail.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(b"hippocampus"), fnv1a(b"hippocampus"));
        assert_ne!(fnv1a(b"hippocampus"), fnv1a(b"hippocampu"));
    }

    #[test]
    fn the_deepest_containing_zone_wins() {
        let zones = vec![zone("/home/user"), zone("/home/user/Projects"), zone("/home/user/Projects/aurora")];

        let picked = zone_for(Path::new("/home/user/Projects/aurora/src/main.rs"), &zones).unwrap();
        assert_eq!(picked.root_path, PathBuf::from("/home/user/Projects/aurora"));

        let picked = zone_for(Path::new("/home/user/Projects/other/file.txt"), &zones).unwrap();
        assert_eq!(picked.root_path, PathBuf::from("/home/user/Projects"));

        let picked = zone_for(Path::new("/home/user/notes.md"), &zones).unwrap();
        assert_eq!(picked.root_path, PathBuf::from("/home/user"));
    }

    #[test]
    fn a_path_outside_every_zone_has_none() {
        let zones = vec![zone("/home/user/Projects")];
        assert!(zone_for(Path::new("/srv/elsewhere/file.txt"), &zones).is_none());
    }

    #[test]
    fn a_sibling_sharing_a_name_prefix_is_not_swept_in() {
        // starts_with on Path compares whole components, which is exactly what is wanted here;
        // a string prefix check would put /home/user/Projects-old inside /home/user/Projects.
        let zones = vec![zone("/home/user/Projects")];
        assert!(zone_for(Path::new("/home/user/Projects-old/file.txt"), &zones).is_none());
    }

    #[test]
    fn the_root_itself_belongs_to_its_zone() {
        let zones = vec![zone("/home/user/Projects")];
        assert!(zone_for(Path::new("/home/user/Projects"), &zones).is_some());
    }

    #[test]
    fn names_come_from_the_final_component() {
        assert_eq!(name_for(Path::new("/home/user/Projects")), "Projects");
        assert_eq!(name_for(Path::new("/home/user/Projects/")), "Projects");
        assert_eq!(name_for(Path::new("/")), "/");
    }

    #[test]
    fn a_derived_zone_is_not_marked_as_overridden() {
        let z = zone("/home/user/Documents");
        assert!(!z.user_override, "only a human choice counts as an override");
        assert_eq!(z.color_hue, hue_for(Path::new("/home/user/Documents")));
    }
}
