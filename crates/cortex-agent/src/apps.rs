//! What opens this file — read from the desktop's own databases, not from a list of our own.
//!
//! A file manager that keeps its own idea of which program opens a `.odt` is a file manager that
//! disagrees with the rest of the session. Somebody sets their editor in GNOME's settings and
//! this window still opens the old one; they set it here and nothing else notices. So every
//! answer in this module is read from the files the specifications name, and setting a default
//! writes to the same file `xdg-mime default` writes to.
//!
//! Three separate databases, and conflating them is the usual way this goes wrong:
//!
//! 1. **What the file is** — `mime/globs2` from shared-mime-info, plus the magic numbers the
//!    indexer already knows about ([`cortex_extract::classify`]).
//! 2. **Which applications claim the type** — `mimeapps.list` in the order the Association
//!    specification gives, then `mimeinfo.cache` in each applications directory.
//! 3. **What an application is** — its desktop entry: a name in the session's language, an
//!    `Exec` line, and whether it wants a terminal.
//!
//! ## Why the daemon and not the window
//!
//! The same reason as [`crate::places`] (D4): this is judgement — precedence between eight
//! configuration files, glob weighting, subclass fallback — and judgement implemented twice is
//! judgement that drifts. `cortex apps <file>` prints what the window shows, which is the cheap
//! way to find out which of the two is wrong.
//!
//! **Running the application is not here.** The daemon is headless (D49): it can say that
//! `Exec=viewer %U` is what opens a picture, and it has no session to open a window in. The
//! split is [`cortex_core::tools::Application::command_line`], which both halves share.
//!
//! ## Everything is read from files, again
//!
//! No `gio`, no `libgio-2.0`, no D-Bus. The formats are text, the rules are written down, and
//! the alternative is a GLib main loop inside a daemon whose first-screen latency is a budget.

use cortex_core::tools::Application;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// What an unrecognised file is called. The desktop's name for "some bytes".
pub const UNKNOWN: &str = "application/octet-stream";
/// What a directory is called, in the same vocabulary.
pub const DIRECTORY: &str = "inode/directory";
/// What a file with nothing in it is called. shared-mime-info's own name for it: reporting
/// `text/plain` instead would be a guess that only looks harmless.
pub const EMPTY: &str = "application/x-zerosize";

/// The directories the XDG specifications say to look in, and the session's language.
///
/// Built from the environment in production and by hand in tests. Everything below is a
/// function of this struct and the filesystem under it, so the whole module can be tested
/// against a directory tree that was made for the purpose — which matters more here than
/// usual, because the answers depend on what happens to be installed on the machine.
#[derive(Debug, Clone)]
pub struct Xdg {
    pub config_home: PathBuf,
    pub config_dirs: Vec<PathBuf>,
    pub data_home: PathBuf,
    pub data_dirs: Vec<PathBuf>,
    /// `XDG_CURRENT_DESKTOP`, split and lowercased. Each name gives a `<desktop>-mimeapps.list`
    /// that outranks the plain one in the same directory.
    pub desktops: Vec<String>,
    /// The language of the session as `de_DE`, if it says. Desktop entries carry translated
    /// names and a chooser that ignores them is an English list in a German window.
    pub locale: Option<String>,
}

impl Xdg {
    pub fn from_env() -> Self {
        let home = crate::places::home_dir();
        Self {
            config_home: dir_var("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config")),
            config_dirs: dirs_var("XDG_CONFIG_DIRS")
                .unwrap_or_else(|| vec![PathBuf::from("/etc/xdg")]),
            data_home: dir_var("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share")),
            data_dirs: dirs_var("XDG_DATA_DIRS").unwrap_or_else(|| {
                vec![PathBuf::from("/usr/local/share"), PathBuf::from("/usr/share")]
            }),
            desktops: std::env::var("XDG_CURRENT_DESKTOP")
                .unwrap_or_default()
                .split(':')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_ascii_lowercase())
                .collect(),
            locale: locale_from_env(),
        }
    }

    /// Every `applications` directory, most specific first.
    fn application_dirs(&self) -> Vec<PathBuf> {
        std::iter::once(&self.data_home)
            .chain(&self.data_dirs)
            .map(|d| d.join("applications"))
            .collect()
    }

    /// Every `mime` directory, most specific first.
    fn mime_dirs(&self) -> Vec<PathBuf> {
        std::iter::once(&self.data_home)
            .chain(&self.data_dirs)
            .map(|d| d.join("mime"))
            .collect()
    }

    /// The `mimeapps.list` files, in the precedence the Association specification gives.
    ///
    /// The desktop-prefixed file wins over the plain one *within each directory*, rather than
    /// all prefixed files winning over all plain ones — a detail that decides whether a
    /// system-wide GNOME default can override somebody's personal choice. It must not.
    fn mimeapps_files(&self) -> Vec<PathBuf> {
        let pair = |dir: &Path| -> Vec<PathBuf> {
            let mut both: Vec<PathBuf> = self
                .desktops
                .iter()
                .map(|desktop| dir.join(format!("{desktop}-mimeapps.list")))
                .collect();
            both.push(dir.join("mimeapps.list"));
            both
        };

        let mut files = pair(&self.config_home);
        for dir in &self.config_dirs {
            files.extend(pair(dir));
        }
        // The deprecated location, still written by older tools and still read by everything.
        for dir in self.application_dirs() {
            files.extend(pair(&dir));
            // `defaults.list` is the name this file had before `mimeapps.list`, and packages
            // still ship it. `xdg-mime query default` reads it, so a machine where it is the
            // only thing naming a default must not get a different answer from this window.
            files.push(dir.join("defaults.list"));
        }
        files
    }

    /// Where a default is recorded when this machine sets one: the user's own, desktop-agnostic
    /// file, which is what `xdg-mime default` writes and every other application reads.
    pub fn user_mimeapps(&self) -> PathBuf {
        self.config_home.join("mimeapps.list")
    }
}

