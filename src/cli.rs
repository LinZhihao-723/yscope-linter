//! The command-line interfaces of `yscope-linter` and `cargo yscope-linter`, and their conversion
//! into a [`LintConfig`] and a [`CargoConfig`].

use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::str::FromStr;

use clap::Parser;

use crate::cargo::Settings;
use crate::cargo::SkipTests;
use crate::config::CargoConfig;
use crate::config::DiffInput;
use crate::config::LintConfig;
use crate::config::OutputFormat;
use crate::config::PackageSelection;
use crate::config::Width;
use crate::error::Error;
use crate::rules::RuleSelector;
use crate::source::TargetSpec;

/// Lints the y-scope Rust comment and docstring rules that rustfmt and clippy can't check.
#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Cli {
    /// A file, a directory, or `FILE:RANGES` (e.g. `src/a.rs:10-20,30,40-45`).
    #[arg(value_name = "TARGET", required_unless_present = "diff")]
    targets: Vec<String>,

    #[command(flatten)]
    lint_options: LintOptions,
}

impl Cli {
    /// # Returns
    ///
    /// A warning for each deprecated rule that `--rule` or `--skip-tests` names, since it never
    /// runs, one for each rule without a test-code mode that `--skip-tests` names by its name or
    /// code, and one for `--include-tests`, since neither has an effect.
    pub fn warnings(&self) -> Vec<String> {
        self.lint_options.warnings()
    }

    /// Validates the parsed arguments.
    ///
    /// # Returns
    ///
    /// The run's configuration on success.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * Forwards [`TargetSpec::from_str`]'s return values on failure.
    pub fn into_config(self) -> Result<LintConfig, Error> {
        let targets = self
            .targets
            .iter()
            .map(|argument| TargetSpec::from_str(argument))
            .collect::<Result<Vec<_>, _>>()?;
        let LintOptions {
            settings_options,
            diff,
            format,
        } = self.lint_options;
        let settings = Settings::from(settings_options);
        Ok(LintConfig {
            check_config: settings.check_config(),
            format,
            targets,
            diff_input: diff.map(DiffInput::from_argument),
            excludes: settings.excludes,
        })
    }
}

/// Lints a cargo workspace when cargo runs it as `cargo yscope-linter`.
#[derive(Debug, Parser)]
#[command(name = "cargo", bin_name = "cargo")]
pub enum CargoCli {
    /// Lints the packages of a cargo workspace for the y-scope Rust comment and docstring rules
    /// that rustfmt and clippy can't check.
    #[command(name = "yscope-linter", version)]
    YscopeLinter(CargoArgs),
}

/// The arguments of `cargo yscope-linter`.
#[derive(Debug, clap::Args)]
pub struct CargoArgs {
    /// Path to the Cargo.toml of the workspace or package to lint.
    #[arg(long, value_name = "PATH", help_heading = "Package Selection")]
    manifest_path: Option<PathBuf>,

    /// Lint only this workspace member. Repeatable.
    #[arg(
        short,
        long = "package",
        value_name = "NAME",
        help_heading = "Package Selection"
    )]
    packages: Vec<String>,

    /// Lint every workspace member.
    #[arg(long, help_heading = "Package Selection")]
    workspace: bool,

    /// Skip this workspace member with --workspace. Repeatable.
    #[arg(
        long = "exclude-package",
        value_name = "NAME",
        requires = "workspace",
        help_heading = "Package Selection"
    )]
    excluded_packages: Vec<String>,

    #[command(flatten)]
    lint_options: LintOptions,
}

impl CargoArgs {
    /// # Returns
    ///
    /// A warning for each deprecated rule that `--rule` or `--skip-tests` names, since it never
    /// runs, one for each rule without a test-code mode that `--skip-tests` names by its name or
    /// code, and one for `--include-tests`, since neither has an effect.
    pub fn warnings(&self) -> Vec<String> {
        self.lint_options.warnings()
    }

    /// Converts the parsed arguments. `--workspace` takes precedence over `-p`, as it does in
    /// cargo.
    ///
    /// # Returns
    ///
    /// The run's configuration.
    pub fn into_config(self) -> CargoConfig {
        let package_selection = if self.workspace {
            PackageSelection::Workspace {
                excluded_names: self.excluded_packages,
            }
        } else if self.packages.is_empty() {
            PackageSelection::Default
        } else {
            PackageSelection::Packages {
                names: self.packages,
            }
        };
        let LintOptions {
            settings_options,
            diff,
            format,
        } = self.lint_options;
        CargoConfig {
            manifest_path: self.manifest_path,
            package_selection,
            overrides: Settings::from(settings_options),
            format,
            diff_input: diff.map(DiffInput::from_argument),
        }
    }
}

