//! The lint settings of the command line and of the `metadata.yscope-linter` tables of a
//! workspace's `Cargo.toml` files, and their merging.

use std::fmt;
use std::num::NonZeroUsize;
use std::path::PathBuf;

use serde::Deserialize;

use crate::cargo::metadata::ManifestPath;
use crate::config::CheckConfig;
use crate::config::RuleSet;
use crate::config::Width;
use crate::error::Error;
use crate::rules::RuleSelector;

/// The settings that one source gives. Each setting that is `None` is left to the sources it
/// overrides, and then to the default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub rule_selectors: Option<Vec<RuleSelector>>,
    pub width: Option<Width>,
    pub skip_tests: Option<bool>,

    /// The files and directories to skip, which add up across sources instead of overriding.
    pub excludes: Vec<PathBuf>,
}

impl Settings {
    /// Parses the `yscope-linter` table that `table` names, of the `Cargo.toml` at `manifest_path`.
    ///
    /// # Returns
    ///
    /// The table's settings, with each exclude joined to the manifest's directory, or the empty
    /// settings if `settings_table` is `None`, on success.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * [`Error::InvalidSettings`] if the table has an unknown key or a value of the wrong type,
    ///   or isn't a table.
    pub fn from_table(
        settings_table: Option<&serde_json::Value>,
        table: SettingsTable,
        manifest_path: &ManifestPath,
    ) -> Result<Self, Error> {
        let Some(settings_table) = settings_table else {
            return Ok(Self::default());
        };
        let raw_settings =
            RawSettings::deserialize(settings_table).map_err(|source| Error::InvalidSettings {
                manifest_path: manifest_path.path().to_path_buf(),
                table,
                source,
            })?;
        Ok(Self {
            rule_selectors: raw_settings.rules,
            width: raw_settings.width.map(Width::new),
            skip_tests: raw_settings.skip_tests,
            excludes: raw_settings
                .exclude
                .into_iter()
                .map(|exclude| manifest_path.directory().join(exclude))
                .collect(),
        })
    }

    /// # Returns
    ///
    /// The settings of `overrides` where they are set, and of `self` elsewhere, with the excludes
    /// of both.
    pub fn overridden_by(mut self, overrides: Self) -> Self {
        self.excludes.extend(overrides.excludes);
        Self {
            rule_selectors: overrides.rule_selectors.or(self.rule_selectors),
            width: overrides.width.or(self.width),
            skip_tests: overrides.skip_tests.or(self.skip_tests),
            excludes: self.excludes,
        }
    }

    /// # Returns
    ///
    /// A warning for each deprecated rule that the `rules` setting of `table` in the `Cargo.toml`
    /// at `manifest_path` names.
    pub fn deprecation_warnings(
        &self,
        table: SettingsTable,
        manifest_path: &ManifestPath,
    ) -> Vec<String> {
        RuleSelector::deprecated_rule_ids(self.rule_selectors.as_deref().unwrap_or_default())
            .into_iter()
            .map(|rule_id| {
                format!(
                    "`{rule_id}` ({}) is deprecated and never runs; remove it from `rules` in \
                     `{table}` of `{}`",
                    rule_id.code(),
                    manifest_path.path().display()
                )
            })
            .collect()
    }

    /// # Returns
    ///
    /// The settings to check files with: the set ones, and the defaults of every rule, a width of
    /// 100 and checked test code for the others.
    pub fn check_config(&self) -> CheckConfig {
        CheckConfig {
            rule_set: self
                .rule_selectors
                .as_ref()
                .map_or_else(RuleSet::all, |selectors| {
                    selectors
                        .iter()
                        .flat_map(|selector| selector.rule_ids())
                        .collect()
                }),
            width: self.width.unwrap_or(Width::DEFAULT),
            skip_tests: self.skip_tests.unwrap_or(false),
        }
    }
}

/// The `metadata.yscope-linter` table of a `Cargo.toml` that settings come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTable {
    Workspace,
    Package,
}

impl fmt::Display for SettingsTable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Workspace => "[workspace.metadata.yscope-linter]",
            Self::Package => "[package.metadata.yscope-linter]",
        })
    }
}

/// A `metadata.yscope-linter` table as written.
#[derive(Debug, Deserialize)]
#[serde(
    deny_unknown_fields,
    rename_all = "kebab-case",
    expecting = "a table with the keys `width`, `skip-tests`, `exclude` and `rules`"
)]
struct RawSettings {
    width: Option<NonZeroUsize>,
    skip_tests: Option<bool>,

    #[serde(default)]
    exclude: Vec<PathBuf>,

    rules: Option<Vec<RuleSelector>>,
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::rules::RuleId;

