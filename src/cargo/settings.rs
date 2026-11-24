//! The lint settings of the command line and of the `metadata.yscope-linter` tables of a
//! workspace's `Cargo.toml` files, and their merging.

use std::fmt;
use std::num::NonZeroUsize;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Deserializer;
use serde::de::SeqAccess;
use serde::de::Visitor;

use crate::cargo::metadata::ManifestPath;
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
    pub skip_tests: Option<SkipTests>,

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
    /// A warning for each deprecated rule that the `rules` or `skip-tests` setting of `table` in
    /// the `Cargo.toml` at `manifest_path` names, and one for each rule without a test-code mode
    /// that the `skip-tests` setting names by its name or code.
    pub fn warnings(&self, table: SettingsTable, manifest_path: &ManifestPath) -> Vec<String> {
        let listed_skip_tests_selectors: &[RuleSelector] = self
            .skip_tests
            .as_ref()
            .map_or(&[], SkipTests::listed_rule_selectors);
        let mut warnings: Vec<String> = [
            ("rules", self.rule_selectors.as_deref().unwrap_or_default()),
            ("skip-tests", listed_skip_tests_selectors),
        ]
        .into_iter()
        .flat_map(|(key, selectors)| {
            RuleSelector::deprecated_rule_ids(selectors)
                .into_iter()
                .map(move |rule_id| {
                    format!(
                        "`{rule_id}` ({}) is deprecated and never runs; remove it from `{key}` in \
                         `{table}` of `{}`",
                        rule_id.code(),
                        manifest_path.path().display()
                    )
                })
        })
        .collect();
        warnings.extend(
            RuleSelector::named_rule_ids_without_test_code_mode(listed_skip_tests_selectors)
                .into_iter()
                .map(|rule_id| {
                    format!(
                        "`{rule_id}` ({}) has no notion of test code, so naming it has no effect; \
                         remove it from `skip-tests` in `{table}` of `{}`",
                        rule_id.code(),
                        manifest_path.path().display()
                    )
                }),
        );
        warnings
    }

    /// # Returns
    ///
    /// The settings to check files with: the set ones, and the defaults for the others: every rule,
    /// a width of 100, and each rule's default for test code.
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
            test_skipping_rules: RuleId::ALL
                .into_iter()
                .filter(|rule_id| match &self.skip_tests {
                    Some(SkipTests::All) => true,
                    Some(SkipTests::Off) => false,
                    Some(SkipTests::Listed { rule_selectors }) => {
                        !rule_id.checks_tests_by_default()
                            || rule_selectors.iter().any(|selector| {
                                selector.rule_ids().any(|listed| listed == *rule_id)
                            })
                    }
                    None => !rule_id.checks_tests_by_default(),
                })
                .collect(),
        }
    }
}

/// A `skip-tests` setting: the rules that skip `#[test]` fns, `#[cfg(test)]` items and out-of-line
/// test modules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipTests {
    /// Every rule skips test code: `true`, or `--skip-tests`.
    All,

    /// Every rule checks test code: `false`, or `--no-skip-tests`.
    Off,

    /// The listed rules skip test code, and the others follow their defaults: a list, or
    /// `--skip-tests=<ID>,...`.
    Listed { rule_selectors: Vec<RuleSelector> },
}

impl SkipTests {
    /// # Returns
    ///
    /// The selectors of the listed rules, which are none unless the setting is a list.
    fn listed_rule_selectors(&self) -> &[RuleSelector] {
        match self {
            Self::All | Self::Off => &[],
            Self::Listed { rule_selectors } => rule_selectors,
        }
    }
}

impl<'de> Deserialize<'de> for SkipTests {
    fn deserialize<DeserializerType: Deserializer<'de>>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error> {
        deserializer.deserialize_any(SkipTestsVisitor)
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
    skip_tests: Option<SkipTests>,

    #[serde(default)]
    exclude: Vec<PathBuf>,

    rules: Option<Vec<RuleSelector>>,
}

/// Parses a `skip-tests` setting, a boolean or an array of rule selectors.
struct SkipTestsVisitor;