/// The options that both interfaces share.
#[derive(Debug, clap::Args)]
struct LintOptions {
    #[command(flatten)]
    settings_options: SettingsOptions,

    /// Lint the changed lines of a unified diff (`git diff -U0`); `-` reads stdin.
    #[arg(long, value_name = "FILE")]
    diff: Option<PathBuf>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    format: OutputFormat,
}

impl LintOptions {
    /// # Returns
    ///
    /// A warning for each deprecated rule that `--rule` or `--skip-tests` names, since it never
    /// runs, one for each rule without a test-code mode that `--skip-tests` names by its name or
    /// code, and one for `--include-tests`, since neither has an effect.
    fn warnings(&self) -> Vec<String> {
        let skip_tests_selectors = self
            .settings_options
            .skip_tests
            .as_deref()
            .unwrap_or_default();
        let mut warnings: Vec<String> = [
            ("--rule", self.settings_options.rules.as_slice()),
            ("--skip-tests", skip_tests_selectors),
        ]
        .into_iter()
        .flat_map(|(flag, selectors)| {
            RuleSelector::deprecated_rule_ids(selectors)
                .into_iter()
                .map(move |rule_id| {
                    format!(
                        "`{rule_id}` ({}) is deprecated and never runs; remove it from `{flag}`",
                        rule_id.code()
                    )
                })
        })
        .collect();
        warnings.extend(
            RuleSelector::named_rule_ids_without_test_code_mode(skip_tests_selectors)
                .into_iter()
                .map(|rule_id| {
                    format!(
                        "`{rule_id}` ({}) has no notion of test code, so naming it has no effect; \
                         remove it from `--skip-tests`",
                        rule_id.code()
                    )
                }),
        );
        if self.settings_options.include_tests {
            warnings.push(
                "`--include-tests` is deprecated and has no effect, since test code is checked by \
                 default; remove it"
                    .to_owned(),
            );
        }
        warnings
    }
}

/// The options that set the command line's [`Settings`].
#[derive(Debug, clap::Args)]
struct SettingsOptions {
    /// Enable only this rule: a name, a code (DOC003) or a set (DOC). Repeatable. Default: all
    /// rules.
    #[arg(long = "rule", value_name = "ID")]
    rules: Vec<RuleSelector>,

    /// Fill width for comment-greedy-wrap. Default: 100.
    #[arg(long, value_name = "N")]
    width: Option<NonZeroUsize>,

    /// Skip #[test] fns and #[cfg(test)] items in every rule, or, as --skip-tests=ID,..., only in
    /// the listed rules: names, codes (ORG001) or sets (DOC). Default: each rule's own.
    #[arg(
        long,
        value_name = "ID",
        num_args = 0..=1,
        // Without `=`, `--skip-tests` stays bare, so a following `TARGET` isn't read as its list.
        require_equals = true,
        value_delimiter = ',',
        overrides_with_all = ["skip_tests", "no_skip_tests"]
    )]
    skip_tests: Option<Vec<RuleSelector>>,

    /// Check test code in every rule, overriding a skip-tests setting.
    #[arg(long)]
    no_skip_tests: bool,

    /// Deprecated, with no effect: test code is checked by default.
    #[arg(long, hide = true)]
    include_tests: bool,

    /// Skip this file or directory (path-prefix match by whole components). Repeatable.
    #[arg(long = "exclude", value_name = "PATH")]
    excludes: Vec<PathBuf>,
}

impl From<SettingsOptions> for Settings {
    fn from(settings_options: SettingsOptions) -> Self {
        let SettingsOptions {
            rules,
            width,
            skip_tests,
            no_skip_tests,
            include_tests: _,
            excludes,
        } = settings_options;
        Self {
            rule_selectors: (!rules.is_empty()).then_some(rules),
            width: width.map(Width::new),
            skip_tests: match skip_tests {
                Some(rule_selectors) if rule_selectors.is_empty() => Some(SkipTests::All),
                Some(rule_selectors) => Some(SkipTests::Listed { rule_selectors }),
                None => no_skip_tests.then_some(SkipTests::Off),
            },
            excludes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::RuleId;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("yscope-linter").chain(args.iter().copied()))
            .expect("valid arguments")
    }

    fn enabled_rules(cli: Cli) -> Vec<RuleId> {
        let rule_set = cli
            .into_config()
            .expect("valid config")
            .check_config
            .rule_set;
        RuleId::ALL
            .into_iter()
            .filter(|rule_id| rule_set.contains(*rule_id))
            .collect()
    }