    fn parsed(json: &str) -> Result<Settings, Error> {
        let settings_table: serde_json::Value = serde_json::from_str(json).expect("valid JSON");
        let manifest_path = ManifestPath::try_from(PathBuf::from("/ws/a/Cargo.toml"))
            .expect("a manifest path with a parent");
        Settings::from_table(
            Some(&settings_table),
            SettingsTable::Package,
            &manifest_path,
        )
    }

    fn width(columns: usize) -> Width {
        Width::new(NonZeroUsize::new(columns).expect("non-zero"))
    }

    fn enabled_rules(settings: &Settings) -> Vec<RuleId> {
        let rule_set = settings.check_config().rule_set;
        RuleId::ALL
            .into_iter()
            .filter(|rule_id| rule_set.contains(*rule_id))
            .collect()
    }

    #[test]
    fn table_sets_each_setting_and_excludes_are_relative_to_the_manifest() {
        let settings = parsed(
            r#"{"width": 80, "skip-tests": true, "exclude": ["src/gen", "../b", "/abs"],
                "rules": ["errors-doc"]}"#,
        )
        .expect("valid settings");
        assert_eq!(
            settings,
            Settings {
                rule_selectors: Some(vec![RuleSelector::Rule(RuleId::ErrorsDoc)]),
                width: Some(width(80)),
                skip_tests: Some(true),
                excludes: ["/ws/a/src/gen", "/ws/a/../b", "/abs"]
                    .map(PathBuf::from)
                    .to_vec(),
            }
        );
        assert_eq!(parsed("{}").expect("valid settings"), Settings::default());
    }

    #[test]
    fn rules_are_selected_by_name_code_and_set() {
        let settings =
            parsed(r#"{"rules": ["comment-greedy-wrap", "DOC003", "ORG"]}"#).expect("valid rules");
        assert_eq!(
            enabled_rules(&settings),
            [
                RuleId::CommentGreedyWrap,
                RuleId::SymbolOrder,
                RuleId::TypeParamsDoc
            ]
        );
    }

    #[test]
    fn invalid_table_is_an_error_that_names_the_table_and_the_manifest() {
        let error = parsed(r#"{"width": "80"}"#).expect_err("invalid settings");
        let Error::InvalidSettings {
            manifest_path,
            table,
            source,
        } = &error
        else {
            panic!("unexpected error: {error:?}");
        };
        assert_eq!(manifest_path, Path::new("/ws/a/Cargo.toml"));
        assert_eq!(*table, SettingsTable::Package);
        assert!(
            source.to_string().contains("invalid type: string \"80\""),
            "{source}"
        );
    }

    #[test]
    fn overrides_win_where_set_and_excludes_add_up() {
        let workspace_settings = Settings {
            rule_selectors: Some(vec![RuleSelector::Rule(RuleId::ErrorsDoc)]),
            width: Some(width(80)),
            skip_tests: Some(true),
            excludes: vec![PathBuf::from("/ws/w")],
        };
        let package_settings = Settings {
            width: Some(width(90)),
            excludes: vec![PathBuf::from("/ws/a/p")],
            ..Settings::default()
        };
        let cli_settings = Settings {
            rule_selectors: Some(vec![RuleSelector::Rule(RuleId::SymbolOrder)]),
            ..Settings::default()
        };
        let settings = workspace_settings
            .overridden_by(package_settings)
            .overridden_by(cli_settings);
        assert_eq!(
            settings,
            Settings {
                rule_selectors: Some(vec![RuleSelector::Rule(RuleId::SymbolOrder)]),
                width: Some(width(90)),
                skip_tests: Some(true),
                excludes: vec![PathBuf::from("/ws/w"), PathBuf::from("/ws/a/p")],
            }
        );
        assert_eq!(
            Settings::default().check_config(),
            CheckConfig {
                rule_set: RuleSet::all(),
                width: Width::DEFAULT,
                skip_tests: false,
            }
        );
    }

    #[test]
    fn skip_tests_is_overridden_in_both_directions() {
        let settings = |skip_tests: Option<bool>| Settings {
            skip_tests,
            ..Settings::default()
        };
        for (workspace_skip_tests, package_skip_tests, cli_skip_tests, expected) in [
            (Some(true), Some(false), None, false),
            (Some(false), Some(true), None, true),
            (None, Some(true), Some(false), false),
            (Some(false), None, Some(true), true),
        ] {
            let check_config = settings(workspace_skip_tests)
                .overridden_by(settings(package_skip_tests))
                .overridden_by(settings(cli_skip_tests))
                .check_config();
            assert_eq!(
                check_config.skip_tests, expected,
                "{workspace_skip_tests:?} {package_skip_tests:?} {cli_skip_tests:?}"
            );
        }
    }
}