fn dir_var(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

fn dirs_var(name: &str) -> Option<Vec<PathBuf>> {
    let raw = std::env::var(name).ok()?;
    let dirs: Vec<PathBuf> = raw
        .split(':')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .collect();
    (!dirs.is_empty()).then_some(dirs)
}

/// `de_DE.UTF-8` and `de_DE@euro` both mean `de_DE`. The encoding and the modifier are not part
/// of the key a desktop entry uses.
fn locale_from_env() -> Option<String> {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(var) {
            let trimmed = value.split(['.', '@']).next().unwrap_or("").to_string();
            if !trimmed.is_empty() && trimmed != "C" && trimmed != "POSIX" {
                return Some(trimmed);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// What the file is
// ---------------------------------------------------------------------------

/// One line of `globs2`: how strongly a pattern claims a type.
#[derive(Debug, Clone)]
struct Glob {
    weight: i32,
    mime: String,
    pattern: String,
    case_sensitive: bool,
}

/// How specific a pattern is, before weight is considered.
///
/// The order is the shared-mime-info specification's: a literal file name beats `*.ext`, which
/// beats anything else. Without it, `Makefile` would be typed by whichever `*` pattern happened
/// to sort first.
#[derive(PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Specificity {
    Other,
    Suffix,
    Literal,
}

impl Glob {
    fn specificity(&self) -> Specificity {
        if !self.pattern.contains(['*', '?', '[']) {
            Specificity::Literal
        } else if self.pattern.starts_with("*.") && !self.pattern[2..].contains(['*', '?', '[']) {
            Specificity::Suffix
        } else {
            Specificity::Other
        }
    }

    fn matches(&self, name: &str) -> bool {
        if self.case_sensitive {
            wildcard(&self.pattern, name)
        } else {
            wildcard(&self.pattern.to_lowercase(), &name.to_lowercase())
        }
    }
}

/// `*` and `?` only, and deliberately no character classes.
///
/// The database does contain a handful of `[0-9]` patterns; they are for types nobody opens from
/// a file manager, and a matcher that half-implements a dialect is worse than one that says
/// which dialect it implements. A pattern this cannot represent simply never matches.
fn wildcard(pattern: &str, name: &str) -> bool {
    if pattern.contains('[') {
        return false;
    }
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    // The usual two-pointer walk with one remembered star, which is linear and needs no
    // allocation — this runs once per candidate pattern and the database has thousands.
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut resume) = (None, 0usize);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            resume = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            resume += 1;
            ni = resume;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Read every `globs2` there is, most specific directory first.
fn load_globs(xdg: &Xdg) -> Vec<Glob> {
    let mut globs = Vec::new();
    for dir in xdg.mime_dirs() {
        let Ok(text) = std::fs::read_to_string(dir.join("globs2")) else {
            continue;
        };
        for line in text.lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            // `weight:mime:glob` with optional flags after a third colon. The glob itself can
            // contain a colon, so the split is bounded rather than greedy.
            let mut parts = line.splitn(3, ':');
            let (Some(weight), Some(mime), Some(rest)) =
                (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let Ok(weight) = weight.parse::<i32>() else {
                continue;
            };
            let (pattern, flags) = match rest.rsplit_once(':') {
                Some((p, f)) if f == "cs" => (p, f),
                _ => (rest, ""),
            };
            globs.push(Glob {
                weight,
                mime: mime.to_string(),
                pattern: pattern.to_string(),
                case_sensitive: flags == "cs",
            });
        }
    }
    globs
}

/// What this file is, in the vocabulary the desktop's association files are keyed on.
///
/// **The name is consulted before the content here, and that is the opposite of what the indexer
/// does** ([`cortex_extract::classify`], whose first rule is that a magic number outranks the
/// extension). The two are answering different questions. The indexer asks "may I extract text
/// from this", where a `.txt` full of JPEG bytes must not be read as text. This asks "what would
/// the rest of the desktop open it with", and the rest of the desktop — `xdg-mime`, `gio`,
/// every other file manager — matches the name first. Disagreeing would mean this window opens
/// a file with a different application than a double click in Nautilus does, which is the one
/// thing this module exists to prevent.
pub fn mime_for(xdg: &Xdg, path: &Path) -> String {
    let metadata = std::fs::metadata(path);
    if metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false) {
        return DIRECTORY.to_string();
    }

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let globs = load_globs(xdg);
    let best = globs
        .iter()
        .filter(|g| g.matches(&name))
        .max_by(|a, b| {
            a.specificity()
                .cmp(&b.specificity())
                .then(a.pattern.chars().count().cmp(&b.pattern.chars().count()))
                .then(a.weight.cmp(&b.weight))
                // A genuine tie, and the database has them: `*.json` is claimed by
                // `application/json` and `application/schema+json` at the same weight and the
                // same length. Something has to decide, and the order the files happened to be
                // read in would make the answer depend on the machine. The type name does not,
                // and it agrees with `xdg-mime` on the one case that occurs in practice.
                .then_with(|| b.mime.cmp(&a.mime))
        });
    if let Some(glob) = best {
        return glob.mime.clone();
    }

    // Nothing in the name said anything. Now the content may.
    if metadata.map(|m| m.len() == 0).unwrap_or(false) {
        return EMPTY.to_string();
    }
    match read_head(path) {
        Some(head) => cortex_extract::classify::classify_bytes(path, &head)
            .mime
            .unwrap_or_else(|| UNKNOWN.to_string()),
        None => UNKNOWN.to_string(),
    }
}

fn read_head(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut buffer = vec![0u8; cortex_extract::classify::SNIFF_BYTES];
    let read = file.read(&mut buffer).ok()?;
    buffer.truncate(read);
    Some(buffer)
}

/// The types this one inherits from, nearest first.
///
/// A `.py` file is `text/x-python`, and the editor somebody uses for it is very often registered
/// for `text/plain` and nothing else. Without the subclass chain the chooser would be empty for
/// exactly the files a person edits most.
fn ancestors(xdg: &Xdg, mime: &str) -> Vec<String> {
    let mut parents: HashMap<String, Vec<String>> = HashMap::new();
    for dir in xdg.mime_dirs() {
        let Ok(text) = std::fs::read_to_string(dir.join("subclasses")) else {
            continue;
        };
        for line in text.lines() {
            if let Some((child, parent)) = line.split_once(' ') {
                parents
                    .entry(child.trim().to_string())
                    .or_default()
                    .push(parent.trim().to_string());
            }
        }
    }

    // Breadth-first, so `text/plain` comes before whatever `text/plain` inherits from, and a
    // cycle in a badly generated database cannot loop for ever.
    let mut seen: HashSet<String> = HashSet::from([mime.to_string()]);
    let mut queue: Vec<String> = parents.get(mime).cloned().unwrap_or_default();
    let mut out = Vec::new();
    while let Some(next) = queue.first().cloned() {
        queue.remove(0);
        if !seen.insert(next.clone()) {
            continue;
        }
        out.push(next.clone());
        queue.extend(parents.get(&next).cloned().unwrap_or_default());
    }

    // Every text type is a `text/plain` whether the database says so or not, and every file is
    // a stream of bytes. Both are in the standard database; adding them when it is absent keeps
    // a machine with no shared-mime-info from having an empty chooser.
    if mime.starts_with("text/") && mime != "text/plain" && !out.iter().any(|m| m == "text/plain") {
        out.push("text/plain".to_string());
    }
    out
}

// ---------------------------------------------------------------------------
// Which applications claim the type
// ---------------------------------------------------------------------------

/// One `.desktop`- or `.list`-style file, as groups of key/value pairs.
///
/// Written here rather than taken from a crate because the format is twelve lines of parsing and
/// the two crates that do it disagree about locale keys, which is the only part that is subtle.
#[derive(Debug, Default)]
struct Ini {
    groups: HashMap<String, Vec<(String, String)>>,
}

impl Ini {
    fn parse(text: &str) -> Self {
        let mut groups: HashMap<String, Vec<(String, String)>> = HashMap::new();
        let mut current = String::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                current = name.to_string();
                groups.entry(current.clone()).or_default();
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                groups
                    .entry(current.clone())
                    .or_default()
                    .push((key.trim().to_string(), value.trim().to_string()));
            }
        }
        Self { groups }
    }

    fn read(path: &Path) -> Option<Self> {
        std::fs::read_to_string(path).ok().map(|t| Self::parse(&t))
    }

    fn get(&self, group: &str, key: &str) -> Option<&str> {
        self.groups
            .get(group)?
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// A value in the session's language, falling back the way the specification says: the full
    /// `de_DE`, then the bare `de`, then the untranslated key.
    fn localised(&self, group: &str, key: &str, locale: Option<&str>) -> Option<&str> {
        if let Some(locale) = locale {
            if let Some(value) = self.get(group, &format!("{key}[{locale}]")) {
                return Some(value);
            }
            if let Some(short) = locale.split('_').next() {
                if let Some(value) = self.get(group, &format!("{key}[{short}]")) {
                    return Some(value);
                }
            }
        }
        self.get(group, key)
    }

    /// The semicolon-separated list form, empty entries dropped.
    fn list(&self, group: &str, key: &str) -> Vec<String> {
        self.get(group, key)
            .map(|v| {
                v.split(';')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }
}

const DEFAULTS: &str = "Default Applications";
const ADDED: &str = "Added Associations";
const REMOVED: &str = "Removed Associations";
const ENTRY: &str = "Desktop Entry";

/// Find the file behind a desktop id.
///
/// The id is the path under `applications` with the separators turned into hyphens, so
/// `kde4-konsole.desktop` may be `kde4/konsole.desktop`. Tried in order, most specific data
/// directory first, because an entry in the user's own directory is meant to shadow the system's.
fn entry_path(xdg: &Xdg, id: &str) -> Option<PathBuf> {
    for dir in xdg.application_dirs() {
        let direct = dir.join(id);
        if direct.is_file() {
            return Some(direct);
        }
        // Each hyphen, left to right, may be a directory separator instead.
        let mut candidate = id.to_string();
        while let Some(position) = candidate.find('-') {
            candidate.replace_range(position..position + 1, "/");
            let nested = dir.join(&candidate);
            if nested.is_file() {
                return Some(nested);
            }
        }
    }
    None
}

/// Read one desktop entry, or decline to offer it.
///
/// Declines, and why each one is a decline rather than a row somebody can press:
///
/// - Not `Type=Application` — a link or a directory entry has nothing to run.
/// - `Hidden=true` — the specification's word for "deleted"; the file exists only to mask one
///   in a lower-precedence directory.
/// - `TryExec` naming a program that is not installed — the usual way a package leaves an entry
///   behind for a binary that is gone.
/// - No `Exec` — nothing to run.
///
/// **`NoDisplay=true` is not on that list, and the first version of this had it there.** It
/// reads like "do not show this to anybody", and it means "do not show this in the applications
/// menu" — an entry that exists to handle a type rather than to be launched from a menu. On the
/// machine this was written on, the desktop's own default handler for `text/plain` is exactly
/// such an entry, so filtering it hid the application that a double click actually opens. A
/// chooser that cannot offer what the desktop is already using is worse than no chooser.
fn read_entry(xdg: &Xdg, id: &str, path: &Path) -> Option<Application> {
    let ini = Ini::read(path)?;
    if ini.get(ENTRY, "Type") != Some("Application") {
        return None;
    }
    if ini.get(ENTRY, "Hidden") == Some("true") {
        return None;
    }
    if let Some(try_exec) = ini.get(ENTRY, "TryExec") {
        if !is_executable(try_exec) {
            return None;
        }
    }
    let exec = ini.get(ENTRY, "Exec")?.to_string();
    let locale = xdg.locale.as_deref();
    Some(Application {
        id: id.to_string(),
        name: ini
            .localised(ENTRY, "Name", locale)
            .unwrap_or(id)
            .to_string(),
        comment: ini.localised(ENTRY, "Comment", locale).map(str::to_string),
        exec,
        icon: ini.get(ENTRY, "Icon").map(str::to_string),
        terminal: ini.get(ENTRY, "Terminal") == Some("true"),
        is_default: false,
        path: path.to_path_buf(),
    })
}

/// Whether `TryExec` names something that exists. An absolute path is checked directly;
/// a bare name is looked for on `PATH`, as the specification says.
fn is_executable(name: &str) -> bool {
    let candidate = Path::new(name);
    if candidate.is_absolute() {
        return candidate.is_file();
    }
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| dir.join(candidate).is_file())
        })
        .unwrap_or(false)
}

