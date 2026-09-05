//! Plugin declarations (D9).
//!
//! A plugin is a separate process that speaks the same JSON-RPC as every other client. It is
//! declared, not discovered: one `plugins/<name>.toml` per plugin, naming a command to run.
//! The Core never scans `$PATH` for plugins and never runs anything it was not told about.
//!
//! ```toml
//! # $XDG_CONFIG_HOME/hippocampus/plugins/main-brain.toml
//! kind = "sync"
//! command = "/opt/hippocampus-ext/bin/bridge"
//! args = ["--profile", "home"]
//! ```
//!
//! What the command does with a batch — which host it talks to, how it authenticates — is the
//! plugin's business and lives with the plugin. The declaration carries no endpoint and no
//! credential, so it is safe to show in `sync.status`.

use crate::error::{Error, Result};
use std::path::{Path, PathBuf};

/// What a plugin is for. The Core dispatches to a plugin by kind; a `sync` plugin is handed
/// batches, and nothing else is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginKind {
    Sync,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginDecl {
    pub kind: PluginKind,
    /// The program to run. Absolute, or found on `$PATH` by the OS when the plugin runs.
    pub command: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    /// How long one exchange may take before the plugin is killed, in milliseconds.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_timeout_ms() -> u64 {
    30_000
}

impl PluginDecl {
    /// Read `plugins/<name>.toml` under `plugins_dir`.
    pub fn load(plugins_dir: &Path, name: &str) -> Result<Self> {
        if name.is_empty()
            || name.contains('/')
            || name.contains('\\')
            || name.starts_with('.')
        {
            return Err(Error::config(
                plugins_dir,
                format!("{name:?} is not a plugin name; use the file name without .toml"),
            ));
        }
        let path = plugins_dir.join(format!("{name}.toml"));
        let text = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
        Self::from_toml(&text, &path)
    }

    pub fn from_toml(text: &str, path: &Path) -> Result<Self> {
        let decl: PluginDecl =
            toml::from_str(text).map_err(|e| Error::config(path, e.to_string()))?;
        if decl.command.as_os_str().is_empty() {
            return Err(Error::config(path, "command must not be empty"));
        }
        if decl.timeout_ms == 0 {
            return Err(Error::config(path, "timeout_ms must be at least 1"));
        }
        Ok(decl)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_declaration_is_read_by_name_and_names_no_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("bridge.toml"),
            "kind = \"sync\"\ncommand = \"bridge\"\nargs = [\"--quiet\"]\n",
        )
        .unwrap();
        let decl = PluginDecl::load(dir.path(), "bridge").unwrap();
        assert_eq!(decl.kind, PluginKind::Sync);
        assert_eq!(decl.command, PathBuf::from("bridge"));
        assert_eq!(decl.args, vec!["--quiet".to_string()]);
        assert_eq!(decl.timeout_ms, 30_000);
    }

    #[test]
    fn a_name_that_is_a_path_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        for bad in ["../x", "a/b", ".hidden", ""] {
            assert!(PluginDecl::load(dir.path(), bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn unknown_fields_are_refused_so_a_typo_cannot_pass_as_an_option() {
        let err = PluginDecl::from_toml(
            "kind = \"sync\"\ncommand = \"x\"\nendpoint = \"nope\"\n",
            Path::new("<test>"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("endpoint"), "{err}");
    }
}
