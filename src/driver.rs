//! The lint run: discovering files, running each enabled rule on them, applying suppressions and
//! line filters, and sorting the result.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use crate::config::CheckConfig;
use crate::config::LintConfig;
use crate::config::RuleSet;
use crate::config::Width;
use crate::diagnostic::Diagnostic;
use crate::diagnostic::FileDiagnostic;
use crate::diff;
use crate::error::Error;
use crate::file_context::FileContext;
use crate::line_ranges::LineRanges;
use crate::rules::Rule;
use crate::rules::RuleId;
use crate::source;
use crate::source::LineFilter;
use crate::source::SourceFile;
use crate::syntax::test_code::TestModuleCache;

/// The outcome of a lint run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The diagnostics of every file, in output order and without duplicates.
    pub diagnostics: Vec<FileDiagnostic>,

    /// The number of files checked.
    pub num_files_checked: usize,

    /// The number of files skipped because they are marked `@generated`.
    pub num_generated_files: usize,
}

/// Files that the rules check with the same settings.
#[derive(Debug)]
pub struct FileGroup {
    pub check_config: CheckConfig,
    pub source_files: Vec<SourceFile>,
}

/// Runs the lint over every file the configuration of a `yscope-linter` run selects.
///
/// # Returns
///
/// The report of [`check`] on success.
///
/// # Errors
///
/// Returns an error if:
///
/// * Forwards [`crate::config::DiffInput::read`]'s return values on failure.
/// * Forwards [`diff::parse`]'s return values on failure.
/// * Forwards [`source::discover`]'s return values on failure.
/// * Forwards [`check`]'s return values on failure.
pub fn run(config: LintConfig) -> Result<Report, Error> {
    let diff_ranges: BTreeMap<PathBuf, LineRanges> = match &config.diff_input {
        Some(diff_input) => diff::parse(&diff_input.read()?)?,
        None => BTreeMap::new(),
    };
    let source_files = source::discover(config.targets, diff_ranges, &config.excludes)?;
    check(vec![FileGroup {
        check_config: config.check_config,
        source_files,
    }])
}

/// Checks the files of every group with the group's settings, skipping the files marked
/// `@generated` in their first five lines.
///
/// # Returns
///
/// The sorted diagnostics of every file on success.
///
/// # Errors
///
/// Returns an error if:
///
/// * [`Error::Io`] if a source file can't be read.
pub fn check(file_groups: Vec<FileGroup>) -> Result<Report, Error> {
    let mut diagnostics = Vec::new();
    let mut num_files_checked = 0;
    let mut num_generated_files = 0;
    for file_group in file_groups {
        let mut file_checker = FileChecker::new(file_group.check_config);
        for source_file in &file_group.source_files {
            let source_text =
                fs::read_to_string(&source_file.path).map_err(|source| Error::Io {
                    path: source_file.path.clone(),
                    source,
                })?;
            if source::is_generated(&source_text) {
                num_generated_files += 1;
                continue;
            }
            num_files_checked += 1;
            diagnostics.extend(
                file_checker
                    .check(&source_file.path, &source_text, &source_file.line_filter)
                    .into_iter()
                    .map(|diagnostic| FileDiagnostic {
                        path: source_file.path.clone(),
                        diagnostic,
                    }),
            );
        }
    }
    FileDiagnostic::sort_and_dedup(&mut diagnostics);
    Ok(Report {
        diagnostics,
        num_files_checked,
        num_generated_files,
    })
}

/// The enabled rules of a run, with the state they keep across files.
struct FileChecker {
    rule_set: RuleSet,
    rules: Vec<Rule>,
    width: Width,

    /// The out-of-line test module verdicts, shared by the rules through each file's
    /// [`FileContext`].
    test_module_cache: TestModuleCache,
}