impl<'de> Visitor<'de> for SkipTestsVisitor {
    type Value = SkipTests;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a boolean or an array of rule IDs")
    }

    fn visit_bool<ErrorType: serde::de::Error>(self, value: bool) -> Result<SkipTests, ErrorType> {
        Ok(if value {
            SkipTests::All
        } else {
            SkipTests::Off
        })
    }

    fn visit_seq<SeqAccessType: SeqAccess<'de>>(
        self,
        mut seq_access: SeqAccessType,
    ) -> Result<SkipTests, SeqAccessType::Error> {
        let mut rule_selectors = Vec::new();
        while let Some(rule_selector) = seq_access.next_element()? {
            rule_selectors.push(rule_selector);
        }
        Ok(SkipTests::Listed { rule_selectors })
    }
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

    fn invalid_settings_message(json: &str) -> String {
        match parsed(json).expect_err("invalid settings") {
            Error::InvalidSettings { source, .. } => source.to_string(),
            error => panic!("unexpected error: {error:?}"),
        }
    }

    fn enabled_rules(settings: &Settings) -> Vec<RuleId> {
        let rule_set = settings.check_config().rule_set;
        RuleId::ALL
            .into_iter()
            .filter(|rule_id| rule_set.contains(*rule_id))
            .collect()
    }

    fn test_skipping_rules(settings: &Settings) -> Vec<RuleId> {
        let check_config = settings.check_config();
        RuleId::ALL
            .into_iter()
            .filter(|rule_id| check_config.skips_tests(*rule_id))
            .collect()
    }

    fn listed(texts: &[&str]) -> SkipTests {
        SkipTests::Listed {
            rule_selectors: texts
                .iter()
                .map(|text| text.parse().expect("a valid selector"))
                .collect(),
        }
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
                skip_tests: Some(SkipTests::All),
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
            skip_tests: Some(SkipTests::All),
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
                skip_tests: Some(SkipTests::All),
                excludes: vec![PathBuf::from("/ws/w"), PathBuf::from("/ws/a/p")],
            }
        );
        assert_eq!(
            Settings::default().check_config(),
            CheckConfig {
                rule_set: RuleSet::all(),
                width: Width::DEFAULT,
                test_skipping_rules: std::iter::empty().collect(),
            }
        );
    }

    #[test]
    fn skip_tests_is_a_boolean_or_a_list_of_rule_selectors() {
        let doc_rules = [
            RuleId::CommentGreedyWrap,
            RuleId::ErrorsDoc,
            RuleId::TraitParamsDoc,
            RuleId::TypeParamsDoc,
        ];
        let cases: [(&str, SkipTests, &[RuleId]); 5] = [
            ("true", SkipTests::All, &RuleId::ALL),
            ("false", SkipTests::Off, &[]),
            (
                r#"["ORG001", "type-params-doc"]"#,
                listed(&["ORG001", "type-params-doc"]),
                &[RuleId::SymbolOrder, RuleId::TypeParamsDoc],
            ),
            (r#"["DOC"]"#, listed(&["DOC"]), &doc_rules),
            ("[]", listed(&[]), &[]),
        ];
        for (value, expected_skip_tests, expected_rules) in cases {
            let settings =
                parsed(&format!(r#"{{"skip-tests": {value}}}"#)).expect("valid settings");
            assert_eq!(settings.skip_tests, Some(expected_skip_tests), "{value}");
            assert_eq!(test_skipping_rules(&settings), expected_rules, "{value}");
        }
    }

    #[test]
    fn invalid_skip_tests_entries_are_rejected_as_in_rules() {
        let message = invalid_settings_message(r#"{"skip-tests": ["LNT001"]}"#);
        assert_eq!(
            message,
            invalid_settings_message(r#"{"rules": ["LNT001"]}"#)
        );
        let selector_error = "LNT001"
            .parse::<RuleSelector>()
            .expect_err("an invalid selector")
            .to_string();
        assert!(message.contains(&selector_error), "{message}");
        for value in [r#""ORG001""#, "1", "{}"] {
            let message = invalid_settings_message(&format!(r#"{{"skip-tests": {value}}}"#));
            assert!(
                message.ends_with("expected a boolean or an array of rule IDs"),
                "{value}: {message}"
            );
        }
    }

    #[test]
    fn skip_tests_of_a_higher_level_replaces_the_lower_levels() {
        let settings = |skip_tests: Option<SkipTests>| Settings {
            skip_tests,
            ..Settings::default()
        };
        let cases = [
            (Some(SkipTests::All), Some(SkipTests::Off), None, vec![]),
            (
                Some(SkipTests::Off),
                None,
                Some(SkipTests::All),
                RuleId::ALL.to_vec(),
            ),
            (
                Some(listed(&["errors-doc"])),
                Some(listed(&["ORG001"])),
                None,
                vec![RuleId::SymbolOrder],
            ),
            (
                Some(SkipTests::All),
                Some(listed(&["ORG001"])),
                None,
                vec![RuleId::SymbolOrder],
            ),
            (
                Some(listed(&["ORG001"])),
                None,
                None,
                vec![RuleId::SymbolOrder],
            ),
            (
                Some(listed(&["ORG001"])),
                Some(listed(&["DOC003"])),
                Some(listed(&["DOC002"])),
                vec![RuleId::ErrorsDoc],
            ),
            (
                None,
                Some(listed(&["DOC002"])),
                Some(SkipTests::Off),
                vec![],
            ),
            (
                Some(listed(&["ORG001"])),
                None,
                Some(SkipTests::All),
                RuleId::ALL.to_vec(),
            ),
        ];
        for (workspace_skip_tests, package_skip_tests, cli_skip_tests, expected) in cases {
            let description =
                format!("{workspace_skip_tests:?} {package_skip_tests:?} {cli_skip_tests:?}");
            let merged_settings = settings(workspace_skip_tests)
                .overridden_by(settings(package_skip_tests))
                .overridden_by(settings(cli_skip_tests));
            assert_eq!(
                test_skipping_rules(&merged_settings),
                expected,
                "{description}"
            );
        }
    }
}
