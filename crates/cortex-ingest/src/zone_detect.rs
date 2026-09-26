//! Deciding which directories become zones.
//!
//! Zones exist to make the tree navigable at a glance, so the rule for creating one is: it must
//! be somewhere the user thinks of as a *place*. Two sources qualify.
//!
//! **The configured roots and their immediate children.** `~/Documents`, `~/Projects`,
//! `~/Pictures` — the areas a person names when describing where something is.
//!
//! **Project roots.** A directory containing `.git`, `Cargo.toml`, `package.json` and the like
//! is a place in its own right, however deep it sits. This is the case where the deepest-wins
//! rule earns itself: a repository inside `~/Documents` should read as that repository, not
//! dissolve into the surrounding area.
//!
//! What is deliberately *not* a zone: every directory. A hundred colours is the same as none.

use cortex_core::denylist;
use cortex_core::zones::{name_for, Zone};

use std::path::{Path, PathBuf};

/// Files whose presence marks a directory as a project root.
const PROJECT_MARKERS: &[&str] = &[
    ".git",
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "CMakeLists.txt",
    "Makefile",
    "flake.nix",
    "composer.json",
    "Gemfile",
    // A project that is mostly prose, or Python without packaging, or a sketch an agent keeps
    // notes for, has none of the above — and on a workstation where agents start most projects,
    // that is most of them. Their entry files are markers just as much as a manifest is.
    "README.md",
    "CLAUDE.md",
    "AGENTS.md",
    "requirements.txt",
    "setup.py",
    "PROGRESS.md",
    "docs",
];

/// How deep below a configured root to look for project markers.
///
/// Projects live near the top of the areas people keep them in. Scanning the whole tree would
/// find vendored copies inside dependency directories, which are not places anybody navigates to.
const PROJECT_SCAN_DEPTH: usize = 3;

/// Directories that are areas by convention, matched by name directly under a configured root.
const CONVENTIONAL_AREAS: &[&str] = &[
    "Documents",
    "Downloads",
    "Pictures",
    "Music",
    "Videos",
    "Desktop",
    "Projects",
    "Archive",
    "Notes",
    "Templates",
    "Public",
    "src",
];

/// Find the zones for a set of configured roots.
///
/// Returns them shallowest-first, capped at `budget`. Colours are derived from each root's
/// path, so the result is stable across runs and across rebuilds.
///
/// The cap keeps the shallowest candidates. A tree with three hundred repositories in it has
/// three hundred *places*, but not three hundred places a person orients by — and every zone
/// costs a pass over the index when assignments are computed.
pub fn detect(roots: &[PathBuf], budget: usize) -> Vec<Zone> {
    let mut found: Vec<Zone> = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    for root in roots {
        if !root.is_dir() {
            continue;
        }

        // The root itself is always a zone, so that everything indexed belongs somewhere.
        push_unique(&mut found, &mut seen, Zone::derived(root, name_for(root), icon_for(root)));

        for area in immediate_areas(root) {
            push_unique(&mut found, &mut seen, Zone::derived(&area, name_for(&area), icon_for(&area)));
        }

        let projects = project_roots(root, PROJECT_SCAN_DEPTH);
        for project in &projects {
            push_unique(
                &mut found,
                &mut seen,
                Zone::derived(project, name_for(project), Some("project".to_string())),
            );
        }

        // A root whose children are mostly projects is a projects folder, and in a projects
        // folder every child is a project — including the ones that carry no marker yet: a plan
        // and a script, a research dump, a study. Without this they all share the root's zone,
        // and a heatmap by zone paints them as one place.
        let direct = projects.iter().filter(|p| p.parent() == Some(root.as_path())).count();
        if direct >= 3 {
            for child in plain_children(root) {
                push_unique(
                    &mut found,
                    &mut seen,
                    Zone::derived(&child, name_for(&child), Some("project".to_string())),
                );
            }
        }
    }

    // Depth first, then path. The second key is not cosmetic: sorting by depth alone is not a
    // total order, so which candidates survive the budget below would depend on directory
    // iteration order — and the surviving set, hence the colours, could differ between runs on
    // the same tree. Stability is the entire premise of D15.
    found.sort_by(|a, b| {
        a.root_path
            .components()
            .count()
            .cmp(&b.root_path.components().count())
            .then_with(|| a.root_path.cmp(&b.root_path))
    });

    if found.len() > budget {
        tracing::info!(
            found = found.len(),
            budget,
            "more zone candidates than the budget allows; keeping the shallowest"
        );
        found.truncate(budget);
    }
    found
}