impl FileChecker {
    /// Factory function.
    ///
    /// # Returns
    ///
    /// A checker with the rules and settings of `check_config`.
    fn new(check_config: CheckConfig) -> Self {
        Self {
            rules: RuleId::ALL
                .into_iter()
                .filter(|rule_id| check_config.rule_set.contains(*rule_id))
                .map(|rule_id| Rule::new(rule_id, &check_config))
                .collect(),
            rule_set: check_config.rule_set,
            width: check_config.width,
            test_module_cache: TestModuleCache::default(),
        }
    }

    /// Runs every enabled rule on one file and applies its suppressions and line filter.
    ///
    /// The steps, in order:
    ///
    /// 1. The file's [`FileContext`] is built: it is lexed, and suppression comments are split from
    ///    the other comments.
    /// 2. Each enabled rule checks the file through the context. A rule that needs the `syn` syntax
    ///    tree gets it parsed once per file, and is skipped if the file failed to lex or to parse.
    /// 3. A lex failure becomes a `parse-error` diagnostic, suppressions drop the diagnostics they
    ///    cover, and unused and malformed suppressions are reported.
    /// 4. Diagnostics whose anchor span misses `line_filter` are dropped.
    ///
    /// # Returns
    ///
    /// The file's diagnostics, unsorted.
    fn check(
        &mut self,
        path: &Path,
        source_text: &str,
        line_filter: &LineFilter,
    ) -> Vec<Diagnostic> {
        let context = FileContext::new(path, source_text, self.width, &self.test_module_cache);
        let rule_diagnostics = self
            .rules
            .iter_mut()
            .flat_map(|rule| rule.check(&context))
            .collect();
        let mut diagnostics = context.finish(rule_diagnostics, &self.rule_set);
        if let LineFilter::Ranges(line_ranges) = line_filter {
            diagnostics.retain(|diagnostic| line_ranges.intersects(diagnostic.anchor()));
        }
        diagnostics
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OutputFormat;
    use crate::diagnostic::DiagnosticKind;

    fn file_checker(rule_set: RuleSet) -> FileChecker {
        FileChecker::new(CheckConfig {
            rule_set,
            width: Width::DEFAULT,
            test_skipping_rules: std::iter::empty().collect(),
        })
    }

    fn rendered(diagnostics: Vec<Diagnostic>) -> Vec<String> {
        let mut file_diagnostics: Vec<_> = diagnostics
            .into_iter()
            .map(|diagnostic| FileDiagnostic {
                path: PathBuf::from("a.rs"),
                diagnostic,
            })
            .collect();
        FileDiagnostic::sort_and_dedup(&mut file_diagnostics);
        file_diagnostics
            .iter()
            .map(|file_diagnostic| file_diagnostic.render(OutputFormat::Text))
            .collect()
    }

    #[test]
    fn lex_failure_is_one_parse_error() {
        let diagnostics = file_checker(RuleSet::all()).check(
            Path::new("a.rs"),
            "fn f() {\n    \"open\n",
            &LineFilter::All,
        );
        assert_eq!(
            rendered(diagnostics),
            [
                "a.rs:2:5: LNT001 parse-error: unterminated string literal; only the comments \
                 before it are checked"
            ]
        );
    }

    #[test]
    fn rule_that_skips_tests_drops_only_its_own_test_code_findings() {
        let source_text = concat!(
            "#[cfg(test)]\n",
            "mod tests {\n",
            "    /// Checks.\n",
            "    ///\n",
            "    /// # Errors\n",
            "    ///\n",
            "    /// Returns an error if:\n",
            "    ///\n",
            "    /// * Forwards [`load`]'s return values on failure.\n",
            "    fn checks() -> Result<(), E> {\n",
            "        Ok(())\n",
            "    }\n",
            "    /// A wrapper.\n",
            "    pub struct Wrapper<T>(T);\n",
            "    pub trait Store {\n",
            "        /// Loads.\n",
            "        fn load(&self, key: u8);\n",
            "    }\n",
            "}\n",
        );
        let diagnostics = |test_skipping_rules: &[RuleId]| {
            FileChecker::new(CheckConfig {
                rule_set: RuleSet::all(),
                width: Width::DEFAULT,
                test_skipping_rules: test_skipping_rules.iter().copied().collect(),
            })
            .check(Path::new("a.rs"), source_text, &LineFilter::All)
        };
        let all_diagnostics = diagnostics(&[]);
        let syntax_tree_rules = [
            RuleId::ErrorsDoc,
            RuleId::SymbolOrder,
            RuleId::TraitParamsDoc,
            RuleId::TypeParamsDoc,
        ];
        for rule_id in syntax_tree_rules {
            assert!(
                all_diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.kind() == rule_id.into()),
                "{rule_id}"
            );
            let others_diagnostics = all_diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.kind() != rule_id.into())
                .cloned()
                .collect();
            assert_eq!(
                rendered(diagnostics(&[rule_id])),
                rendered(others_diagnostics),
                "{rule_id}"
            );
        }
        assert_eq!(diagnostics(&syntax_tree_rules), []);
    }

    #[test]
    fn syn_failure_is_a_parse_error_only_when_errors_doc_runs() {
        let source_text = "fn f( {}\n";
        let diagnostics =
            file_checker(RuleSet::all()).check(Path::new("a.rs"), source_text, &LineFilter::All);
        let kinds: Vec<_> = diagnostics.iter().map(Diagnostic::kind).collect();
        assert_eq!(kinds, [DiagnosticKind::ParseError]);

        let rule_set: RuleSet = std::iter::once(RuleId::CommentGreedyWrap).collect();
        assert_eq!(
            file_checker(rule_set).check(Path::new("a.rs"), source_text, &LineFilter::All),
            []
        );
    }

    #[test]
    fn syn_failure_drops_only_the_rustfmt_hint_of_comment_greedy_wrap() {
        let source_text = format!(
            "struct S {{\n    /// {}\n    /// cc dd\n    a: u8,\n",
            "a".repeat(89)
        );
        let rule_set: RuleSet = std::iter::once(RuleId::CommentGreedyWrap).collect();
        let diagnostics =
            file_checker(rule_set).check(Path::new("a.rs"), &source_text, &LineFilter::All);
        let messages: Vec<_> = diagnostics.iter().map(Diagnostic::message).collect();
        assert_eq!(
            messages,
            [
                "line is 97 columns wide and the next line's first word `cc` (2 columns) fits \
                 within 100; move it up"
            ]
        );
        let parsable = format!("{source_text}}}\n");
        let diagnostics =
            file_checker(RuleSet::all()).check(Path::new("a.rs"), &parsable, &LineFilter::All);
        let messages: Vec<_> = diagnostics.iter().map(Diagnostic::message).collect();
        assert_eq!(
            messages,
            [
                "line is 97 columns wide and the next line's first word `cc` (2 columns) fits \
                 within 100; move it up, but rustfmt wraps this comment one column early \
                 (rust-lang/rustfmt#6180), so reword the paragraph instead"
            ]
        );
    }

    #[test]
    fn errors_doc_finding_is_kept_when_any_line_of_its_fn_is_in_range() {
        let source_text = "/// Loads.\n///\n/// # Errors\n///\n/// Returns an error if:\n///\n/// \
                           * Forwards [`save`]'s return values on failure.\nfn f() -> Result<(), \
                           E> {\n    load()?;\n    save()?;\n    Ok(())\n}\n\nfn g() {}\n";
        let messages = |ranges: &str| -> Vec<String> {
            let line_filter = LineFilter::Ranges(ranges.parse().expect("valid ranges"));
            file_checker(RuleSet::all())
                .check(Path::new("a.rs"), source_text, &line_filter)
                .iter()
                .map(|diagnostic| diagnostic.message().to_owned())
                .collect()
        };
        let finding = "`f` forwards errors from `load`, but `# Errors` does not list it; add \"* \
                       Forwards [`load`]'s return values on failure.\"";
        assert_eq!(messages("4"), [finding]);
        assert_eq!(messages("12"), [finding]);
        assert_eq!(messages("13-14"), Vec::<String>::new());
    }

    #[test]
    fn line_filter_keeps_diagnostics_whose_anchor_intersects_it() {
        let source_text = "// yscope-linter: allow(nothing)\n\n// yscope-linter: allow(nothing)\n";
        let line_filter = LineFilter::Ranges("3".parse().expect("valid ranges"));
        let diagnostics =
            file_checker(RuleSet::all()).check(Path::new("a.rs"), source_text, &line_filter);
        let lines: Vec<_> = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.position().line)
            .collect();
        assert_eq!(lines, [3]);
    }

    #[test]
    fn greedy_wrap_finding_is_kept_when_only_its_next_line_is_in_range() {
        let source_text = "// Short\n// line.\n\nfn f() {}\n";
        let lines = |ranges: &str| -> Vec<usize> {
            let line_filter = LineFilter::Ranges(ranges.parse().expect("valid ranges"));
            file_checker(RuleSet::all())
                .check(Path::new("a.rs"), source_text, &line_filter)
                .iter()
                .map(|diagnostic| diagnostic.position().line)
                .collect()
        };
        assert_eq!(lines("2"), [1]);
        assert_eq!(lines("3-4"), Vec::<usize>::new());
    }

    #[test]
    fn unused_suppression_is_reported_only_when_its_line_is_in_range() {
        let source_text = "// yscope-linter: allow(errors-doc): reason\nfn f() {}\n\nfn g() {}\n";
        let kinds = |ranges: &str| -> Vec<DiagnosticKind> {
            let line_filter = LineFilter::Ranges(ranges.parse().expect("valid ranges"));
            file_checker(RuleSet::all())
                .check(Path::new("a.rs"), source_text, &line_filter)
                .iter()
                .map(Diagnostic::kind)
                .collect()
        };
        assert_eq!(kinds("1"), [DiagnosticKind::UnusedSuppression]);
        assert_eq!(kinds("2-4"), []);
    }

    #[test]
    fn suppression_comment_ends_the_greedy_wrap_paragraph() {
        let rule_set: RuleSet = std::iter::once(RuleId::CommentGreedyWrap).collect();
        let check = |source_text: &str| -> Vec<usize> {
            file_checker(rule_set.clone())
                .check(Path::new("a.rs"), source_text, &LineFilter::All)
                .iter()
                .filter(|diagnostic| diagnostic.kind() == RuleId::CommentGreedyWrap.into())
                .map(|diagnostic| diagnostic.position().line)
                .collect()
        };
        assert_eq!(check("// Short\n// line.\n"), [1]);
        for suppression in [
            "allow(errors-doc): reason",
            "allow-item(errors-doc): reason",
            "allow-start(errors-doc): reason",
            "allow-end(errors-doc)",
            "allow-file(errors-doc): reason",
        ] {
            assert_eq!(
                check(&format!(
                    "// Short\n// yscope-linter: {suppression}\n// line.\nfn f() {{}}\n"
                )),
                Vec::<usize>::new(),
                "{suppression}"
            );
        }
    }

    #[test]
    fn allow_item_suppresses_the_greedy_wrap_findings_inside_its_item_only() {
        let rule_set: RuleSet = std::iter::once(RuleId::CommentGreedyWrap).collect();
        let source_text = "// yscope-linter: allow-item(comment-greedy-wrap): reason\n/// \
                           Short\n/// line.\nfn f() {}\n// Short\n// line.\n";
        let lines: Vec<_> = file_checker(rule_set)
            .check(Path::new("a.rs"), source_text, &LineFilter::All)
            .iter()
            .map(|diagnostic| (diagnostic.kind(), diagnostic.position().line))
            .collect();
        assert_eq!(lines, [(RuleId::CommentGreedyWrap.into(), 5)]);
    }

    #[test]
    fn greedy_wrap_only_run_parses_only_files_that_need_items_or_statements() {
        let rule_set: RuleSet = std::iter::once(RuleId::CommentGreedyWrap).collect();
        let kinds = |source_text: &str| -> Vec<DiagnosticKind> {
            file_checker(rule_set.clone())
                .check(Path::new("a.rs"), source_text, &LineFilter::All)
                .iter()
                .map(Diagnostic::kind)
                .collect()
        };
        assert_eq!(kinds("fn f( {}\n"), []);
        assert_eq!(
            kinds("// yscope-linter: allow-item(comment-greedy-wrap): reason\nfn f( {}\n"),
            [DiagnosticKind::InvalidSuppression]
        );
        assert_eq!(
            kinds("// yscope-linter: allow(comment-greedy-wrap): reason\nfn f( {}\n"),
            [DiagnosticKind::UnusedSuppression]
        );
    }

    #[test]
    fn own_line_allow_above_a_multi_line_statement_covers_its_question_mark() {
        let source_text = concat!(
            "/// Runs the job.\n",
            "///\n",
            "/// # Errors\n",
            "///\n",
            "/// Returns an error if:\n",
            "///\n",
            "/// * Forwards [`Store::insert`]'s return values on failure.\n",
            "fn run(&self, job: Job) -> Result<(), Error> {\n",
            "    self.store.insert(&job)?;\n",
            "    // yscope-linter: allow(errors-doc): the audit log documents its own errors\n",
            "    self.audit\n",
            "        .record(&job)\n",
            "        .await?;\n",
            "    Ok(())\n",
            "}\n",
        );
        let diagnostics =
            file_checker(RuleSet::all()).check(Path::new("a.rs"), source_text, &LineFilter::All);
        assert_eq!(rendered(diagnostics), Vec::<String>::new());
    }

    #[test]
    fn allow_with_a_rustfmt_wrapped_reason_covers_the_statement_below_it() {
        let source_text = concat!(
            "/// Runs the job.\n",
            "///\n",
            "/// # Errors\n",
            "///\n",
            "/// Returns an error if:\n",
            "///\n",
            "/// * Forwards [`Store::insert`]'s return values on failure.\n",
            "fn run(&self, job: Job) -> Result<(), Error> {\n",
            "    self.store.insert(&job)?;\n",
            "    // yscope-linter: allow(errors-doc): the audit log documents its own errors, and \
             the caller\n",
            "    // reports them to the user\n",
            "    self.audit\n",
            "        .record(&job)\n",
            "        .await?;\n",
            "    Ok(())\n",
            "}\n",
        );
        let diagnostics =
            file_checker(RuleSet::all()).check(Path::new("a.rs"), source_text, &LineFilter::All);
        assert_eq!(rendered(diagnostics), Vec::<String>::new());

        let rule_set: RuleSet = std::iter::once(RuleId::CommentGreedyWrap).collect();
        let source_text = concat!(
            "fn f() {\n",
            "    // yscope-linter: allow(comment-greedy-wrap): the comments in the closure are \
             copied from the\n",
            "    // upstream source\n",
            "    let c = || {\n",
            "        // A comment that ends\n",
            "        // early.\n",
            "    };\n",
            "}\n",
        );
        assert_eq!(
            file_checker(rule_set).check(Path::new("a.rs"), source_text, &LineFilter::All),
            []
        );
    }

    #[test]
    fn fn_header_allow_still_covers_every_errors_doc_finding_of_the_fn() {
        let source_text = concat!(
            "/// Runs.\n",
            "///\n",
            "/// # Errors\n",
            "///\n",
            "/// Returns an error if:\n",
            "///\n",
            "/// * Forwards [`a`]'s return values on failure.\n",
            "// yscope-linter: allow(errors-doc): the errors come from the callback the caller \
             provides, so the\n",
            "// caller documents them\n",
            "fn run() -> Result<(), E> {\n",
            "    b()?;\n",
            "    self.audit\n",
            "        .record()\n",
            "        .await?;\n",
            "    Ok(())\n",
            "}\n",
        );
        let diagnostics =
            file_checker(RuleSet::all()).check(Path::new("a.rs"), source_text, &LineFilter::All);
        assert_eq!(rendered(diagnostics), Vec::<String>::new());
    }
}