    #[test]
    fn rule_flag_selects_rules_by_name_code_and_set() {
        assert_eq!(
            enabled_rules(parse(&["--rule", "errors-doc", "--rule", "ORG", "a.rs"])),
            [RuleId::ErrorsDoc, RuleId::SymbolOrder]
        );
        assert_eq!(
            enabled_rules(parse(&[
                "--rule",
                "DOC003",
                "--rule",
                "type-params-doc",
                "a.rs"
            ])),
            [RuleId::TypeParamsDoc]
        );
        assert_eq!(enabled_rules(parse(&["a.rs"])), RuleId::ALL);
    }

    #[test]
    fn settings_options_set_only_the_settings_they_are_given() {
        let settings = |args: &[&str]| Settings::from(parse(args).lint_options.settings_options);
        assert_eq!(settings(&["a.rs"]), Settings::default());
        assert_eq!(
            settings(&[
                "--rule",
                "errors-doc",
                "--width",
                "80",
                "--skip-tests",
                "--exclude",
                "b",
                "--exclude",
                "c",
                "a.rs"
            ]),
            Settings {
                rule_selectors: Some(vec![RuleSelector::Rule(RuleId::ErrorsDoc)]),
                width: Some(Width::new(NonZeroUsize::new(80).expect("non-zero"))),
                skip_tests: Some(SkipTests::All),
                excludes: vec![PathBuf::from("b"), PathBuf::from("c")],
            }
        );
    }

    #[test]
    fn last_skip_tests_flag_wins_and_include_tests_has_no_effect() {
        let skip_tests =
            |args: &[&str]| Settings::from(parse(args).lint_options.settings_options).skip_tests;
        let listed = |texts: &[&str]| SkipTests::Listed {
            rule_selectors: texts
                .iter()
                .map(|text| text.parse().expect("a valid selector"))
                .collect(),
        };
        let cases: [(&[&str], Option<SkipTests>); 12] = [
            (&["--skip-tests"], Some(SkipTests::All)),
            (&["--no-skip-tests"], Some(SkipTests::Off)),
            (
                &["--skip-tests=ORG001,errors-doc,DOC"],
                Some(listed(&["ORG001", "errors-doc", "DOC"])),
            ),
            (&["--no-skip-tests", "--skip-tests"], Some(SkipTests::All)),
            (&["--skip-tests", "--no-skip-tests"], Some(SkipTests::Off)),
            (
                &["--no-skip-tests", "--skip-tests=ORG001"],
                Some(listed(&["ORG001"])),
            ),
            (
                &["--skip-tests=ORG001", "--no-skip-tests"],
                Some(SkipTests::Off),
            ),
            (
                &["--skip-tests", "--skip-tests=ORG001"],
                Some(listed(&["ORG001"])),
            ),
            (
                &["--skip-tests=ORG001", "--skip-tests"],
                Some(SkipTests::All),
            ),
            (
                &["--skip-tests=ORG001", "--skip-tests=DOC002"],
                Some(listed(&["DOC002"])),
            ),
            (&["--include-tests"], None),
            (
                &["--skip-tests=ORG001", "--include-tests"],
                Some(listed(&["ORG001"])),
            ),
        ];
        for (flags, expected) in cases {
            let args: Vec<&str> = flags.iter().copied().chain(["a.rs"]).collect();
            assert_eq!(skip_tests(&args), expected, "{flags:?}");
        }
    }

    #[test]
    fn skip_tests_flag_rejects_invalid_and_empty_entries() {
        for (value, invalid_entry) in [("ORG001,doc003", "doc003"), ("", ""), ("ORG001,", "")] {
            let error =
                Cli::try_parse_from(["yscope-linter", &format!("--skip-tests={value}"), "a.rs"])
                    .expect_err("an invalid `--skip-tests` value");
            assert_eq!(
                error.kind(),
                clap::error::ErrorKind::ValueValidation,
                "{value:?}"
            );
            let selector_error = invalid_entry
                .parse::<RuleSelector>()
                .expect_err("an invalid selector")
                .to_string();
            assert!(
                error.to_string().contains(&selector_error),
                "{value:?}: {error}"
            );
        }
    }

    #[test]
    fn rule_flag_rejects_unknown_values_and_meta_codes() {
        for value in ["nope", "doc003", "LNT001", "parse-error", "LNT"] {
            let error = Cli::try_parse_from(["yscope-linter", "--rule", value, "a.rs"])
                .expect_err("an invalid `--rule` value");
            assert_eq!(
                error.kind(),
                clap::error::ErrorKind::ValueValidation,
                "{value:?}"
            );
            let selector_error = value
                .parse::<RuleSelector>()
                .expect_err("an invalid selector")
                .to_string();
            assert!(
                error.to_string().contains(&selector_error),
                "{value:?}: {error}"
            );
        }
    }
}