fn push_unique(
    found: &mut Vec<Zone>,
    seen: &mut std::collections::HashSet<PathBuf>,
    zone: Zone,
) {
    if seen.insert(zone.root_path.clone()) {
        found.push(zone);
    }
}

/// Conventional areas directly below a root.
fn immediate_areas(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            !denylist::should_skip_subtree(&name) && CONVENTIONAL_AREAS.contains(&name.as_ref())
        })
        .map(|e| e.path())
        .collect()
}

/// The directories directly under `root` that a walk would enter, in name order.
fn plain_children(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            !name.starts_with('.') && !denylist::should_skip_subtree(&name)
        })
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

/// Directories within `depth` of `root` that look like project roots.
fn project_roots(root: &Path, depth: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut queue: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];

    while let Some((dir, level)) = queue.pop() {
        if is_project_root(&dir) && dir != root {
            // A project's subdirectories are part of that project, not places of their own,
            // so the walk does not continue below one.
            found.push(dir);
            continue;
        }
        if level >= depth {
            continue;
        }

        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name();
            if denylist::should_skip_subtree(&name.to_string_lossy()) {
                continue;
            }
            queue.push((entry.path(), level + 1));
        }
    }

    found
}

fn is_project_root(dir: &Path) -> bool {
    PROJECT_MARKERS.iter().any(|marker| dir.join(marker).exists())
}

/// Whether a file name is one of the markers that turns its directory into a project.
///
/// Exposed so the watcher can tell, cheaply, whether a newly created file might have changed
/// the zone layout — without the watcher having to know what a project marker looks like.
pub fn is_project_marker(file_name: &str) -> bool {
    PROJECT_MARKERS.contains(&file_name)
}

/// Whether a directory name is one that would become a zone if it sat directly under a root.
pub fn is_conventional_area(dir_name: &str) -> bool {
    CONVENTIONAL_AREAS.contains(&dir_name)
}

