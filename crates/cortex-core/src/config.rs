//! Configuration.
//!
//! Layering is `built-in defaults → user config file → environment overrides`. A missing
//! configuration file is valid and yields a working system — rule R5 means those defaults must
//! also be the *safe* ones: nothing is indexed for content, nothing talks to the network.

use crate::error::{Error, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub index: IndexConfig,
    pub search: SearchConfig,
    pub resources: ResourceConfig,
    pub agent: AgentConfig,
    pub federation: FederationConfig,
    pub log: LogConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            index: IndexConfig::default(),
            search: SearchConfig::default(),
            resources: ResourceConfig::default(),
            agent: AgentConfig::default(),
            federation: FederationConfig::default(),
            log: LogConfig::default(),
        }
    }
}

/// What gets seen, and how deeply.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IndexConfig {
    /// Roots that are watched and indexed as metadata (path, stat — no content).
    pub metadata_roots: Vec<PathBuf>,

    /// Roots whose contents may be extracted, chunked and embedded.
    ///
    /// Rule R3: this is opt-in and empty by default. Metadata indexing may span a whole home
    /// directory; reading file contents may not, until the user says which parts.
    pub content_roots: Vec<PathBuf>,

    /// Extra deny-list patterns, added to the built-in minimum. The built-ins cannot be removed.
    pub extra_deny: Vec<String>,

    /// Coalescing window for filesystem events. Editors write a file three to five times per
    /// save; without this the ingest queue does five times the work for one edit.
    pub debounce_ms: u64,

    /// How often the reconcile scan runs. Watcher events *are* dropped under load, and an
    /// index that silently drifts is worse than no index.
    pub reconcile_interval_secs: u64,

    /// Respect `.gitignore` inside git repositories, in addition to `.brainignore`.
    pub respect_gitignore: bool,

    /// Maximum directories watched at once.
    ///
    /// The kernel has its own limit (`fs.inotify.max_user_watches`, commonly 65 536 and shared
    /// across every application on the machine). Staying well below it is deliberate: a file
    /// manager that consumes the whole system's watch budget breaks the user's editor and
    /// their sync client, and they will rightly blame us. Exceeding this degrades visibly in
    /// `cortex status` and falls back to reconciliation for the unwatched part.
    pub watch_budget: usize,

    /// Maximum number of spatial zones.
    ///
    /// Zones exist for orientation, and orientation does not survive being given a thousand
    /// categories — a hundred colours is the same as none. When more candidates are found than
    /// this, the shallowest are kept, because those are the ones a person actually names when
    /// describing where something is. Assignment also costs one pass over the index per zone,
    /// so an unbounded count makes indexing quadratic in a way nobody would attribute to
    /// colour coding.
    pub zone_budget: usize,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            metadata_roots: Vec::new(),
            content_roots: Vec::new(),
            extra_deny: Vec::new(),
            debounce_ms: 500,
            reconcile_interval_secs: 3600,
            respect_gitignore: true,
            watch_budget: 20_000,
            zone_budget: 64,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SearchConfig {
    /// Results returned by default.
    pub default_limit: usize,
    /// Candidates each retrieval arm contributes to fusion before ranking.
    pub candidates_per_arm: usize,
    /// The `k` constant in reciprocal rank fusion. 60 is the value from the original paper and
    /// there is no reason to deviate without measurement.
    pub rrf_k: f64,
    /// How strongly salience is allowed to move a result. A boost that can override relevance
    /// makes the search feel arbitrary, so this is bounded.
    pub max_salience_boost: f64,
    /// Run the cross-encoder reranker over the top results, asynchronously.
    pub rerank: bool,
    pub rerank_top_n: usize,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            default_limit: 50,
            candidates_per_arm: 100,
            rrf_k: 60.0,
            max_salience_boost: 1.5,
            rerank: true,
            rerank_top_n: 30,
        }
    }
}

/// Hard ceilings on what the daemon may consume.
///
/// A first index run that makes the machine feel slow is how this project loses its user in
/// week one, so these are enforced rather than advisory.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResourceConfig {
    /// Fraction of total CPU the ingest pipeline may use while the user is active.
    pub cpu_share_active: f64,
    /// Fraction it may use while the machine is idle.
    pub cpu_share_idle: f64,
    /// Idle threshold, in seconds without user input.
    pub idle_after_secs: u64,
    /// Soft memory ceiling for the daemon, in megabytes. Exceeding it sheds caches first.
    pub memory_budget_mb: u64,
    /// `nice` level for background ingest work.
    pub nice_level: i32,
}

