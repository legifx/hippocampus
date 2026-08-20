//! Turning user and group numbers into names.
//!
//! A file manager that shows `1000` has told you less than it knows. The names live in
//! `/etc/passwd` and `/etc/group`, which are read once per listing rather than once per entry —
//! a directory of ten thousand files must not mean ten thousand file reads to render a column.
//!
//! **Read directly rather than through `getpwuid`.** That call consults NSS, which on a machine
//! with LDAP or SSSD configured can block on the network for as long as it likes. The daemon
//! promises no network access by default (R5) and a directory listing is the wrong place to
//! discover that a directory server is down. The cost is that names from a network directory do
//! not resolve — they show as numbers, which is exactly what they would show as if the lookup
//! timed out, and it happens instantly instead.

use std::collections::HashMap;

/// Names for the ids on this machine, read once and reused.
#[derive(Debug, Default, Clone)]
pub struct Names {
    users: HashMap<u32, String>,
    groups: HashMap<u32, String>,
}

impl Names {
    /// Read `/etc/passwd` and `/etc/group`.
    ///
    /// A file that cannot be read gives an empty map rather than an error: not knowing a name
    /// is a column showing a number, and never a reason to fail a listing.
    pub fn from_system() -> Self {
        Self {
            users: parse("/etc/passwd"),
            groups: parse("/etc/group"),
        }
    }

    pub fn user(&self, id: u32) -> cortex_core::tools::Owner {
        cortex_core::tools::Owner {
            id,
            name: self.users.get(&id).cloned(),
        }
    }

    pub fn group(&self, id: u32) -> cortex_core::tools::Owner {
        cortex_core::tools::Owner {
            id,
            name: self.groups.get(&id).cloned(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.users.is_empty() && self.groups.is_empty()
    }
}

/// Both files have the same shape for the two fields that matter: `name:x:id:…`.
fn parse(path: &str) -> HashMap<u32, String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };

    let mut out = HashMap::new();
    for line in text.lines() {
        // A comment or a blank line, or NIS's `+` continuation lines, which are not entries.
        if line.starts_with('#') || line.starts_with('+') || line.is_empty() {
            continue;
        }
        let mut fields = line.split(':');
        let (Some(name), Some(_), Some(id)) = (fields.next(), fields.next(), fields.next()) else {
            continue;
        };
        if let Ok(id) = id.parse::<u32>() {
            // First wins: a duplicated id in `/etc/passwd` is a misconfiguration, and picking
            // the first is at least the same answer `id` gives.
            out.entry(id).or_insert_with(|| name.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_passwd_file_yields_names() {
        // The shape of a real one, including the entries that are not users.
        let dir = tempfile::tempdir().unwrap();
        let passwd = dir.path().join("passwd");
        std::fs::write(
            &passwd,
            "# a comment\n\
             root:x:0:0:root:/root:/bin/bash\n\
             daemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin\n\
             \n\
             someone:x:1000:1000:Someone:/home/someone:/bin/bash\n",
        )
        .unwrap();

        let names = parse(passwd.to_str().unwrap());
        assert_eq!(names.get(&0).map(String::as_str), Some("root"));
        assert_eq!(names.get(&1000).map(String::as_str), Some("someone"));
        assert_eq!(names.len(), 3);
    }

    #[test]
    fn a_missing_file_is_a_column_of_numbers_rather_than_an_error() {
        assert!(parse("/nonexistent/passwd").is_empty());

        let names = Names {
            users: HashMap::new(),
            groups: HashMap::new(),
        };
        assert_eq!(names.user(1000).label(), "1000");
    }

    #[test]
    fn a_known_id_shows_its_name() {
        let mut users = HashMap::new();
        users.insert(1000, "someone".to_string());
        let names = Names {
            users,
            groups: HashMap::new(),
        };
        assert_eq!(names.user(1000).label(), "someone");
        assert_eq!(names.user(1001).label(), "1001", "an unknown id is still an answer");
    }

    #[test]
    fn the_real_system_has_a_root() {
        // The one assertion that touches the machine, because a parser that works on a fixture
        // and not on `/etc/passwd` would be a parser for a file format nobody uses.
        let names = Names::from_system();
        if !names.is_empty() {
            assert_eq!(names.user(0).name.as_deref(), Some("root"));
        }
    }
}
