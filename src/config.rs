//! The validated settings of one lint run.

use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::num::NonZeroUsize;
use std::path::PathBuf;

use crate::cargo::Settings;
use crate::error::Error;
use crate::rules::RuleId;
use crate::source::TargetSpec;

/// Everything a `yscope-linter` run needs, validated from the command line.
#[derive(Debug, Clone)]
pub struct LintConfig {
    pub check_config: CheckConfig,
    pub format: OutputFormat,
    pub targets: Vec<TargetSpec>,
    pub diff_input: Option<DiffInput>,
    pub excludes: Vec<PathBuf>,
}

/// Everything a `cargo yscope-linter` run needs, validated from the command line.
#[derive(Debug)]
pub struct CargoConfig {
    pub manifest_path: Option<PathBuf>,
    pub package_selection: PackageSelection,

    /// The command-line settings, which override the `Cargo.toml` settings. Their excludes are
    /// relative to the current directory.
    pub overrides: Settings,

    pub format: OutputFormat,
    pub diff_input: Option<DiffInput>,
}

/// The settings that the rules check a file with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckConfig {
    pub rule_set: RuleSet,
    pub width: Width,

    /// The rules that skip `#[test]` fns, `#[cfg(test)]` items and out-of-line test modules.
    pub test_skipping_rules: RuleSet,
}

impl CheckConfig {
    /// # Returns
    ///
    /// Whether the rule `rule_id` skips test code.
    pub fn skips_tests(&self, rule_id: RuleId) -> bool {
        self.test_skipping_rules.contains(rule_id)
    }
}

/// The workspace members a `cargo yscope-linter` run lints, selected as cargo selects them.
#[derive(Debug)]
pub enum PackageSelection {
    /// The default members of the selected manifest's workspace, as `cargo metadata` reports them.
    Default,

    /// The members named by `-p`.
    Packages { names: Vec<String> },

    /// Every member except the ones named by `--exclude-package`.
    Workspace { excluded_names: Vec<String> },
}

/// A set of rules, such as the rules enabled in a run. It never holds a deprecated rule, since a
/// deprecated rule never runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleSet {
    rule_ids: HashSet<RuleId>,
}

impl RuleSet {
    /// # Returns
    ///
    /// The set of every rule that isn't deprecated.
    pub fn all() -> Self {
        RuleId::ALL.into_iter().collect()
    }

    /// # Returns
    ///
    /// Whether `rule_id` is enabled.
    pub fn contains(&self, rule_id: RuleId) -> bool {
        self.rule_ids.contains(&rule_id)
    }
}

impl FromIterator<RuleId> for RuleSet {
    fn from_iter<IteratorType: IntoIterator<Item = RuleId>>(rule_ids: IteratorType) -> Self {
        Self {
            rule_ids: rule_ids
                .into_iter()
                .filter(|rule_id| !rule_id.is_deprecated())
                .collect(),
        }
    }
}

/// The column limit that `comment-greedy-wrap` fills comment lines to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Width {
    columns: NonZeroUsize,
}

impl Width {
    pub const DEFAULT: Self = Self::new(NonZeroUsize::new(100).expect("100 is non-zero"));

    /// Factory function.
    ///
    /// # Returns
    ///
    /// The width of `columns` columns.
    pub const fn new(columns: NonZeroUsize) -> Self {
        Self { columns }
    }

    pub const fn columns(self) -> usize {
        self.columns.get()
    }
}

/// How diagnostics are printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    /// `<path>:<line>:<column>: <code> <name>: <message>`.
    Text,

    /// GitHub Actions `::error` workflow commands.
    Github,
}

/// Where the unified diff of `--diff` is read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffInput {
    Stdin,
    File { path: PathBuf },
}

impl DiffInput {
    /// Factory function.
    ///
    /// # Returns
    ///
    /// The input that the `--diff` argument `path` names: the standard input for `-`, and the file
    /// otherwise.
    pub fn from_argument(path: PathBuf) -> Self {
        if path.as_os_str() == "-" {
            Self::Stdin
        } else {
            Self::File { path }
        }
    }

    /// Reads the whole diff text.
    ///
    /// # Returns
    ///
    /// The diff text, with each invalid UTF-8 sequence replaced by U+FFFD, on success.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * [`Error::ReadStdin`] if the standard input can't be read.
    /// * [`Error::Io`] if the diff file can't be read.
    pub fn read(&self) -> Result<String, Error> {
        let diff_bytes = match self {
            Self::Stdin => {
                let mut diff_bytes = Vec::new();
                std::io::stdin()
                    .read_to_end(&mut diff_bytes)
                    .map_err(|source| Error::ReadStdin { source })?;
                diff_bytes
            }
            Self::File { path } => fs::read(path).map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?,
        };
        Ok(String::from_utf8(diff_bytes)
            .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_set_from_rule_ids_contains_only_those_rules() {
        let rule_set: RuleSet = std::iter::once(RuleId::ErrorsDoc).collect();
        assert!(rule_set.contains(RuleId::ErrorsDoc));
        assert!(!rule_set.contains(RuleId::CommentGreedyWrap));
        assert!(
            RuleId::ALL
                .into_iter()
                .all(|rule_id| RuleSet::all().contains(rule_id))
        );
    }
}
