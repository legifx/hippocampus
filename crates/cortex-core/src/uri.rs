//! `file://` URIs, encoded the way the rest of the desktop encodes them.
//!
//! One implementation, because the places this appears all have to agree with software that is
//! not ours: the thumbnail cache names its files after the MD5 of this string and another
//! application's thumbnail is only found if the byte before the hash matched; a desktop entry's
//! `%u` hands the URI to a program that will open the wrong file, or none, if it is escaped
//! differently; and `text/uri-list` on the clipboard is read by Nautilus. A second encoder that
//! disagreed by one character would be wrong in a way that looks like the other program's bug.

use std::path::Path;

/// The URI of a local path, percent-encoded as the standard requires.
///
/// Encoded by hand rather than with a dependency: the rule is "everything outside the unreserved
/// set", and it is four lines.
pub fn file_uri(path: &Path) -> String {
    let mut out = String::from("file://");
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn an_ordinary_path_is_left_alone() {
        assert_eq!(file_uri(&PathBuf::from("/home/user/notes.txt")), "file:///home/user/notes.txt");
    }

    #[test]
    fn everything_outside_the_unreserved_set_is_escaped() {
        // A space and a `#` are the two that break a URI reader rather than merely looking odd:
        // one truncates the path and the other starts a fragment.
        assert_eq!(file_uri(&PathBuf::from("/a/two words")), "file:///a/two%20words");
        assert_eq!(file_uri(&PathBuf::from("/a/b#c")), "file:///a/b%23c");
        assert_eq!(file_uri(&PathBuf::from("/a/100%")), "file:///a/100%25");
    }

    #[test]
    fn a_name_that_is_not_ascii_is_escaped_as_its_utf8_bytes() {
        assert_eq!(file_uri(&PathBuf::from("/tmp/ü.png")), "file:///tmp/%C3%BC.png");
    }
}