/// A hint for the GUI. Deliberately a name rather than a glyph: the icon set belongs to the
/// front end, and baking Unicode into the daemon would freeze that choice here.
fn icon_for(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    let icon = match name.as_str() {
        "documents" => "document",
        "downloads" => "download",
        "pictures" => "image",
        "music" => "music",
        "videos" => "video",
        "desktop" => "desktop",
        "projects" | "src" => "code",
        "archive" => "archive",
        "notes" => "note",
        _ => "folder",
    };
    Some(icon.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        std::fs::create_dir_all(root.join("Documents/2026")).unwrap();
        std::fs::create_dir_all(root.join("Pictures/holiday")).unwrap();
        std::fs::create_dir_all(root.join("Projects/aurora/src")).unwrap();
        std::fs::create_dir_all(root.join("Projects/basalt")).unwrap();
        std::fs::create_dir_all(root.join("Projects/aurora/node_modules/pkg")).unwrap();
        std::fs::create_dir_all(root.join("random-folder")).unwrap();

        // Project markers.
        std::fs::write(root.join("Projects/aurora/Cargo.toml"), "").unwrap();
        std::fs::create_dir_all(root.join("Projects/basalt/.git")).unwrap();
        // A vendored copy that must not become its own zone.
        std::fs::write(root.join("Projects/aurora/node_modules/pkg/package.json"), "").unwrap();

        tmp
    }

    fn roots_of(zones: &[Zone]) -> Vec<String> {
        zones.iter().map(|z| z.root_path.display().to_string()).collect()
    }

    #[test]
    fn in_a_projects_folder_every_child_is_a_project_even_without_a_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for (dir, marker) in [("a", ".git"), ("b", "Cargo.toml"), ("c", "README.md")] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            std::fs::write(root.join(dir).join(marker), "").unwrap();
        }
        std::fs::create_dir_all(root.join("sketch")).unwrap();
        std::fs::write(root.join("sketch/plan.txt"), "an idea").unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();

        let roots = roots_of(&detect(&[root.to_path_buf()], 1000));
        let has = |p: std::path::PathBuf| roots.contains(&p.display().to_string());
        assert!(has(root.join("sketch")), "{roots:?}");
        assert!(!has(root.join(".hidden")));
    }

    #[test]
    fn two_projects_do_not_make_a_projects_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for dir in ["a", "b"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            std::fs::write(root.join(dir).join("Cargo.toml"), "").unwrap();
        }
        std::fs::create_dir_all(root.join("music")).unwrap();
        let roots = roots_of(&detect(&[root.to_path_buf()], 1000));
        assert!(!roots.contains(&root.join("music").display().to_string()));
    }

    #[test]
    fn the_configured_root_is_always_a_zone() {
        let tmp = corpus();
        let zones = detect(&[tmp.path().to_path_buf()], 1000);
        assert!(
            zones.iter().any(|z| z.root_path == tmp.path()),
            "everything indexed must belong to some zone"
        );
    }

    #[test]
    fn conventional_areas_become_zones() {
        let tmp = corpus();
        let zones = detect(&[tmp.path().to_path_buf()], 1000);
        let roots = roots_of(&zones);

        for area in ["Documents", "Pictures", "Projects"] {
            assert!(
                roots.iter().any(|r| r.ends_with(area)),
                "{area} should be a zone: {roots:?}"
            );
        }
    }

    #[test]
    fn an_unrecognised_directory_does_not_become_a_zone() {
        // A hundred colours is the same as none.
        let tmp = corpus();
        let roots = roots_of(&detect(&[tmp.path().to_path_buf()], 1000));
        assert!(
            !roots.iter().any(|r| r.ends_with("random-folder")),
            "every directory becoming a zone would make the colours meaningless: {roots:?}"
        );
    }

    #[test]
    fn project_roots_become_zones_wherever_they_sit() {
        let tmp = corpus();
        let roots = roots_of(&detect(&[tmp.path().to_path_buf()], 1000));

        assert!(roots.iter().any(|r| r.ends_with("Projects/aurora")), "{roots:?}");
        assert!(roots.iter().any(|r| r.ends_with("Projects/basalt")), "{roots:?}");
    }

    #[test]
    fn a_vendored_project_inside_a_dependency_tree_is_not_a_zone() {
        let tmp = corpus();
        let roots = roots_of(&detect(&[tmp.path().to_path_buf()], 1000));
        assert!(
            !roots.iter().any(|r| r.contains("node_modules")),
            "dependency trees are not places anybody navigates to: {roots:?}"
        );
    }

    #[test]
    fn the_walk_stops_inside_a_project() {
        // src/ is a conventional area name, but inside a project it is part of that project,
        // not an area of its own.
        let tmp = corpus();
        let roots = roots_of(&detect(&[tmp.path().to_path_buf()], 1000));
        assert!(
            !roots.iter().any(|r| r.ends_with("aurora/src")),
            "a project's internals must not fragment into separate zones: {roots:?}"
        );
    }

    #[test]
    fn zones_come_back_shallowest_first() {
        let tmp = corpus();
        let zones = detect(&[tmp.path().to_path_buf()], 1000);
        let depths: Vec<usize> = zones.iter().map(|z| z.root_path.components().count()).collect();
        assert!(
            depths.windows(2).all(|w| w[0] <= w[1]),
            "assignment applies zones in order and relies on this: {depths:?}"
        );
    }

    #[test]
    fn detection_is_stable_across_runs() {
        // Colours that change between sessions are noise rather than a memory aid.
        let tmp = corpus();
        let first = detect(&[tmp.path().to_path_buf()], 1000);
        let second = detect(&[tmp.path().to_path_buf()], 1000);
        assert_eq!(first, second);
    }

    #[test]
    fn the_surviving_set_is_stable_when_the_budget_is_saturated() {
        // With more candidates than slots, which ones survive must be decided by a total
        // order. Sorting on depth alone leaves ties to directory iteration order, so the
        // surviving set — and every colour with it — could change between runs on an
        // unchanged tree.
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..40 {
            let dir = tmp.path().join("Projects").join(format!("p{i:03}"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("Cargo.toml"), "").unwrap();
        }

        let first = roots_of(&detect(&[tmp.path().to_path_buf()], 10));
        for _ in 0..5 {
            assert_eq!(
                roots_of(&detect(&[tmp.path().to_path_buf()], 10)),
                first,
                "the surviving set changed between identical runs"
            );
        }
    }

    #[test]
    fn zones_are_ordered_deterministically_within_a_depth() {
        let tmp = corpus();
        let zones = detect(&[tmp.path().to_path_buf()], 1000);
        let mut by_depth: std::collections::HashMap<usize, Vec<&PathBuf>> = Default::default();
        for z in &zones {
            by_depth
                .entry(z.root_path.components().count())
                .or_default()
                .push(&z.root_path);
        }
        for (depth, paths) in by_depth {
            let mut sorted = paths.clone();
            sorted.sort();
            assert_eq!(paths, sorted, "depth {depth} is not in path order");
        }
    }

    #[test]
    fn a_missing_root_yields_nothing_rather_than_panicking() {
        assert!(detect(&[PathBuf::from("/definitely/not/here")], 1000).is_empty());
    }

    #[test]
    fn overlapping_roots_do_not_produce_duplicate_zones() {
        let tmp = corpus();
        let zones = detect(&[tmp.path().to_path_buf(), tmp.path().join("Projects")], 1000);

        let mut roots = roots_of(&zones);
        let before = roots.len();
        roots.sort();
        roots.dedup();
        assert_eq!(before, roots.len(), "a zone was registered twice");
    }

    #[test]
    fn project_markers_are_recognisable_by_name_alone() {
        // The watcher uses this to decide whether a new file could have changed the zone
        // layout, so it has to agree with what detection actually looks for.
        for marker in PROJECT_MARKERS {
            assert!(is_project_marker(marker), "{marker} should be recognised");
        }
        assert!(!is_project_marker("readme.md"));
        assert!(!is_project_marker("main.rs"));
    }

    #[test]
    fn conventional_areas_are_recognisable_by_name_alone() {
        assert!(is_conventional_area("Documents"));
        assert!(!is_conventional_area("random-folder"));
    }

    #[test]
    fn the_budget_caps_the_count_and_keeps_the_shallowest() {
        let tmp = corpus();
        let zones = detect(&[tmp.path().to_path_buf()], 2);

        assert_eq!(zones.len(), 2);
        let deepest = zones.iter().map(|z| z.root_path.components().count()).max().unwrap();
        let all = detect(&[tmp.path().to_path_buf()], 1000);
        let deepest_overall = all.iter().map(|z| z.root_path.components().count()).max().unwrap();
        assert!(
            deepest < deepest_overall,
            "the cap should drop deep zones, not arbitrary ones"
        );
    }

    #[test]
    fn every_zone_carries_a_name_so_colour_is_never_the_only_signal() {
        let tmp = corpus();
        for zone in detect(&[tmp.path().to_path_buf()], 1000) {
            assert!(!zone.name.is_empty(), "{:?} has no name", zone.root_path);
        }
    }
}
