//! `cortex setup` — the onboarding (M9).
//!
//! Under ten minutes from a fresh install to a working search, with three questions: what to
//! index, what to read, and whether to fetch a model. Everything else has a default that is
//! also the safe one (R3, R5). The answers become `config.toml`; the daemon is started if
//! systemd is there to start it; the first index run is kicked off and reported.
//!
//! The plan is a value, so what the questions produce is tested without a terminal.

use anyhow::{Context, Result};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelChoice {
    None,
    Cpu,
    Gpu,
}

impl ModelChoice {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "none" | "n" | "no" => Some(Self::None),
            "cpu" | "c" => Some(Self::Cpu),
            "gpu" | "g" | "cuda" => Some(Self::Gpu),
            _ => None,
        }
    }
}

/// What the setup decided. Rendered to `config.toml` by [`Plan::render`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub metadata_roots: Vec<PathBuf>,
    pub content_roots: Vec<PathBuf>,
    pub model: ModelChoice,
}

impl Plan {
    /// The defaults for a home directory: index all of it, read the folders people keep
    /// documents and code in — only those that exist.
    pub fn defaults(home: &Path) -> Self {
        let content_roots = ["Documents", "Projects", "Notes", "Dokumente", "Projekte"]
            .iter()
            .map(|d| home.join(d))
            .filter(|p| p.is_dir())
            .collect();
        Self {
            metadata_roots: vec![home.to_path_buf()],
            content_roots,
            model: ModelChoice::None,
        }
    }

    /// The configuration file, with the safe defaults left to the daemon's defaults.
    pub fn render(&self) -> String {
        let list = |paths: &[PathBuf]| {
            paths
                .iter()
                .map(|p| format!("{:?}", p.display().to_string()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut out = String::new();
        out.push_str("# Written by `cortex setup`. Every setting not named here keeps its default;\n");
        out.push_str("# `cortexd --check` prints the whole effective configuration.\n\n");
        out.push_str("[index]\n");
        out.push_str("# Paths and stats. Cheap, may span the whole home directory.\n");
        out.push_str(&format!("metadata_roots = [{}]\n", list(&self.metadata_roots)));
        out.push_str("# What is *inside* files is read only here (hard rule R3).\n");
        out.push_str(&format!("content_roots = [{}]\n", list(&self.content_roots)));
        if self.model != ModelChoice::None {
            out.push_str("\n[embedding]\n");
            out.push_str(&format!(
                "device = \"{}\"\n",
                if self.model == ModelChoice::Gpu { "cuda" } else { "cpu" }
            ));
        }
        out
    }
}

/// Read paths from one answer line: comma-separated, `~` expanded, empty keeps the default.
pub fn parse_paths(answer: &str, home: &Path, default: &[PathBuf]) -> Vec<PathBuf> {
    let trimmed = answer.trim();
    if trimmed.is_empty() {
        return default.to_vec();
    }
    if trimmed == "-" {
        return Vec::new();
    }
    trimmed
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| match s.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None if s == "~" => home.to_path_buf(),
            None => PathBuf::from(s),
        })
        .collect()
}

fn ask(prompt: &str) -> Result<String> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line)
}

fn show(paths: &[PathBuf]) -> String {
    if paths.is_empty() {
        "(none)".to_string()
    } else {
        paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
    }
}

/// Ask the three questions, or take the answers from the flags.
pub fn plan_interactively(
    home: &Path,
    yes: bool,
    metadata: Option<Vec<PathBuf>>,
    content: Option<Vec<PathBuf>>,
    model: Option<ModelChoice>,
) -> Result<Plan> {
    let mut plan = Plan::defaults(home);
    if let Some(m) = metadata {
        plan.metadata_roots = m;
    }
    if let Some(c) = content {
        plan.content_roots = c;
    }
    if let Some(m) = model {
        plan.model = m;
    }
    if yes {
        return Ok(plan);
    }

    println!("Hippocampus setup. Three questions; Enter keeps the default, `-` means none.\n");
    let answer = ask(&format!(
        "1. Index which directories (paths and stats only)?\n   [{}] ",
        show(&plan.metadata_roots)
    ))?;
    plan.metadata_roots = parse_paths(&answer, home, &plan.metadata_roots);

    let answer = ask(&format!(
        "2. Read the contents of files in which directories (for search by meaning)?\n   [{}] ",
        show(&plan.content_roots)
    ))?;
    plan.content_roots = parse_paths(&answer, home, &plan.content_roots);

    loop {
        let answer = ask(
            "3. Download the embedding model (about 120 MB, the only network access there is)?\n   none / cpu / gpu [none] ",
        )?;
        match ModelChoice::parse(&answer) {
            Some(choice) => {
                plan.model = choice;
                break;
            }
            None => println!("   please answer none, cpu or gpu"),
        }
    }
    Ok(plan)
}