impl Default for ResourceConfig {
    fn default() -> Self {
        Self {
            cpu_share_active: 0.15,
            cpu_share_idle: 0.80,
            idle_after_secs: 120,
            memory_budget_mb: 250,
            nice_level: 10,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentConfig {
    /// Master switch for the Agent Surface. Part of the kill switch (R7).
    pub enabled: bool,
    /// Paths agents may mutate. An allow-list, never a deny-list — an agent with unchecked
    /// `fs.move` over an indexed home directory can do more damage in 200 ms than a backup undoes.
    pub write_allowlist: Vec<PathBuf>,
    /// Require a confirmed dry-run plan before any filesystem mutation is applied.
    pub require_dry_run: bool,
    /// Keep this many undoable operations.
    pub undo_journal_depth: usize,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            write_allowlist: Vec::new(),
            require_dry_run: true,
            undo_journal_depth: 1000,
        }
    }
}

/// Federation with a peer. Off by default — rule R5.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FederationConfig {
    pub enabled: bool,
    /// Maximum events sent per batch, so that days of accumulated events do not flood a peer
    /// on reconnect.
    pub batch_size: usize,
    /// Minimum gap between batches, in milliseconds.
    pub batch_interval_ms: u64,
    /// Whether chunk text may leave the machine. Off means only metadata, summaries and vectors.
    pub send_chunk_text: bool,
}

impl Default for FederationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            batch_size: 500,
            batch_interval_ms: 1000,
            send_chunk_text: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LogConfig {
    /// `tracing` filter directive, e.g. `info` or `cortexd=debug,cortex_store=trace`.
    pub filter: String,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            filter: "info".to_string(),
        }
    }
}

