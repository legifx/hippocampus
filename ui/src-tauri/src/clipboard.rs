//! Cut and copy that other applications can hear.
//!
//! Until now the clipboard was this window's own: cut remembered some paths, paste turned them
//! into a transfer, and nothing outside the process ever knew. That is fine for a launcher and
//! useless for a file manager, because M6's actual test is a week of exclusive use — and a file
//! manager you cannot copy *out of* fails that test on the first day.
//!
//! ## Why this lives in the shell and not in the daemon
//!
//! The daemon is headless. It has no display connection and must not acquire one: it runs as a
//! user service, it is meant to be reachable from a terminal over a socket, and a clipboard is
//! a property of a *session*, not of an index. The shell is the process with a Wayland
//! connection, so the clipboard is the shell's, alongside the window and the hotkey.
//!
//! ## The formats, and why there are four
//!
//! There is no standard for "these files are on the clipboard". There is a convention, and it
//! is a GNOME one that everything else adopted:
//!
//! - **`x-special/gnome-copied-files`** — `copy` or `cut`, a newline, then one `file://` URI per
//!   line. This is the only format that carries the *verb*, which is what makes "cut" possible
//!   at all. Nautilus, Nemo, Caja, Thunar and PCManFM all read and write it.
//! - **`text/uri-list`** — the freedesktop standard (RFC 2483), CRLF-separated. Everything
//!   understands it; none of it knows whether a cut was meant, so a paste from this alone is a
//!   copy. This is what makes a drop into GIMP or a browser work.
//! - **`application/x-kde-cutselection`** — `1` when it was a cut. KDE's way of saying the same
//!   thing GNOME says in its first line, and the reason Dolphin gets a cut right.
//! - **`text/plain`** — the plain paths, so pasting into a terminal or an editor gives
//!   something a person meant rather than nothing.
//!
//! All four are offered at once, which costs nothing: the clipboard advertises targets and the
//! receiver picks. Reading goes the other way, most specific first, so that a cut from any of
//! the three families stays a cut.
//!
//! ## Encoding
//!
//! A file URI percent-encodes everything outside the unreserved set. That is not pedantry: a
//! file name may contain a **newline**, and both formats here are line-oriented, so a name with
//! a newline in it would otherwise become two files — one of which does not exist and one of
//! which might. It also means paths that are not valid UTF-8 survive, because the encoding is
//! over bytes rather than characters.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// The GTK half. Linux-only, because a Wayland compositor is the only thing this runs under and
/// pretending otherwise would mean a second implementation nobody exercises.
#[cfg(target_os = "linux")]
pub mod session;

/// How long a clipboard round trip may take before the answer is "nothing".
///
/// Reading the clipboard means asking whichever application owns it, and GTK does that by
/// running a nested main loop until the owner replies. An application that has hung — or one
/// whose window has gone while the compositor still names it the owner — would otherwise hang
/// this one. Two seconds is far longer than any real answer takes and short enough that a paste
/// which is going nowhere says so.
const ROUND_TRIP_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, thiserror::Error)]
pub enum ClipboardError {
    #[error("the clipboard is only available while the window is on screen")]
    NoWindow,
    #[error("the clipboard did not answer")]
    TimedOut,
    #[error("the compositor refused the selection; the window has to be focused to copy")]
    Refused,
}

impl serde::Serialize for ClipboardError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

/// Run a closure on the GTK thread and wait for its answer.
///
/// Every function in [`session`] must run there — GTK's clipboard is not thread-safe and gtk-rs
/// asserts it — while Tauri's commands run on a worker pool. This is the hop.
///
/// It cannot deadlock: a command never runs on the main thread, so the main thread is never
/// waiting on itself. The timeout is for the other case, where the main thread is busy inside a
/// clipboard round trip of its own.
#[cfg(target_os = "linux")]
pub fn on_main_thread<T: Send + 'static>(
    app: &tauri::AppHandle,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, ClipboardError> {

    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(work());
    })
    .map_err(|_| ClipboardError::NoWindow)?;

    rx.recv_timeout(ROUND_TRIP_TIMEOUT)
        .map_err(|_| ClipboardError::TimedOut)
}

/// The verb. The same two words the daemon's `fs.transfer` uses, so nothing has to be
/// translated between the clipboard and the operation it becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Copy,
    #[serde(rename = "move")]
    Move,
}

impl Mode {
    /// What GNOME's format calls it. `cut`, not `move` — this is one of the places the
    /// convention and the daemon's vocabulary differ, and the translation belongs here.
    fn gnome_verb(self) -> &'static str {
        match self {
            Mode::Copy => "copy",
            Mode::Move => "cut",
        }
    }
}