/// Write the configuration, refusing to overwrite unless told to.
pub fn write_config(path: &Path, plan: &Plan, force: bool) -> Result<()> {
    if path.exists() && !force {
        anyhow::bail!(
            "{} exists; run with --force to replace it, or edit it by hand",
            path.display()
        );
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, plan.render()).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Start the daemon through systemd if that is how it is installed. Returns what happened,
/// in words, for the summary.
pub fn start_daemon() -> String {
    let unit = "hippocampus-cortexd.service";
    let known = std::process::Command::new("systemctl")
        .args(["--user", "cat", unit])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !known {
        return format!(
            "no systemd unit {unit} found; start the daemon yourself with `cortexd`"
        );
    }
    match std::process::Command::new("systemctl")
        .args(["--user", "enable", "--now", unit])
        .status()
    {
        Ok(s) if s.success() => format!("{unit} enabled and started"),
        Ok(s) => format!("systemctl returned {s}; start the daemon yourself with `cortexd`"),
        Err(e) => format!("could not run systemctl ({e}); start the daemon yourself with `cortexd`"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_index_the_home_and_read_only_folders_that_exist() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join("Documents")).unwrap();
        let plan = Plan::defaults(home.path());
        assert_eq!(plan.metadata_roots, vec![home.path().to_path_buf()]);
        assert_eq!(plan.content_roots, vec![home.path().join("Documents")]);
        assert_eq!(plan.model, ModelChoice::None, "no download without being asked (R5)");
    }

    #[test]
    fn the_rendered_file_is_a_valid_configuration_with_the_answers_in_it() {
        let home = tempfile::tempdir().unwrap();
        let plan = Plan {
            metadata_roots: vec![home.path().to_path_buf()],
            content_roots: vec![home.path().join("Documents")],
            model: ModelChoice::Gpu,
        };
        let text = plan.render();
        let config = cortex_core::config::Config::from_toml(&text, Path::new("<test>")).unwrap();
        assert_eq!(config.index.metadata_roots, plan.metadata_roots);
        assert_eq!(config.index.content_roots, plan.content_roots);
        assert_eq!(config.embedding.device, "cuda");
        assert!(!config.federation.enabled);
        let none = Plan { model: ModelChoice::None, ..plan };
        let config = cortex_core::config::Config::from_toml(&none.render(), Path::new("<test>")).unwrap();
        assert_eq!(config.embedding.device, "auto");
    }

    #[test]
    fn answers_are_comma_lists_with_tilde_and_a_dash_means_none() {
        let home = Path::new("/h");
        let default = vec![PathBuf::from("/d")];
        assert_eq!(parse_paths("", home, &default), default);
        assert_eq!(parse_paths("-", home, &default), Vec::<PathBuf>::new());
        assert_eq!(
            parse_paths("~/Docs, /srv/x ,~", home, &default),
            vec![PathBuf::from("/h/Docs"), PathBuf::from("/srv/x"), PathBuf::from("/h")]
        );
        assert_eq!(ModelChoice::parse(" GPU "), Some(ModelChoice::Gpu));
        assert_eq!(ModelChoice::parse("maybe"), None);
    }

    #[test]
    fn an_existing_configuration_is_not_overwritten_by_accident() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let plan = Plan::defaults(dir.path());
        write_config(&path, &plan, false).unwrap();
        assert!(write_config(&path, &plan, false).is_err());
        write_config(&path, &plan, true).unwrap();
    }
}