/// The applications that claim a type, the desktop's default first.
///
/// The order of the files is the Association specification's, and the loop below is its
/// algorithm rather than an approximation of it. Two parts of it are easy to get wrong and are
/// the reason somebody's choice stops being honoured:
///
/// - A **removal** in one file hides an association added by any *lower*-precedence file, and
///   not by a higher one. So removals are collected as the walk descends.
/// - The **default** is the first entry, in the first file that names one, that still resolves
///   to an installed application. A default naming a package that has been removed must fall
///   through to the next candidate rather than leaving the type with no default at all.
pub fn applications_for(xdg: &Xdg, mime: &str) -> Vec<Application> {
    let mut chain = vec![mime.to_string()];
    chain.extend(ancestors(xdg, mime));

    let mut removed: HashSet<(String, String)> = HashSet::new();
    let mut default: Option<String> = None;
    let mut ordered: Vec<String> = Vec::new();

    for file in xdg.mimeapps_files() {
        let Some(ini) = Ini::read(&file) else { continue };
        for type_name in &chain {
            if default.is_none() {
                for id in ini.list(DEFAULTS, type_name) {
                    if !removed.contains(&(type_name.clone(), id.clone()))
                        && entry_path(xdg, &id).is_some()
                    {
                        default = Some(id);
                        break;
                    }
                }
            }
            for id in ini.list(ADDED, type_name) {
                if !removed.contains(&(type_name.clone(), id.clone())) && !ordered.contains(&id) {
                    ordered.push(id);
                }
            }
            for id in ini.list(REMOVED, type_name) {
                removed.insert((type_name.clone(), id));
            }
        }
    }

    // Then what the applications themselves registered, which is the base every mimeapps.list
    // is an amendment to.
    for dir in xdg.application_dirs() {
        let Some(cache) = Ini::read(&dir.join("mimeinfo.cache")) else {
            continue;
        };
        for type_name in &chain {
            for id in cache.list("MIME Cache", type_name) {
                if !removed.contains(&(type_name.clone(), id.clone())) && !ordered.contains(&id) {
                    ordered.push(id);
                }
            }
        }
    }

    // **Nobody chose, so the desktop chooses: the first association wins.** Not a guess — it
    // is what `xdg-mime query default` answers and therefore what `xdg-open`, and every Enter
    // in this window, actually does. Leaving the list unmarked instead would be truthful about
    // the configuration files and wrong about the machine: the chooser would show no default
    // for `text/plain` on a system where opening a `.txt` plainly opens something.
    if default.is_none() {
        default = ordered.first().cloned();
    }

    // A default that nothing else mentions is still an entry in the list, and it is first.
    if let Some(id) = &default {
        ordered.retain(|other| other != id);
        ordered.insert(0, id.clone());
    }

    ordered
        .iter()
        .filter_map(|id| {
            let path = entry_path(xdg, id)?;
            let mut app = read_entry(xdg, id, &path)?;
            app.is_default = default.as_deref() == Some(id.as_str());
            Some(app)
        })
        .collect()
}