/// What is on the clipboard: some files, and what should happen to them.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Selection {
    pub mode: Mode,
    pub paths: Vec<PathBuf>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

// ---------------------------------------------------------------------------
// The MIME types, by the names the receiving applications look for.
// ---------------------------------------------------------------------------

pub const GNOME_COPIED_FILES: &str = "x-special/gnome-copied-files";
pub const URI_LIST: &str = "text/uri-list";
pub const KDE_CUT_SELECTION: &str = "application/x-kde-cutselection";
pub const UTF8_TEXT: &str = "text/plain;charset=utf-8";

// ---------------------------------------------------------------------------
// URIs. Pure, byte-oriented, and tested against the names that actually break things.
// ---------------------------------------------------------------------------

/// `/home/user/two words.txt` → `file:///home/user/two%20words.txt`.
///
/// Everything outside the unreserved set of RFC 3986 is encoded, except `/`, which is the path
/// separator and has to stay one. Encoding more than strictly necessary is safe — every reader
/// decodes — and encoding less is not.
pub fn encode_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;

    let bytes = path.as_os_str().as_bytes();
    let mut out = String::with_capacity(bytes.len() + 8);
    out.push_str("file://");
    for &byte in bytes {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The other direction, accepting rather more than it produces.
///
/// Other applications write URIs to their own taste: with a `localhost` authority, with
/// characters left unencoded that did not have to be, occasionally as a bare path. All of those
/// name a real file and refusing them would be pedantry with a cost.
///
/// Returns `None` for anything that is not a local absolute path, which includes `http://` and
/// `trash://` — both of which do turn up on a clipboard, and neither of which this can copy.
pub fn decode_uri(uri: &str) -> Option<PathBuf> {
    let uri = uri.trim();
    if uri.is_empty() {
        return None;
    }

    let rest = if let Some(rest) = uri.strip_prefix("file://") {
        // `file:///path` leaves `/path`; `file://localhost/path` leaves `localhost/path`, and
        // an authority naming another machine is not something this can open.
        match rest.strip_prefix("localhost/") {
            Some(path) => {
                let mut owned = String::with_capacity(path.len() + 1);
                owned.push('/');
                owned.push_str(path);
                return decode_percent(&owned);
            }
            None if rest.starts_with('/') => rest,
            None => return None,
        }
    } else if uri.starts_with('/') {
        uri
    } else {
        return None;
    };

    decode_percent(rest)
}

fn decode_percent(text: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;

    let source = text.as_bytes();
    let mut bytes = Vec::with_capacity(source.len());
    let mut i = 0;
    while i < source.len() {
        if source[i] == b'%' && i + 2 < source.len() {
            match hex_pair(source[i + 1], source[i + 2]) {
                Some(byte) => {
                    bytes.push(byte);
                    i += 3;
                    continue;
                }
                // A stray `%` that is not an escape. Some applications write them; treating it
                // as a literal is what every browser does and loses nothing.
                None => {}
            }
        }
        bytes.push(source[i]);
        i += 1;
    }

    if bytes.is_empty() || bytes[0] != b'/' {
        return None;
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}

fn hex_pair(high: u8, low: u8) -> Option<u8> {
    Some(hex(high)? << 4 | hex(low)?)
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// The payloads.
// ---------------------------------------------------------------------------

/// `copy\nfile:///a\nfile:///b` — the verb, then the files.
pub fn gnome_payload(selection: &Selection) -> Vec<u8> {
    let mut out = String::from(selection.mode.gnome_verb());
    for path in &selection.paths {
        out.push('\n');
        out.push_str(&encode_uri(path));
    }
    out.into_bytes()
}

/// Read what another file manager put there.
///
/// Tolerant about the verb line because implementations disagree in small ways — Nautilus has
/// written both `cut` and `copy` with and without a trailing newline, and at least one writes
/// `copy\n\n` when the list is empty.
pub fn parse_gnome(payload: &[u8]) -> Option<Selection> {
    let text = String::from_utf8_lossy(payload);
    let mut lines = text.split('\n');
    let verb = lines.next()?.trim();
    let mode = match verb {
        "cut" => Mode::Move,
        "copy" => Mode::Copy,
        // Not this format at all. Better to say so than to guess a verb and move files.
        _ => return None,
    };

    let mut paths = Vec::new();
    for line in lines {
        if let Some(path) = decode_uri(line) {
            paths.push(path);
        }
    }
    Some(Selection { mode, paths })
}

/// RFC 2483: CRLF-separated, and a trailing one.
pub fn uri_list_payload(selection: &Selection) -> Vec<u8> {
    let mut out = String::new();
    for path in &selection.paths {
        out.push_str(&encode_uri(path));
        out.push_str("\r\n");
    }
    out.into_bytes()
}

/// Lines that begin with `#` are comments, by that same RFC. In practice nobody writes them and
/// a file called `#notes` is a real thing, so only a `#` at the start of a *line* counts — which
/// is what the RFC says and what makes `file:///home/user/%23notes` survive.
pub fn parse_uri_list(payload: &[u8]) -> Vec<PathBuf> {
    let text = String::from_utf8_lossy(payload);
    let mut paths = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(path) = decode_uri(line) {
            paths.push(path);
        }
    }
    paths
}

/// The paths, one per line, for pasting into a terminal or a note.
///
/// Lossy for a path that is not valid UTF-8, because `text/plain` has no way to be anything
/// else. The formats above carry those paths exactly; this one is a convenience.
pub fn text_payload(selection: &Selection) -> Vec<u8> {
    let mut out = String::new();
    for (index, path) in selection.paths.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&path.to_string_lossy());
    }
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection(mode: Mode, paths: &[&str]) -> Selection {
        Selection {
            mode,
            paths: paths.iter().map(PathBuf::from).collect(),
        }
    }

    #[test]
    fn a_plain_path_encodes_to_a_plain_uri() {
        assert_eq!(
            encode_uri(Path::new("/home/user/notes.txt")),
            "file:///home/user/notes.txt"
        );
    }

    #[test]
    fn the_characters_that_break_things_are_encoded() {
        // A space is the common one. The others are the ones that would end the URI early, or
        // turn one file into two.
        assert_eq!(encode_uri(Path::new("/a/two words")), "file:///a/two%20words");
        assert_eq!(encode_uri(Path::new("/a/b#c")), "file:///a/b%23c");
        assert_eq!(encode_uri(Path::new("/a/b?c")), "file:///a/b%3Fc");
        assert_eq!(encode_uri(Path::new("/a/100%")), "file:///a/100%25");
        assert_eq!(encode_uri(Path::new("/a/b\nc")), "file:///a/b%0Ac");
        assert_eq!(encode_uri(Path::new("/a/Grüße")), "file:///a/Gr%C3%BC%C3%9Fe");
    }

    #[test]
    fn a_name_with_a_newline_in_it_stays_one_file() {
        // The case both line-oriented formats would otherwise get wrong, and the reason the
        // encoding is not optional.
        let one = selection(Mode::Copy, &["/a/two\nlines"]);
        let payload = gnome_payload(&one);
        assert_eq!(String::from_utf8_lossy(&payload), "copy\nfile:///a/two%0Alines");

        let back = parse_gnome(&payload).unwrap();
        assert_eq!(back.paths, vec![PathBuf::from("/a/two\nlines")]);
        assert_eq!(back.paths.len(), 1, "one file, not two");
    }

    #[test]
    fn every_path_survives_the_round_trip() {
        for name in [
            "/home/user/notes.txt",
            "/a/two words.txt",
            "/a/Grüße und Küsse.pdf",
            "/a/100% done",
            "/a/b#c?d&e",
            "/a/emoji 🐘.png",
            "/a/'quoted'",
            "/a/back\\slash",
            "/",
        ] {
            let path = PathBuf::from(name);
            assert_eq!(decode_uri(&encode_uri(&path)), Some(path.clone()), "{name}");
        }
    }

    #[test]
    fn a_path_that_is_not_utf8_survives_too() {
        // Encoding over bytes rather than characters is what makes this work. A name like this
        // arrives on any machine that has ever unpacked an old archive.
        use std::os::unix::ffi::OsStringExt;
        let path = PathBuf::from(std::ffi::OsString::from_vec(b"/a/caf\xe9".to_vec()));
        assert_eq!(encode_uri(&path), "file:///a/caf%E9");
        assert_eq!(decode_uri("file:///a/caf%E9"), Some(path));
    }

    #[test]
    fn uris_from_other_applications_are_read_generously() {
        let expected = Some(PathBuf::from("/home/user/notes.txt"));
        assert_eq!(decode_uri("file:///home/user/notes.txt"), expected);
        assert_eq!(decode_uri("file://localhost/home/user/notes.txt"), expected);
        assert_eq!(decode_uri("  file:///home/user/notes.txt  "), expected);
        assert_eq!(decode_uri("/home/user/notes.txt"), expected, "a bare path");
        // Lowercase escapes are valid and some applications write them.
        assert_eq!(decode_uri("file:///a/two%20words"), Some(PathBuf::from("/a/two words")));
        assert_eq!(decode_uri("file:///a/two%20words"), decode_uri("file:///a/two%20words"));
    }

    #[test]
    fn what_cannot_be_copied_is_refused_rather_than_guessed() {
        assert_eq!(decode_uri("https://example.invalid/a.txt"), None);
        assert_eq!(decode_uri("trash:///notes.txt"), None);
        assert_eq!(decode_uri("file://otherhost/a.txt"), None, "another machine");
        assert_eq!(decode_uri("relative/path"), None);
        assert_eq!(decode_uri(""), None);
    }

    #[test]
    fn a_stray_percent_is_a_literal_rather_than_a_failure() {
        // Applications do write these. A browser treats them literally; so does this.
        assert_eq!(decode_uri("file:///a/50%off"), Some(PathBuf::from("/a/50%off")));
        assert_eq!(decode_uri("file:///a/ends%"), Some(PathBuf::from("/a/ends%")));
    }

    #[test]
    fn the_gnome_payload_says_the_verb_first() {
        let copied = gnome_payload(&selection(Mode::Copy, &["/a/one", "/a/two"]));
        assert_eq!(
            String::from_utf8_lossy(&copied),
            "copy\nfile:///a/one\nfile:///a/two"
        );

        // `cut`, not `move`: the convention's word, not the daemon's.
        let cut = gnome_payload(&selection(Mode::Move, &["/a/one"]));
        assert_eq!(String::from_utf8_lossy(&cut), "cut\nfile:///a/one");
    }

    #[test]
    fn a_cut_from_another_file_manager_stays_a_cut() {
        let from_nautilus = b"cut\nfile:///home/user/a.txt\nfile:///home/user/b.txt";
        let parsed = parse_gnome(from_nautilus).unwrap();
        assert_eq!(parsed.mode, Mode::Move);
        assert_eq!(parsed.paths.len(), 2);
        assert_eq!(parsed.paths[0], PathBuf::from("/home/user/a.txt"));
    }

    #[test]
    fn a_trailing_newline_and_an_empty_list_are_both_tolerated() {
        assert_eq!(parse_gnome(b"copy\nfile:///a/one\n").unwrap().paths.len(), 1);
        assert_eq!(parse_gnome(b"copy\n").unwrap().paths.len(), 0);
        assert_eq!(parse_gnome(b"copy").unwrap().paths.len(), 0);
    }

    #[test]
    fn something_that_is_not_this_format_is_refused_rather_than_guessed() {
        // Guessing a verb here would mean moving files somebody meant to copy.
        assert_eq!(parse_gnome(b"file:///a/one"), None);
        assert_eq!(parse_gnome(b""), None);
        assert_eq!(parse_gnome(b"COPY\nfile:///a/one"), None);
    }

    #[test]
    fn the_uri_list_is_crlf_separated_with_a_trailing_one() {
        // RFC 2483, and the receivers that get it wrong get it wrong in the lenient direction.
        let payload = uri_list_payload(&selection(Mode::Copy, &["/a/one", "/a/two"]));
        assert_eq!(
            String::from_utf8_lossy(&payload),
            "file:///a/one\r\nfile:///a/two\r\n"
        );
    }

    #[test]
    fn a_uri_list_is_read_with_or_without_carriage_returns() {
        assert_eq!(parse_uri_list(b"file:///a/one\r\nfile:///a/two\r\n").len(), 2);
        assert_eq!(parse_uri_list(b"file:///a/one\nfile:///a/two").len(), 2);
        assert_eq!(parse_uri_list(b"").len(), 0);
    }

    #[test]
    fn a_comment_line_is_skipped_and_a_file_called_hash_is_not() {
        // The `#` that matters is the one at the start of a line. A file whose *name* begins
        // with one arrives percent-encoded and is not a comment.
        let payload = b"# a comment\r\nfile:///a/one\r\nfile:///a/%23notes\r\n";
        let paths = parse_uri_list(payload);
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[1], PathBuf::from("/a/#notes"));
    }

    #[test]
    fn the_plain_text_is_the_paths_and_nothing_else() {
        let payload = text_payload(&selection(Mode::Copy, &["/a/one", "/a/two"]));
        assert_eq!(String::from_utf8_lossy(&payload), "/a/one\n/a/two");
    }

    #[test]
    fn a_selection_knows_when_it_is_empty() {
        assert!(selection(Mode::Copy, &[]).is_empty());
        assert!(!selection(Mode::Copy, &["/a"]).is_empty());
    }
}