impl Config {
    /// Load from a file, falling back to defaults when it does not exist.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::from_toml(&text, path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!(path = %path.display(), "no config file, using defaults");
                Ok(Self::default())
            }
            Err(e) => Err(Error::io(path, e)),
        }
    }

    pub fn from_toml(text: &str, path: &Path) -> Result<Self> {
        let config: Config =
            toml::from_str(text).map_err(|e| Error::config(path, e.to_string()))?;
        config.validate(path)?;
        Ok(config)
    }

    /// Reject configurations that would silently misbehave.
    ///
    /// Failing loudly at startup is much cheaper than a user wondering for a week why their
    /// search returns nothing.
    pub fn validate(&self, path: &Path) -> Result<()> {
        let bad = |msg: String| Err(Error::config(path, msg));

        if !(0.0..=1.0).contains(&self.resources.cpu_share_active) {
            return bad("resources.cpu_share_active must be between 0.0 and 1.0".into());
        }
        if !(0.0..=1.0).contains(&self.resources.cpu_share_idle) {
            return bad("resources.cpu_share_idle must be between 0.0 and 1.0".into());
        }
        if self.resources.cpu_share_active > self.resources.cpu_share_idle {
            return bad(
                "resources.cpu_share_active must not exceed cpu_share_idle — background work \
                 may not be more aggressive while the user is present"
                    .into(),
            );
        }
        if self.search.default_limit == 0 {
            return bad("search.default_limit must be greater than zero".into());
        }
        if self.search.candidates_per_arm < self.search.default_limit {
            return bad(
                "search.candidates_per_arm must be at least search.default_limit, otherwise \
                 fusion cannot fill the requested result count"
                    .into(),
            );
        }
        if self.search.rrf_k <= 0.0 {
            return bad("search.rrf_k must be greater than zero".into());
        }
        if self.search.max_salience_boost < 1.0 {
            return bad(
                "search.max_salience_boost must be at least 1.0 — a boost below 1.0 would \
                 penalise salient results"
                    .into(),
            );
        }
        if self.index.debounce_ms == 0 {
            return bad(
                "index.debounce_ms must be greater than zero — without coalescing, one save \
                 produces several ingest passes"
                    .into(),
            );
        }
        if self.index.watch_budget == 0 && !self.index.metadata_roots.is_empty() {
            return bad(
                "index.watch_budget must be greater than zero when metadata_roots are \
                 configured, otherwise nothing is ever watched"
                    .into(),
            );
        }
        if self.index.zone_budget == 0 {
            return bad(
                "index.zone_budget must be greater than zero; set index.metadata_roots to \
                 nothing instead if you want no zones at all"
                    .into(),
            );
        }
        if self.index.reconcile_interval_secs == 0 {
            return bad(
                "index.reconcile_interval_secs must be greater than zero — reconciliation is \
                 the correctness backstop for lost watcher events, not an optional extra"
                    .into(),
            );
        }

        // Content extraction outside a metadata root is not wrong, but a content root that is
        // not also watched will go stale, which looks like a bug to the user.
        for root in &self.index.content_roots {
            let watched = self
                .index
                .metadata_roots
                .iter()
                .any(|m| root.starts_with(m) || m.starts_with(root));
            if !watched {
                return bad(format!(
                    "index.content_roots contains {} which is not inside any metadata_root; \
                     it would be extracted once and then never updated",
                    root.display()
                ));
            }
        }

        Ok(())
    }

    /// Serialise back to TOML, for `cortex config show` and for writing a starter file.
    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("Config is always serialisable")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn p() -> PathBuf {
        PathBuf::from("<test>")
    }

    #[test]
    fn defaults_are_valid() {
        Config::default().validate(&p()).unwrap();
    }

    #[test]
    fn defaults_index_no_content_and_do_not_federate() {
        // Rule R3 and R5: a fresh install reads no file contents and talks to nobody.
        let c = Config::default();
        assert!(c.index.content_roots.is_empty());
        assert!(!c.federation.enabled);
    }

    #[test]
    fn a_missing_file_yields_defaults_rather_than_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = Config::load(&tmp.path().join("does-not-exist.toml")).unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn config_round_trips_through_toml() {
        let mut original = Config::default();
        original.index.metadata_roots = vec![PathBuf::from("/a")];
        original.index.content_roots = vec![PathBuf::from("/a/b")];
        original.log.filter = "debug".into();

        let text = original.to_toml();
        let parsed = Config::from_toml(&text, &p()).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn unknown_keys_are_rejected() {
        // A silently ignored typo in a config file is a support burden forever.
        let err = Config::from_toml("[searhc]\ndefault_limit = 10\n", &p()).unwrap_err();
        assert!(matches!(err, Error::Config { .. }));
    }

    #[test]
    fn partial_config_fills_in_defaults() {
        let cfg = Config::from_toml("[search]\ndefault_limit = 10\n", &p()).unwrap();
        assert_eq!(cfg.search.default_limit, 10);
        assert_eq!(cfg.search.rrf_k, SearchConfig::default().rrf_k);
    }

    #[test]
    fn background_work_may_not_be_more_aggressive_while_the_user_is_active() {
        let text = "[resources]\ncpu_share_active = 0.9\ncpu_share_idle = 0.5\n";
        assert!(Config::from_toml(text, &p()).is_err());
    }

    #[test]
    fn candidates_must_be_able_to_fill_the_result_list() {
        let text = "[search]\ndefault_limit = 200\ncandidates_per_arm = 50\n";
        assert!(Config::from_toml(text, &p()).is_err());
    }

    #[test]
    fn a_content_root_outside_every_metadata_root_is_rejected() {
        let text = r#"
            [index]
            metadata_roots = ["/home/user/documents"]
            content_roots = ["/home/user/projects"]
        "#;
        let err = Config::from_toml(text, &p()).unwrap_err();
        let Error::Config { message, .. } = err else {
            panic!("expected a config error");
        };
        assert!(message.contains("never updated"), "unhelpful message: {message}");
    }

    #[test]
    fn a_content_root_inside_a_metadata_root_is_accepted() {
        let text = r#"
            [index]
            metadata_roots = ["/home/user"]
            content_roots = ["/home/user/projects"]
        "#;
        Config::from_toml(text, &p()).unwrap();
    }

    #[test]
    fn zero_debounce_is_rejected() {
        assert!(Config::from_toml("[index]\ndebounce_ms = 0\n", &p()).is_err());
    }

    #[test]
    fn salience_may_not_penalise() {
        assert!(Config::from_toml("[search]\nmax_salience_boost = 0.5\n", &p()).is_err());
    }
}