/// Every application installed on the machine, by name.
///
/// For "open with something else": the type's own list is the right answer almost always, and
/// the times it is not — a file with no extension, a type nothing claims — are exactly the times
/// somebody needs to reach past it. Sorted by name because there is no better order; the
/// association files say nothing about applications that never claimed the type.
pub fn all_applications(xdg: &Xdg) -> Vec<Application> {
    let mut found: Vec<Application> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for dir in xdg.application_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let Some(id) = path.file_name().map(|n| n.to_string_lossy().to_string()) else {
                continue;
            };
            // The first directory that has an id owns it: a user's own entry shadows the
            // system's, rather than appearing twice.
            if !seen.insert(id.clone()) {
                continue;
            }
            if let Some(app) = read_entry(xdg, &id, &path) {
                found.push(app);
            }
        }
    }
    found.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    found
}

// ---------------------------------------------------------------------------
// Changing what the default is
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum SetDefaultError {
    #[error("no application called {0} is installed")]
    NoSuchApplication(String),
    #[error("{path}: {message}")]
    Io { path: PathBuf, message: String },
}

/// What the default for this type is now, without building the whole list.
///
/// Used to record what a change replaced: an audit line saying only what a default *became* does
/// not let anybody put it back.
pub fn current_default(xdg: &Xdg, mime: &str) -> Option<String> {
    applications_for(xdg, mime)
        .into_iter()
        .find(|a| a.is_default)
        .map(|a| a.id)
}

/// Record `id` as what opens `mime`, in the file the rest of the desktop reads.
///
/// Three things this does not do, each of them deliberate:
///
/// - **It does not rewrite the file.** The user's `mimeapps.list` is hand-edited by people and
///   written by other tools; every line that is not the one key being changed is passed through
///   verbatim, comments and all. A file manager that reformats a configuration file it was
///   asked to change one line of has exceeded its instructions.
/// - **It does not silently drop a contradicting removal.** An id listed under `Removed
///   Associations` for this type would be filtered straight back out of the list by our own
///   resolver, so setting it as the default has to lift that removal or do nothing at all.
/// - **It does not write in place.** A crash halfway through leaves the previous file, not half
///   of one — the same rule as everywhere else here.
pub fn set_default(xdg: &Xdg, mime: &str, id: &str) -> Result<PathBuf, SetDefaultError> {
    if entry_path(xdg, id).is_none() {
        return Err(SetDefaultError::NoSuchApplication(id.to_string()));
    }

    let file = xdg.user_mimeapps();
    let existing = std::fs::read_to_string(&file).unwrap_or_default();
    let updated = with_default(&existing, mime, id);

    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| SetDefaultError::Io {
            path: parent.to_path_buf(),
            message: e.to_string(),
        })?;
    }
    let temporary = file.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&temporary, updated).map_err(|e| SetDefaultError::Io {
        path: temporary.clone(),
        message: e.to_string(),
    })?;
    std::fs::rename(&temporary, &file).map_err(|e| SetDefaultError::Io {
        path: file.clone(),
        message: e.to_string(),
    })?;
    Ok(file)
}

