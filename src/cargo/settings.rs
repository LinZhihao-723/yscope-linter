//! The lint settings of the command line and of the `metadata.yscope-linter` tables of a
//! workspace's `Cargo.toml` files, and their merging.

use std::num::NonZeroUsize;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;

use crate::config::CheckConfig;
use crate::config::RuleSet;
use crate::config::Width;
use crate::error::Error;
use crate::rules::RuleId;
use crate::rules::RuleSelector;

/// The settings that one source gives. Each setting that is `None` is left to the sources it
/// overrides, and then to the default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub rule_selectors: Option<Vec<RuleSelector>>,
    pub width: Option<Width>,
    pub include_tests: Option<bool>,

    /// The files and directories to skip, which add up across sources instead of overriding.
    pub excludes: Vec<PathBuf>,
}

impl Settings {
    pub const PACKAGE_TABLE: &str = "[package.metadata.yscope-linter]";
    pub const WORKSPACE_TABLE: &str = "[workspace.metadata.yscope-linter]";

    /// Parses the `yscope-linter` table that `table` names, of the `Cargo.toml` at `manifest_path`.
    ///
    /// # Returns
    ///
    /// The table's settings, with its excludes as written, relative to the manifest's directory, or
    /// the empty settings if `settings_table` is `None`, on success.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * [`Error::InvalidSettings`] if the table has an unknown key or a value of the wrong type,
    ///   or isn't a table.
    pub fn from_table(
        settings_table: Option<&serde_json::Value>,
        table: &'static str,
        manifest_path: &Path,
    ) -> Result<Self, Error> {
        let Some(settings_table) = settings_table else {
            return Ok(Self::default());
        };
        let raw_settings =
            RawSettings::deserialize(settings_table).map_err(|source| Error::InvalidSettings {
                manifest_path: manifest_path.to_path_buf(),
                table,
                source,
            })?;
        Ok(Self {
            rule_selectors: raw_settings.rules,
            width: raw_settings.width.map(Width::new),
            include_tests: raw_settings.include_tests,
            excludes: raw_settings.exclude,
        })
    }

    /// # Returns
    ///
    /// The settings with each relative exclude joined to `base_directory`.
    pub fn rebased(mut self, base_directory: &Path) -> Self {
        for exclude in &mut self.excludes {
            *exclude = base_directory.join(&*exclude);
        }
        self
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
            include_tests: overrides.include_tests.or(self.include_tests),
            excludes: self.excludes,
        }
    }

    /// # Returns
    ///
    /// The deprecated rules that the `rules` setting names, each once.
    pub fn deprecated_rule_ids(&self) -> Vec<RuleId> {
        self.rule_selectors
            .as_deref()
            .map_or_else(Vec::new, RuleSelector::deprecated_rule_ids)
    }

    /// # Returns
    ///
    /// The settings to check files with: the set ones, and the defaults of every rule, a width of
    /// 100 and no test code for the others.
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
            include_tests: self.include_tests.unwrap_or(false),
        }
    }
}

/// A `metadata.yscope-linter` table as written.
#[derive(Debug, Deserialize)]
#[serde(
    deny_unknown_fields,
    rename_all = "kebab-case",
    expecting = "a table with the keys `width`, `include-tests`, `exclude` and `rules`"
)]
struct RawSettings {
    width: Option<NonZeroUsize>,
    include_tests: Option<bool>,

    #[serde(default)]
    exclude: Vec<PathBuf>,

    rules: Option<Vec<RuleSelector>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(json: &str) -> Result<Settings, Error> {
        let settings_table: serde_json::Value = serde_json::from_str(json).expect("valid JSON");
        Settings::from_table(
            Some(&settings_table),
            Settings::PACKAGE_TABLE,
            Path::new("/ws/a/Cargo.toml"),
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
    fn table_sets_each_setting_and_rebased_excludes_are_relative_to_the_manifest() {
        let settings = parsed(
            r#"{"width": 80, "include-tests": true, "exclude": ["src/gen", "../b", "/abs"],
                "rules": ["errors-doc"]}"#,
        )
        .expect("valid settings")
        .rebased(Path::new("/ws/a"));
        assert_eq!(
            settings,
            Settings {
                rule_selectors: Some(vec![RuleSelector::Rule(RuleId::ErrorsDoc)]),
                width: Some(width(80)),
                include_tests: Some(true),
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
        assert_eq!(enabled_rules(&Settings::default()), RuleId::ALL);
    }

    #[test]
    fn invalid_table_is_an_error_that_names_the_table_and_the_manifest() {
        for (json, cause) in [
            (r#"{"widht": 80}"#, "unknown field `widht`"),
            (r#"{"width": "80"}"#, "invalid type: string \"80\""),
            (r#"{"width": 0}"#, "invalid value: integer `0`"),
            (
                r#"{"include-tests": 1}"#,
                "invalid type: integer `1`, expected a boolean",
            ),
            (
                r#"{"exclude": "src"}"#,
                "invalid type: string \"src\", expected a sequence",
            ),
            (r#"{"rules": ["doc003"]}"#, "unknown rule `doc003`"),
            (r#"{"rules": ["LNT001"]}"#, "`LNT001` is not a rule"),
            (
                "true",
                "invalid type: boolean `true`, expected a table with the keys",
            ),
        ] {
            let message = parsed(json).expect_err("invalid settings").to_string();
            assert!(
                message.starts_with(
                    "invalid `[package.metadata.yscope-linter]` in `/ws/a/Cargo.toml`: "
                ),
                "{json}: {message}"
            );
            assert!(message.contains(cause), "{json}: {message}");
        }
    }

    #[test]
    fn overrides_win_where_set_and_excludes_add_up() {
        let workspace_settings = Settings {
            rule_selectors: Some(vec![RuleSelector::Rule(RuleId::ErrorsDoc)]),
            width: Some(width(80)),
            include_tests: Some(true),
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
                include_tests: Some(true),
                excludes: vec![PathBuf::from("/ws/w"), PathBuf::from("/ws/a/p")],
            }
        );
        assert_eq!(
            Settings::default().check_config(),
            CheckConfig {
                rule_set: RuleSet::all(),
                width: Width::DEFAULT,
                include_tests: false,
            }
        );
    }
}