/// The edit itself, as text in and text out, so that every case below is a test and not a
/// temporary directory.
fn with_default(existing: &str, mime: &str, id: &str) -> String {
    let mut lines: Vec<String> = existing.lines().map(str::to_string).collect();
    let mut group: Option<String> = None;
    let mut defaults_end: Option<usize> = None;
    let mut replaced = false;
    let mut remove: Vec<usize> = Vec::new();

    for index in 0..lines.len() {
        let line = lines[index].trim().to_string();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            if group.as_deref() == Some(DEFAULTS) {
                defaults_end = Some(index);
            }
            group = Some(name.to_string());
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != mime {
            continue;
        }
        match group.as_deref() {
            Some(DEFAULTS) => {
                lines[index] = format!("{mime}={id}");
                replaced = true;
            }
            Some(REMOVED) => {
                // The removal that would otherwise cancel the line just written.
                let kept: Vec<&str> = value
                    .split(';')
                    .map(str::trim)
                    .filter(|entry| !entry.is_empty() && *entry != id)
                    .collect();
                if kept.is_empty() {
                    remove.push(index);
                } else {
                    lines[index] = format!("{mime}={};", kept.join(";"));
                }
            }
            _ => {}
        }
    }

    if !replaced {
        let entry = format!("{mime}={id}");
        match defaults_end.or_else(|| {
            (group.as_deref() == Some(DEFAULTS)).then_some(lines.len())
        }) {
            // Inside the group that is already there, at its end.
            Some(at) => lines.insert(at, entry),
            None => {
                if !lines.is_empty() {
                    lines.push(String::new());
                }
                lines.push(format!("[{DEFAULTS}]"));
                lines.push(entry);
            }
        }
    }

    for index in remove.into_iter().rev() {
        lines.remove(index);
    }

    let mut out = lines.join("\n");
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A machine with nothing on it but what a test puts there.
    ///
    /// Every test below builds its own desktop. The alternative — reading the real one — would
    /// give a suite that passes here and fails on a machine with a different editor installed,
    /// which is the same as no suite at all.
    struct Desktop {
        root: tempfile::TempDir,
        xdg: Xdg,
    }

    impl Desktop {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let base = root.path();
            let xdg = Xdg {
                config_home: base.join("config"),
                config_dirs: vec![base.join("etc/xdg")],
                data_home: base.join("data-home"),
                data_dirs: vec![base.join("data-system")],
                desktops: vec![],
                locale: None,
            };
            for dir in [&xdg.config_home]
                .into_iter()
                .chain(&xdg.config_dirs)
                .chain(xdg.application_dirs().iter())
                .chain(xdg.mime_dirs().iter())
            {
                std::fs::create_dir_all(dir).unwrap();
            }
            Self { root, xdg }
        }

        fn write(&self, relative: &str, contents: &str) -> PathBuf {
            let path = self.root.path().join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, contents).unwrap();
            path
        }

        /// An installed application, in the system's applications directory.
        fn app(&self, id: &str, extra: &str) -> &Self {
            self.named(id, id, extra)
        }

        /// The same, when the test cares what it is called.
        fn named(&self, id: &str, name: &str, extra: &str) -> &Self {
            self.write(
                &format!("data-system/applications/{id}"),
                &format!("[Desktop Entry]\nType=Application\nName={name}\nExec=run-{id} %f\n{extra}\n"),
            );
            self
        }

        fn globs(&self, lines: &str) -> &Self {
            self.write("data-system/mime/globs2", lines);
            self
        }

        fn file(&self, name: &str, contents: &[u8]) -> PathBuf {
            let path = self.root.path().join("files").join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, contents).unwrap();
            path
        }

        fn ids_for(&self, mime: &str) -> Vec<String> {
            applications_for(&self.xdg, mime)
                .into_iter()
                .map(|a| a.id)
                .collect()
        }
    }

    // -----------------------------------------------------------------------
    // What the file is
    // -----------------------------------------------------------------------

    #[test]
    fn the_wildcard_dialect_is_star_and_question_mark() {
        assert!(wildcard("*.txt", "notes.txt"));
        assert!(!wildcard("*.txt", "notes.txt.bak"));
        assert!(wildcard("Makefile*", "Makefile.am"));
        assert!(wildcard("?.c", "a.c"));
        assert!(!wildcard("?.c", "ab.c"));
        assert!(wildcard("*", "anything"));
        assert!(wildcard("*.tar.*", "archive.tar.gz"));
        // A pattern this cannot represent never matches, rather than half-matching.
        assert!(!wildcard("[0-9].txt", "1.txt"));
    }

    #[test]
    fn a_name_that_matches_a_glob_is_that_type() {
        let d = Desktop::new();
        d.globs("50:text/plain:*.txt\n50:image/png:*.png\n");
        assert_eq!(mime_for(&d.xdg, &d.file("notes.txt", b"hello")), "text/plain");
        assert_eq!(mime_for(&d.xdg, &d.file("a.png", b"whatever")), "image/png");
    }

    #[test]
    fn a_literal_name_beats_a_pattern_and_a_longer_suffix_beats_a_shorter_one() {
        let d = Desktop::new();
        d.globs(
            "50:text/x-makefile:Makefile\n\
             50:text/plain:*\n\
             50:application/gzip:*.gz\n\
             50:application/x-compressed-tar:*.tar.gz\n",
        );
        assert_eq!(mime_for(&d.xdg, &d.file("Makefile", b"all:")), "text/x-makefile");
        assert_eq!(
            mime_for(&d.xdg, &d.file("archive.tar.gz", b"x")),
            "application/x-compressed-tar",
            "the more specific suffix is the one that says what the file actually is"
        );
    }

    #[test]
    fn weight_settles_a_tie_and_case_is_ignored_unless_the_database_says_otherwise() {
        let d = Desktop::new();
        d.globs("50:text/plain:*.txt\n60:text/x-special:*.txt\n50:image/x-shouted:*.PNG:cs\n");
        assert_eq!(mime_for(&d.xdg, &d.file("a.txt", b"x")), "text/x-special");
        assert_eq!(
            mime_for(&d.xdg, &d.file("SHOUTING.TXT", b"x")),
            "text/x-special",
            "an ordinary pattern ignores case, which is why .TXT is not a separate database entry"
        );

        // The `cs` flag is the database saying the case is part of the name. Binary content, so
        // that the fallback below cannot answer instead and hide the difference.
        let bytes = &[0xffu8, 0x00, 0xfe];
        assert_eq!(mime_for(&d.xdg, &d.file("a.PNG", bytes)), "image/x-shouted");
        assert_eq!(mime_for(&d.xdg, &d.file("a.png", bytes)), UNKNOWN);
    }

    #[test]
    fn two_types_claiming_the_same_pattern_are_settled_the_same_way_everywhere() {
        let d = Desktop::new();
        d.globs("50:application/schema+json:*.json\n50:application/json:*.json\n");
        assert_eq!(mime_for(&d.xdg, &d.file("a.json", b"{}")), "application/json");
        // And the other way round in the file, to prove the file's order is not the decider.
        d.globs("50:application/json:*.json\n50:application/schema+json:*.json\n");
        assert_eq!(mime_for(&d.xdg, &d.file("b.json", b"{}")), "application/json");
    }

    #[test]
    fn with_no_database_at_all_the_indexers_own_classifier_still_answers() {
        // Not a fallback anybody should rely on, but the difference between a chooser that says
        // "Textdatei" and one that says "some bytes" on a machine where shared-mime-info is not
        // installed. It is the content path from `mime_for`, and that path ends in the
        // classifier the indexer uses — which consults the extension last, exactly as it does
        // when deciding whether a file has text in it.
        let d = Desktop::new();
        assert_eq!(mime_for(&d.xdg, &d.file("notes.md", b"# hello")), "text/markdown");
        assert_eq!(mime_for(&d.xdg, &d.file("main.rs", b"fn main() {}")), "text/rust");
    }

    #[test]
    fn a_directory_and_an_empty_file_have_names_of_their_own() {
        let d = Desktop::new();
        d.globs("50:text/plain:*.txt\n");
        let dir = d.root.path().join("files/somewhere");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(mime_for(&d.xdg, &dir), DIRECTORY);
        assert_eq!(mime_for(&d.xdg, &d.file("nothing", b"")), EMPTY);
    }

    #[test]
    fn a_name_that_says_nothing_falls_through_to_the_content() {
        let d = Desktop::new();
        d.globs("50:text/plain:*.txt\n");
        // A PNG with no extension: the name is silent and the magic number is not.
        let png = d.file("photo", b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR");
        assert_eq!(mime_for(&d.xdg, &png), "image/png");
        // And bytes that are neither named nor recognisable are not guessed at.
        assert_eq!(mime_for(&d.xdg, &d.file("blob", &[0xff, 0x00, 0xfe, 0x01])), UNKNOWN);
    }

    // -----------------------------------------------------------------------
    // Which applications claim it
    // -----------------------------------------------------------------------

    #[test]
    fn the_applications_registered_for_a_type_are_offered() {
        let d = Desktop::new();
        d.app("viewer.desktop", "").app("editor.desktop", "");
        d.write(
            "data-system/applications/mimeinfo.cache",
            "[MIME Cache]\nimage/png=viewer.desktop;editor.desktop;\n",
        );
        assert_eq!(d.ids_for("image/png"), ["viewer.desktop", "editor.desktop"]);
    }

    #[test]
    fn the_users_default_wins_over_the_systems() {
        let d = Desktop::new();
        d.app("viewer.desktop", "").app("editor.desktop", "");
        d.write(
            "etc/xdg/mimeapps.list",
            "[Default Applications]\nimage/png=viewer.desktop\n",
        );
        d.write(
            "config/mimeapps.list",
            "[Default Applications]\nimage/png=editor.desktop\n",
        );

        let apps = applications_for(&d.xdg, "image/png");
        assert_eq!(apps[0].id, "editor.desktop");
        assert!(apps[0].is_default);
        assert!(apps[1..].iter().all(|a| !a.is_default), "there is exactly one default");
    }

    #[test]
    fn a_desktop_specific_file_outranks_the_plain_one_beside_it_and_nothing_below_it() {
        let mut d = Desktop::new();
        d.xdg.desktops = vec!["niri".into()];
        d.app("a.desktop", "").app("b.desktop", "").app("c.desktop", "");

        // The system says a, its niri-flavoured file says b, and the user says c. The user's
        // plain file is in a *higher* directory than either system file and must win.
        d.write("etc/xdg/mimeapps.list", "[Default Applications]\ntext/plain=a.desktop\n");
        d.write("etc/xdg/niri-mimeapps.list", "[Default Applications]\ntext/plain=b.desktop\n");
        d.write("config/mimeapps.list", "[Default Applications]\ntext/plain=c.desktop\n");
        assert_eq!(applications_for(&d.xdg, "text/plain")[0].id, "c.desktop");

        // With the user's choice gone, the desktop-flavoured system file beats the plain one
        // in the same directory.
        std::fs::remove_file(d.root.path().join("config/mimeapps.list")).unwrap();
        assert_eq!(applications_for(&d.xdg, "text/plain")[0].id, "b.desktop");
    }

    #[test]
    fn a_default_naming_an_application_that_is_gone_falls_through() {
        let d = Desktop::new();
        d.app("installed.desktop", "");
        d.write(
            "config/mimeapps.list",
            "[Default Applications]\ntext/plain=uninstalled.desktop;installed.desktop\n",
        );
        let apps = applications_for(&d.xdg, "text/plain");
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].id, "installed.desktop");
        assert!(apps[0].is_default, "a type does not lose its default to a removed package");
    }

    #[test]
    fn a_removal_hides_what_a_lower_precedence_file_added() {
        let d = Desktop::new();
        d.app("unwanted.desktop", "").app("wanted.desktop", "");
        d.write(
            "etc/xdg/mimeapps.list",
            "[Added Associations]\ntext/plain=unwanted.desktop;wanted.desktop;\n",
        );
        d.write(
            "config/mimeapps.list",
            "[Removed Associations]\ntext/plain=unwanted.desktop;\n",
        );
        assert_eq!(d.ids_for("text/plain"), ["wanted.desktop"]);
    }

    #[test]
    fn a_removal_does_not_reach_upwards() {
        let d = Desktop::new();
        d.app("chosen.desktop", "");
        // The user added it; the system, which is read afterwards, says it is removed. The
        // user's file is the one that wins, and a removal that reached backwards would be a
        // system package silently overriding a personal choice.
        d.write("config/mimeapps.list", "[Added Associations]\ntext/plain=chosen.desktop;\n");
        d.write("etc/xdg/mimeapps.list", "[Removed Associations]\ntext/plain=chosen.desktop;\n");
        assert_eq!(d.ids_for("text/plain"), ["chosen.desktop"]);
    }

    #[test]
    fn a_type_inherits_the_applications_of_the_type_it_is_a_kind_of() {
        let d = Desktop::new();
        d.app("editor.desktop", "");
        d.write("data-system/mime/subclasses", "text/x-python text/plain\n");
        d.write(
            "data-system/applications/mimeinfo.cache",
            "[MIME Cache]\ntext/plain=editor.desktop;\n",
        );
        assert_eq!(
            d.ids_for("text/x-python"),
            ["editor.desktop"],
            "the editor somebody uses for Python is registered for text/plain"
        );
    }

    #[test]
    fn the_exact_type_comes_before_what_it_inherits() {
        let d = Desktop::new();
        d.app("python-ide.desktop", "").app("editor.desktop", "");
        d.write("data-system/mime/subclasses", "text/x-python text/plain\n");
        d.write(
            "data-system/applications/mimeinfo.cache",
            "[MIME Cache]\ntext/plain=editor.desktop;\ntext/x-python=python-ide.desktop;\n",
        );
        assert_eq!(d.ids_for("text/x-python"), ["python-ide.desktop", "editor.desktop"]);
    }

    #[test]
    fn entries_that_cannot_be_offered_are_not_offered() {
        let d = Desktop::new();
        d.app("shown.desktop", "")
            .app("hidden.desktop", "Hidden=true")
            .app("missing-binary.desktop", "TryExec=/nonexistent/program");
        d.write(
            "data-system/applications/link.desktop",
            "[Desktop Entry]\nType=Link\nName=A link\nURL=https://example.invalid\n",
        );
        d.write(
            "data-system/applications/no-exec.desktop",
            "[Desktop Entry]\nType=Application\nName=Nothing to run\n",
        );
        d.write(
            "data-system/applications/mimeinfo.cache",
            "[MIME Cache]\ntext/plain=shown.desktop;hidden.desktop;\
             missing-binary.desktop;link.desktop;no-exec.desktop;\n",
        );
        assert_eq!(d.ids_for("text/plain"), ["shown.desktop"]);
    }

    #[test]
    fn an_entry_that_stays_out_of_the_menu_is_still_offered_here() {
        // `NoDisplay=true` means "not in the applications menu", not "not to be used". This is
        // not a hypothetical: on the machine this was written on, the desktop's own default
        // handler for text/plain is a NoDisplay entry, and filtering it left the chooser
        // offering everything *except* the application a double click opens. Checked against
        // `xdg-mime query default`, which lists it too.
        let d = Desktop::new();
        d.app("menu.desktop", "").app("handler.desktop", "NoDisplay=true");
        d.write(
            "data-system/applications/mimeinfo.cache",
            "[MIME Cache]\ntext/plain=handler.desktop;menu.desktop;\n",
        );
        assert_eq!(d.ids_for("text/plain"), ["handler.desktop", "menu.desktop"]);
    }

    #[test]
    fn with_nothing_naming_a_default_the_first_association_is_the_default() {
        // What `xdg-open` does, and therefore what pressing Enter in the window does. A chooser
        // that showed no default here would be describing the configuration files instead of
        // the machine.
        let d = Desktop::new();
        d.app("first.desktop", "").app("second.desktop", "");
        d.write(
            "data-system/applications/mimeinfo.cache",
            "[MIME Cache]\ntext/plain=first.desktop;second.desktop;\n",
        );
        let apps = applications_for(&d.xdg, "text/plain");
        assert!(apps[0].is_default && apps[0].id == "first.desktop");
        assert!(!apps[1].is_default);
    }

    #[test]
    fn the_older_defaults_list_is_read_where_it_is_the_only_thing_naming_one() {
        let d = Desktop::new();
        d.app("first.desktop", "").app("chosen.desktop", "");
        d.write(
            "data-system/applications/mimeinfo.cache",
            "[MIME Cache]\ntext/plain=first.desktop;chosen.desktop;\n",
        );
        d.write(
            "data-system/applications/defaults.list",
            "[Default Applications]\ntext/plain=chosen.desktop\n",
        );
        assert_eq!(current_default(&d.xdg, "text/plain").as_deref(), Some("chosen.desktop"));
    }

    #[test]
    fn a_name_is_read_in_the_language_of_the_session() {
        let mut d = Desktop::new();
        d.xdg.locale = Some("de_DE".into());
        d.write(
            "data-system/applications/viewer.desktop",
            "[Desktop Entry]\nType=Application\nExec=view %f\n\
             Name=Image Viewer\nName[de]=Bildbetrachter\n\
             Comment=Look at pictures\nComment[de_DE]=Bilder ansehen\n",
        );
        d.write(
            "data-system/applications/mimeinfo.cache",
            "[MIME Cache]\nimage/png=viewer.desktop;\n",
        );

        let app = &applications_for(&d.xdg, "image/png")[0];
        assert_eq!(app.name, "Bildbetrachter", "the bare language tag is enough");
        assert_eq!(app.comment.as_deref(), Some("Bilder ansehen"));

        d.xdg.locale = Some("fr_FR".into());
        assert_eq!(
            applications_for(&d.xdg, "image/png")[0].name,
            "Image Viewer",
            "a language the entry does not have falls back to the untranslated name"
        );
    }

    #[test]
    fn a_hyphen_in_an_id_may_be_a_directory() {
        let d = Desktop::new();
        d.write(
            "data-system/applications/kde4/konsole.desktop",
            "[Desktop Entry]\nType=Application\nName=Konsole\nExec=konsole\n",
        );
        d.write(
            "config/mimeapps.list",
            "[Added Associations]\ntext/plain=kde4-konsole.desktop;\n",
        );
        assert_eq!(d.ids_for("text/plain"), ["kde4-konsole.desktop"]);
    }

    #[test]
    fn a_users_own_entry_shadows_the_systems_rather_than_appearing_twice() {
        let d = Desktop::new();
        d.app("editor.desktop", "");
        d.write(
            "data-home/applications/editor.desktop",
            "[Desktop Entry]\nType=Application\nName=Mein Editor\nExec=mine %f\n",
        );
        let all = all_applications(&d.xdg);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "Mein Editor");
    }

    #[test]
    fn everything_installed_can_be_listed_for_the_times_the_association_is_empty() {
        let d = Desktop::new();
        d.named("zebra.desktop", "Zebra", "")
            .named("apple.desktop", "Apple", "")
            .named("quiet.desktop", "Quiet", "NoDisplay=true");
        let names: Vec<String> = all_applications(&d.xdg).into_iter().map(|a| a.name).collect();
        assert_eq!(names, ["Apple", "Quiet", "Zebra"], "sorted by name");
    }

    #[test]
    fn the_command_line_is_the_entrys_own() {
        // The seam with the half that runs it: what this module reads has to survive the trip
        // through the type and come out as the argv the entry asked for.
        let d = Desktop::new();
        d.write(
            "data-system/applications/viewer.desktop",
            "[Desktop Entry]\nType=Application\nName=Viewer\nExec=view --window %U\nTerminal=false\n",
        );
        d.write(
            "data-system/applications/mimeinfo.cache",
            "[MIME Cache]\nimage/png=viewer.desktop;\n",
        );
        let app = &applications_for(&d.xdg, "image/png")[0];
        assert_eq!(
            app.command_line(&[PathBuf::from("/a/b c.png")]),
            ["view", "--window", "file:///a/b%20c.png"]
        );
        assert!(!app.terminal);
    }

    // -----------------------------------------------------------------------
    // Changing the default
    // -----------------------------------------------------------------------

    #[test]
    fn setting_a_default_writes_the_file_the_rest_of_the_desktop_reads() {
        let d = Desktop::new();
        d.app("editor.desktop", "");
        let written = set_default(&d.xdg, "text/plain", "editor.desktop").unwrap();
        assert_eq!(written, d.xdg.config_home.join("mimeapps.list"));
        assert_eq!(current_default(&d.xdg, "text/plain").as_deref(), Some("editor.desktop"));
    }

    #[test]
    fn setting_a_default_to_something_that_is_not_installed_is_refused() {
        let d = Desktop::new();
        assert!(matches!(
            set_default(&d.xdg, "text/plain", "imaginary.desktop"),
            Err(SetDefaultError::NoSuchApplication(_))
        ));
        assert!(!d.xdg.user_mimeapps().exists(), "and nothing is written");
    }

    #[test]
    fn the_rest_of_the_file_survives_the_edit() {
        let d = Desktop::new();
        d.app("new.desktop", "");
        d.write(
            "config/mimeapps.list",
            "# somebody's own comment\n\
             [Default Applications]\n\
             text/plain=old.desktop\n\
             image/png=viewer.desktop\n\
             \n\
             [Added Associations]\n\
             text/plain=old.desktop;\n",
        );
        set_default(&d.xdg, "text/plain", "new.desktop").unwrap();

        let after = std::fs::read_to_string(d.xdg.user_mimeapps()).unwrap();
        assert!(after.contains("# somebody's own comment"), "comments are not ours to delete");
        assert!(after.contains("image/png=viewer.desktop"), "other types are untouched");
        assert!(after.contains("[Added Associations]"));
        assert!(after.contains("text/plain=new.desktop"));
        assert!(!after.contains("text/plain=old.desktop\n"), "the old default is replaced");
        assert!(
            after.contains("[Added Associations]\ntext/plain=old.desktop;"),
            "and the association of the same name in another group is not the one that changed"
        );
    }

    #[test]
    fn a_type_that_is_not_in_the_file_yet_joins_the_group_that_is() {
        let d = Desktop::new();
        d.app("new.desktop", "");
        d.write(
            "config/mimeapps.list",
            "[Default Applications]\nimage/png=viewer.desktop\n\n[Added Associations]\nimage/png=viewer.desktop;\n",
        );
        set_default(&d.xdg, "text/plain", "new.desktop").unwrap();
        let after = std::fs::read_to_string(d.xdg.user_mimeapps()).unwrap();
        let defaults = after.find("[Default Applications]").unwrap();
        let added = after.find("[Added Associations]").unwrap();
        let line = after.find("text/plain=new.desktop").unwrap();
        assert!(
            defaults < line && line < added,
            "the new line belongs inside the group it is a default in, not after the file"
        );
    }

    #[test]
    fn a_file_with_no_defaults_group_grows_one() {
        let d = Desktop::new();
        d.app("new.desktop", "");
        d.write("config/mimeapps.list", "[Added Associations]\nimage/png=viewer.desktop;\n");
        set_default(&d.xdg, "text/plain", "new.desktop").unwrap();
        let after = std::fs::read_to_string(d.xdg.user_mimeapps()).unwrap();
        assert!(after.contains("[Added Associations]"));
        assert!(after.contains("[Default Applications]\ntext/plain=new.desktop"));
    }

    #[test]
    fn setting_a_default_lifts_the_removal_that_would_have_cancelled_it() {
        let d = Desktop::new();
        d.app("editor.desktop", "").app("other.desktop", "");
        d.write(
            "config/mimeapps.list",
            "[Removed Associations]\ntext/plain=editor.desktop;other.desktop;\n",
        );
        set_default(&d.xdg, "text/plain", "editor.desktop").unwrap();

        // The point of the lift: without it the resolver filters out the line just written and
        // the type quietly keeps its old default.
        assert_eq!(current_default(&d.xdg, "text/plain").as_deref(), Some("editor.desktop"));
        let after = std::fs::read_to_string(d.xdg.user_mimeapps()).unwrap();
        assert!(after.contains("text/plain=other.desktop;"), "the other removal stands");
    }

    #[test]
    fn a_removal_group_that_becomes_empty_loses_its_line_rather_than_keeping_a_blank_one() {
        let d = Desktop::new();
        d.app("editor.desktop", "");
        d.write("config/mimeapps.list", "[Removed Associations]\ntext/plain=editor.desktop;\n");
        set_default(&d.xdg, "text/plain", "editor.desktop").unwrap();
        let after = std::fs::read_to_string(d.xdg.user_mimeapps()).unwrap();
        assert!(!after.contains("text/plain=\n"), "an empty value is not a removal of nothing");
    }

    #[test]
    fn writing_the_file_twice_is_the_same_as_writing_it_once() {
        // A rewriter that appends is a file that grows a line every time somebody changes their
        // mind, and the resolver would keep reading the first one.
        let d = Desktop::new();
        d.app("a.desktop", "").app("b.desktop", "");
        set_default(&d.xdg, "text/plain", "a.desktop").unwrap();
        set_default(&d.xdg, "text/plain", "b.desktop").unwrap();
        let after = std::fs::read_to_string(d.xdg.user_mimeapps()).unwrap();
        assert_eq!(after.matches("text/plain=").count(), 1);
        assert_eq!(current_default(&d.xdg, "text/plain").as_deref(), Some("b.desktop"));
    }
}
