//! The entry point of the `tuple-returns-doc` rule.

use crate::diagnostic::Diagnostic;
use crate::diagnostic::Position;
use crate::diagnostic::counted;
use crate::doc::doc_lines;
use crate::doc::links::LinkDefinitions;
use crate::doc::section;
use crate::doc::section::Bullet;
use crate::file_context::FileContext;
use crate::rules::RuleId;
use crate::syntax::doc_items;
use crate::syntax::doc_items::DocItem;
use crate::syntax::types::impl_future_output;
use crate::syntax::types::is_result_path;
use crate::syntax::types::peel_type_parens;

/// Checks the `# Returns` sections of the fns that return tuples in each file of a run.
#[derive(Debug)]
pub struct Checker {
    skip_tests: bool,
}

impl Checker {
    /// Factory function.
    ///
    /// # Returns
    ///
    /// A checker that skips `#[test]` fns and `#[cfg(test)]` items if `skip_tests` is set.
    pub const fn new(skip_tests: bool) -> Self {
        Self { skip_tests }
    }

    /// Checks every documented fn of one file that returns a tuple, or a `Result` of one, and has a
    /// `# Returns` section:
    ///
    /// * The section starts with "A tuple containing:", or "A tuple on success, containing:" for a
    ///   `Result`.
    /// * The list right after that paragraph has one top-level bullet per element of the tuple.
    ///
    /// The file is skipped if it failed to lex or to parse, or if it is an out-of-line
    /// `#[cfg(test)]` module and tests are skipped.
    ///
    /// # Returns
    ///
    /// A `tuple-returns-doc` diagnostic for every wrong intro and every list whose bullet count
    /// doesn't match the tuple. Each one's anchor span is the whole item, and its fn header is the
    /// lines from the item's first attribute or doc line through the item's own first line.
    pub fn check(&self, context: &FileContext<'_>) -> Vec<Diagnostic> {
        let Some(syntax_tree) = context.syntax_tree(RuleId::TupleReturnsDoc) else {
            return Vec::new();
        };
        if self.skip_tests && context.is_test_module(RuleId::TupleReturnsDoc) {
            return Vec::new();
        }
        doc_items::collect(syntax_tree, self.skip_tests)
            .iter()
            .flat_map(|doc_item| {
                item_findings(doc_item)
                    .into_iter()
                    .map(|(position, message)| {
                        Diagnostic::new(RuleId::TupleReturnsDoc.into(), position, message)
                            .with_anchor(doc_item.lines.span)
                            .with_fn_header(doc_item.lines.header)
                    })
            })
            .collect()
    }
}

/// The tuple that a fn returns, directly or as the `Ok` type of a `Result`.
#[derive(Debug, PartialEq, Eq)]
struct TupleReturn {
    num_elements: usize,
    is_result: bool,
}

impl TupleReturn {
    /// Classifies a fn's return type, after peeling an `impl Future<Output = ..>` to its `Output`:
    ///
    /// * A tuple of at least two elements is a plain tuple return.
    /// * A path whose last segment ends with `Result`, and whose first generic type argument is
    ///   such a tuple, is a `Result` of a tuple.
    ///
    /// # Returns
    ///
    /// The tuple return, or [`None`] if `output` is neither.
    fn of(output: &syn::ReturnType) -> Option<Self> {
        let syn::ReturnType::Type(_, return_type) = output else {
            return None;
        };
        let return_type = impl_future_output(return_type).unwrap_or(return_type);
        if let Some(num_elements) = num_tuple_elements(return_type) {
            return Some(Self {
                num_elements,
                is_result: false,
            });
        }
        let syn::Type::Path(type_path) = peel_type_parens(return_type) else {
            return None;
        };
        if !is_result_path(&type_path.path) {
            return None;
        }
        let syn::PathArguments::AngleBracketed(arguments) =
            &type_path.path.segments.last()?.arguments
        else {
            return None;
        };
        let ok_type = arguments.args.iter().find_map(|argument| match argument {
            syn::GenericArgument::Type(ty) => Some(ty),
            _ => None,
        })?;
        num_tuple_elements(ok_type).map(|num_elements| Self {
            num_elements,
            is_result: true,
        })
    }

    /// # Returns
    ///
    /// The paragraph that must start the `# Returns` section.
    const fn intro(&self) -> &'static str {
        if self.is_result {
            "A tuple on success, containing:"
        } else {
            "A tuple containing:"
        }
    }

    /// # Returns
    ///
    /// The message of a `# Returns` section of `fn_name` that doesn't start with the intro.
    fn intro_message(&self, fn_name: &str) -> String {
        let returned = if self.is_result {
            "a `Result` of a tuple"
        } else {
            "a tuple"
        };
        format!(
            "`{fn_name}` returns {returned}; start `# Returns` with \"{}\"",
            self.intro()
        )
    }

    /// # Returns
    ///
    /// The message of a `# Returns` section of `fn_name` whose element list has `num_bullets`
    /// top-level bullets, which isn't the number of elements.
    fn count_message(&self, fn_name: &str, num_bullets: usize) -> String {
        let fix = if num_bullets < self.num_elements {
            "add a bullet for each element, in order"
        } else {
            "keep one bullet per element, in order"
        };
        format!(
            "{}, but `# Returns` lists {}; {fix}",
            self.subject(fn_name),
            counted(num_bullets, "element")
        )
    }

    /// # Returns
    ///
    /// The message of a `# Returns` section of `fn_name` whose bullets follow a paragraph other
    /// than the intro.
    fn detached_list_message(&self, fn_name: &str) -> String {
        format!(
            "{}, but no list follows the `# Returns` intro; move the element bullets right after \
             it",
            self.subject(fn_name)
        )
    }

    /// # Returns
    ///
    /// What `fn_name` returns, such as ``` `f` returns a 2-element tuple ```.
    fn subject(&self, fn_name: &str) -> String {
        let tuple = format!("{}-element tuple", self.num_elements);
        if self.is_result {
            format!("`{fn_name}` returns a `Result` of a {tuple}")
        } else {
            format!("`{fn_name}` returns a {tuple}")
        }
    }
}

/// Checks the `# Returns` section of `doc_item` if it is a documented fn that returns a tuple.
///
/// The element list is the top-level bullets right after the section's first paragraph, or the ones
/// the section starts with if it starts with a bullet. Its bullet count is checked when the intro
/// is right or the list isn't empty.
///
/// # Returns
///
/// The position and message of each finding.
fn item_findings(doc_item: &DocItem<'_>) -> Vec<(Position, String)> {
    let Some(tuple_return) = doc_item
        .kind
        .signature()
        .and_then(|sig| TupleReturn::of(&sig.output))
    else {
        return Vec::new();
    };
    let Some(doc_lines) = doc_lines::collect_documented(doc_item.attrs) else {
        return Vec::new();
    };
    let Some(section) = section::find(&doc_lines, "Returns") else {
        return Vec::new();
    };
    let texts: Vec<&str> = doc_lines.iter().map(|line| line.text.as_str()).collect();
    let definitions = LinkDefinitions::collect(&texts);
    let content = section.content(&doc_lines, &definitions);
    let first_position = content
        .first_content
        .map_or(section.header, |first_content| first_content.position);
    let starts_with_prose = content
        .first_content
        .is_some_and(|first_content| first_content.is_prose);
    let has_intro = starts_with_prose
        && content
            .prose
            .first()
            .is_some_and(|paragraph| paragraph == tuple_return.intro());
    let mut findings = Vec::new();
    if !has_intro {
        findings.push((first_position, tuple_return.intro_message(&doc_item.name)));
    }
    let elements: Vec<&Bullet> = content
        .bullets
        .iter()
        .filter(|bullet| bullet.num_prose_before == usize::from(starts_with_prose))
        .collect();
    match elements.first() {
        Some(first_element) if elements.len() != tuple_return.num_elements => findings.push((
            first_element.position,
            tuple_return.count_message(&doc_item.name, elements.len()),
        )),
        Some(_) => {}
        None if !has_intro => {}
        None if content.bullets.is_empty() => findings.push((
            first_position,
            tuple_return.count_message(&doc_item.name, 0),
        )),
        None => findings.push((
            first_position,
            tuple_return.detached_list_message(&doc_item.name),
        )),
    }
    findings
}

/// # Returns
///
/// The number of elements of `ty` if it is a tuple of at least two elements, or [`None`] otherwise.
fn num_tuple_elements(ty: &syn::Type) -> Option<usize> {
    match peel_type_parens(ty) {
        syn::Type::Tuple(type_tuple) if type_tuple.elems.len() >= 2 => Some(type_tuple.elems.len()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::syntax::test_code::TestModuleCache;

    const NO_FINDINGS: [String; 0] = [];

    /// Checks `source_text` as a file that no other file declares.
    ///
    /// # Returns
    ///
    /// Each finding as `<line>:<column>: <message>`, sorted by position.
    ///
    /// # Panics
    ///
    /// Panics if `source_text` doesn't parse.
    fn checked(source_text: &str, skip_tests: bool) -> Vec<String> {
        diagnostics(source_text, skip_tests)
            .iter()
            .map(|diagnostic| {
                let position = diagnostic.position();
                format!(
                    "{}:{}: {}",
                    position.line,
                    position.column,
                    diagnostic.message()
                )
            })
            .collect()
    }

    fn diagnostics(source_text: &str, skip_tests: bool) -> Vec<Diagnostic> {
        syn::parse_file(source_text).expect("valid file");
        let test_module_cache = TestModuleCache::default();
        let context = FileContext::new(
            Path::new("nonexistent-fixture-directory/subject.rs"),
            source_text,
            &test_module_cache,
        );
        let mut diagnostics = Checker::new(skip_tests).check(&context);
        diagnostics.sort_by_key(Diagnostic::position);
        diagnostics
    }

    /// # Returns
    ///
    /// A file with one fn `f` that returns `return_type`, documented by a summary line and a
    /// `# Returns` section with the lines of `section_body`.
    fn documented_fn(return_type: &str, section_body: &[&str]) -> String {
        let body: String = section_body
            .iter()
            .map(|line| {
                if line.is_empty() {
                    "///\n".to_owned()
                } else {
                    format!("/// {line}\n")
                }
            })
            .collect();
        format!(
            "/// Does it.\n///\n/// # Returns\n///\n{body}fn f() -> {return_type} {{\n    \
             todo!()\n}}\n"
        )
    }

    #[test]
    fn issue_examples_are_reported() {
        let source_text = concat!(
            "/// Resolves the terminal state for a job with the given outcome and current \
             status.\n",
            "///\n",
            "/// # Returns\n",
            "///\n",
            "/// On success, a tuple containing:\n",
            "///\n",
            "/// * The resolved status.\n",
            "/// * The error message to store.\n",
            "fn resolve_terminal_state(\n",
            "    outcome: JobOutcome,\n",
            "    current_status: QueryJobStatus,\n",
            ") -> Result<(QueryJobStatus, Option<String>), Error> {\n",
            "    todo!()\n",
            "}\n",
        );
        assert_eq!(
            checked(source_text, false),
            [
                "5:1: `resolve_terminal_state` returns a `Result` of a tuple; start `# Returns` \
                 with \"A tuple on success, containing:\""
            ]
        );
        let source_text = concat!(
            "/// Splits the submission into its parts.\n",
            "///\n",
            "/// # Returns\n",
            "///\n",
            "/// A tuple containing:\n",
            "///\n",
            "/// * The task graph.\n",
            "/// * The job inputs.\n",
            "pub fn into_parts(self) -> (TaskGraph, TaskGraphInput, \
             ValidatedCompressedJobSubmission) {\n",
            "    todo!()\n",
            "}\n",
        );
        assert_eq!(
            checked(source_text, false),
            [
                "7:1: `into_parts` returns a 3-element tuple, but `# Returns` lists 2 elements; \
                 add a bullet for each element, in order"
            ]
        );
    }

    #[test]
    fn canonical_sections_are_clean_for_every_tuple_return_shape() {
        let plain = ["A tuple containing:", "", "* The first.", "* The second."];
        let on_success = [
            "A tuple on success, containing:",
            "",
            "* The first.",
            "* The second.",
        ];
        let cases = [
            documented_fn(
                "(A, B, C)",
                &[
                    "A tuple containing:",
                    "",
                    "* The first.",
                    "* The second.",
                    "* The third.",
                ],
            ),
            documented_fn("(A, B)", &plain),
            documented_fn("((A, B))", &plain),
            documented_fn("Result<(A, B), E>", &on_success),
            documented_fn("anyhow::Result<(A, B)>", &on_success),
            documented_fn("DbResult<(A, B), E>", &on_success),
            documented_fn("impl Future<Output = (A, B)> + Send", &plain),
            documented_fn("impl Future<Output = Result<(A, B), E>>", &on_success),
        ];
        for source_text in &cases {
            assert_eq!(checked(source_text, false), NO_FINDINGS, "{source_text}");
        }
    }

    #[test]
    fn element_lists_with_nested_bullets_continuations_and_later_text_are_clean() {
        let cases = [
            documented_fn(
                "(A, (B, C))",
                &[
                    "A tuple containing:",
                    "",
                    "* The first.",
                    "* The pair:",
                    "  * The second.",
                    "  * The third.",
                ],
            ),
            documented_fn(
                "(A, B)",
                &[
                    "A tuple containing:",
                    "",
                    "* The first, which continues",
                    "  on the next line.",
                    "* The second.",
                    "",
                    "  More about the second.",
                    "",
                    "A note after the list.",
                    "",
                    "* An unrelated list.",
                    "* After the note.",
                    "* With three bullets.",
                ],
            ),
            documented_fn(
                "(A, B)",
                &[
                    "A tuple",
                    "containing:",
                    "* The first.",
                    "* The second.",
                    "",
                    "```",
                    "* Fenced.",
                    "```",
                    "",
                    "[`x`]: crate::x",
                ],
            ),
            documented_fn(
                "(A, B)",
                &[
                    "A tuple containing:",
                    "",
                    "* The first.",
                    "* The second.",
                    "",
                    "## Errors",
                    "",
                    "* An error of the stream.",
                    "* Another one.",
                ],
            ),
            documented_fn(
                "(A, B)",
                &["A tuple containing:", "", "1. The first.", "2. The second."],
            ),
        ];
        for source_text in &cases {
            assert_eq!(checked(source_text, false), NO_FINDINGS, "{source_text}");
        }
    }

    #[test]
    fn canonical_sections_are_clean_on_every_fn_kind() {
        let source_text = concat!(
            "/// Runs.\n",
            "///\n",
            "/// # Returns\n",
            "///\n",
            "/// A tuple containing:\n",
            "///\n",
            "/// * The first.\n",
            "/// * The second.\n",
            "async fn run() -> (A, B) {\n",
            "    todo!()\n",
            "}\n",
            "trait Store {\n",
            "    /// Loads.\n",
            "    ///\n",
            "    /// # Returns\n",
            "    ///\n",
            "    /// A tuple on success, containing:\n",
            "    ///\n",
            "    /// * The first.\n",
            "    /// * The second.\n",
            "    fn load(&self) -> Result<(A, B), E>;\n",
            "}\n",
            "impl S {\n",
            "    /// Splits.\n",
            "    ///\n",
            "    /// # Returns\n",
            "    ///\n",
            "    /// A tuple containing:\n",
            "    ///\n",
            "    /// * The first.\n",
            "    /// * The second.\n",
            "    fn split(self) -> (A, B) {\n",
            "        todo!()\n",
            "    }\n",
            "}\n",
            "impl Store for S {\n",
            "    /// Loads.\n",
            "    ///\n",
            "    /// # Returns\n",
            "    ///\n",
            "    /// A tuple on success, containing:\n",
            "    ///\n",
            "    /// * The first.\n",
            "    /// * The second.\n",
            "    fn load(&self) -> Result<(A, B), E> {\n",
            "        todo!()\n",
            "    }\n",
            "}\n",
        );
        assert_eq!(checked(source_text, false), NO_FINDINGS);
    }

    #[test]
    fn wrong_intros_are_reported_at_the_first_paragraph() {
        let cases = [
            (
                documented_fn(
                    "Result<(A, B), E>",
                    &[
                        "On success, a tuple containing:",
                        "",
                        "* The first.",
                        "* The second.",
                    ],
                ),
                vec![
                    "5:1: `f` returns a `Result` of a tuple; start `# Returns` with \"A tuple on \
                     success, containing:\"",
                ],
            ),
            (
                documented_fn(
                    "Result<(A, B), E>",
                    &["A tuple containing:", "", "* The first.", "* The second."],
                ),
                vec![
                    "5:1: `f` returns a `Result` of a tuple; start `# Returns` with \"A tuple on \
                     success, containing:\"",
                ],
            ),
            (
                documented_fn(
                    "impl Future<Output = Result<(A, B), E>> + Send",
                    &["A tuple containing:", "", "* The first.", "* The second."],
                ),
                vec![
                    "5:1: `f` returns a `Result` of a tuple; start `# Returns` with \"A tuple on \
                     success, containing:\"",
                ],
            ),
            (
                documented_fn(
                    "(A, B)",
                    &[
                        "A tuple on success, containing:",
                        "",
                        "* The first.",
                        "* The second.",
                    ],
                ),
                vec!["5:1: `f` returns a tuple; start `# Returns` with \"A tuple containing:\""],
            ),
            (
                documented_fn("(A, B)", &["Returns a tuple of the parts."]),
                vec!["5:1: `f` returns a tuple; start `# Returns` with \"A tuple containing:\""],
            ),
            (
                documented_fn("(A, B)", &["* The first.", "* The second."]),
                vec!["5:1: `f` returns a tuple; start `# Returns` with \"A tuple containing:\""],
            ),
            (
                documented_fn(
                    "(A, B)",
                    &["* The first.", "* The second.", "", "A tuple containing:"],
                ),
                vec!["5:1: `f` returns a tuple; start `# Returns` with \"A tuple containing:\""],
            ),
            (
                documented_fn("(A, B)", &["# Errors", "", "Returns an error if:"]),
                vec!["3:1: `f` returns a tuple; start `# Returns` with \"A tuple containing:\""],
            ),
        ];
        for (source_text, expected) in &cases {
            assert_eq!(checked(source_text, false), *expected, "{source_text}");
        }
    }

    #[test]
    fn bullet_counts_that_do_not_match_are_reported_at_the_list() {
        let cases = [
            (
                documented_fn(
                    "(A, B)",
                    &[
                        "A tuple containing:",
                        "",
                        "* The first.",
                        "* The second.",
                        "* The third.",
                    ],
                ),
                vec![
                    "7:1: `f` returns a 2-element tuple, but `# Returns` lists 3 elements; keep \
                     one bullet per element, in order",
                ],
            ),
            (
                documented_fn(
                    "Result<(A, B, C), E>",
                    &["A tuple on success, containing:", "", "* The first."],
                ),
                vec![
                    "7:1: `f` returns a `Result` of a 3-element tuple, but `# Returns` lists 1 \
                     element; add a bullet for each element, in order",
                ],
            ),
            (
                documented_fn("(A, B)", &["A tuple containing:"]),
                vec![
                    "5:1: `f` returns a 2-element tuple, but `# Returns` lists 0 elements; add a \
                     bullet for each element, in order",
                ],
            ),
            (
                documented_fn(
                    "(A, B)",
                    &[
                        "A tuple containing:",
                        "",
                        "Note that both are owned.",
                        "",
                        "* The first.",
                        "* The second.",
                    ],
                ),
                vec![
                    "5:1: `f` returns a 2-element tuple, but no list follows the `# Returns` \
                     intro; move the element bullets right after it",
                ],
            ),
            (
                documented_fn("(A, B)", &["The parts:", "", "* The first."]),
                vec![
                    "5:1: `f` returns a tuple; start `# Returns` with \"A tuple containing:\"",
                    "7:1: `f` returns a 2-element tuple, but `# Returns` lists 1 element; add a \
                     bullet for each element, in order",
                ],
            ),
            (
                documented_fn("(A, B, C)", &["* The first.", "* The second."]),
                vec![
                    "5:1: `f` returns a tuple; start `# Returns` with \"A tuple containing:\"",
                    "5:1: `f` returns a 3-element tuple, but `# Returns` lists 2 elements; add a \
                     bullet for each element, in order",
                ],
            ),
            (
                documented_fn(
                    "(A, B)",
                    &[
                        "A tuple containing:",
                        "",
                        "1. The first.",
                        "2. The second.",
                        "3. The third.",
                    ],
                ),
                vec![
                    "7:1: `f` returns a 2-element tuple, but `# Returns` lists 3 elements; keep \
                     one bullet per element, in order",
                ],
            ),
        ];
        for (source_text, expected) in &cases {
            assert_eq!(checked(source_text, false), *expected, "{source_text}");
        }
        let sub_bullets = documented_fn(
            "(A, B)",
            &[
                "A tuple containing:",
                "",
                "* The first:",
                "  * A detail.",
                "  * Another detail.",
                "  * A third detail.",
                "* The second.",
            ],
        );
        assert_eq!(checked(&sub_bullets, false), NO_FINDINGS);
    }

    #[test]
    fn findings_on_methods_are_reported_at_their_doc_lines() {
        let source_text = concat!(
            "impl S {\n",
            "    /// Splits.\n",
            "    ///\n",
            "    /// # Returns\n",
            "    ///\n",
            "    /// The parts.\n",
            "    fn split(self) -> (A, B) {\n",
            "        todo!()\n",
            "    }\n",
            "}\n",
            "trait Store {\n",
            "    /// Loads.\n",
            "    ///\n",
            "    /// # Returns\n",
            "    ///\n",
            "    /// A tuple on success, containing:\n",
            "    ///\n",
            "    /// * The first.\n",
            "    fn load(&self) -> Result<(A, B), E>;\n",
            "}\n",
        );
        assert_eq!(
            checked(source_text, false),
            [
                "6:5: `split` returns a tuple; start `# Returns` with \"A tuple containing:\"",
                "18:5: `load` returns a `Result` of a 2-element tuple, but `# Returns` lists 1 \
                 element; add a bullet for each element, in order",
            ]
        );
    }

    #[test]
    fn returns_that_are_not_tuples_are_not_checked() {
        let return_types = [
            "()",
            "(A,)",
            "Result<(), E>",
            "Result<(A,), E>",
            "Option<(A, B)>",
            "Vec<(A, B)>",
            "Box<(A, B)>",
            "Pair",
            "PairResult",
            "Pin<Box<dyn Future<Output = (A, B)>>>",
            "impl Iterator<Item = (A, B)>",
            "impl Future<Output = Option<(A, B)>>",
        ];
        for return_type in return_types {
            let source_text = documented_fn(return_type, &["The pair."]);
            assert_eq!(checked(&source_text, false), NO_FINDINGS, "{return_type}");
        }
        let source_text = concat!(
            "type Pair = (A, B);\n",
            "/// Scans.\n",
            "///\n",
            "/// # Returns\n",
            "///\n",
            "/// The first match.\n",
            "fn scan<Callback: FnMut() -> Result<(A, B)>>(\n",
            "    callback: Callback,\n",
            ") -> Result<Option<A>> {\n",
            "    todo!()\n",
            "}\n",
            "/// Runs.\n",
            "///\n",
            "/// # Returns\n",
            "///\n",
            "/// Nothing.\n",
            "fn run() {}\n",
        );
        assert_eq!(checked(source_text, false), NO_FINDINGS);
    }

    #[test]
    fn fns_without_a_returns_section_or_docstring_are_not_checked() {
        let source_text = concat!(
            "fn undocumented() -> (A, B) {\n",
            "    todo!()\n",
            "}\n",
            "#[doc = \"\"]\n",
            "fn blank() -> (A, B) {\n",
            "    todo!()\n",
            "}\n",
            "/// Splits.\n",
            "fn no_section() -> (A, B) {\n",
            "    todo!()\n",
            "}\n",
            "/// Splits.\n",
            "///\n",
            "/// # Return\n",
            "///\n",
            "/// The parts.\n",
            "fn misspelled() -> (A, B) {\n",
            "    todo!()\n",
            "}\n",
            "/// Splits.\n",
            "///\n",
            "/// # Notes\n",
            "///\n",
            "/// ## Returns\n",
            "///\n",
            "/// The parts.\n",
            "fn nested_header() -> (A, B) {\n",
            "    todo!()\n",
            "}\n",
            "/// A pair.\n",
            "///\n",
            "/// # Returns\n",
            "///\n",
            "/// The parts.\n",
            "struct Pair(A, B);\n",
        );
        assert_eq!(checked(source_text, false), NO_FINDINGS);
    }

    #[test]
    fn test_code_is_checked_unless_tests_are_skipped() {
        let source_text = concat!(
            "#[cfg(test)]\n",
            "mod tests {\n",
            "    /// Builds.\n",
            "    ///\n",
            "    /// # Returns\n",
            "    ///\n",
            "    /// The pair.\n",
            "    fn pair() -> (u8, u8) {\n",
            "        (0, 0)\n",
            "    }\n",
            "    trait Fixture {\n",
            "        /// Sets up.\n",
            "        ///\n",
            "        /// # Returns\n",
            "        ///\n",
            "        /// The pair.\n",
            "        fn set_up(&self) -> Result<(u8, u8), E>;\n",
            "    }\n",
            "}\n",
        );
        assert_eq!(checked(source_text, true), NO_FINDINGS);
        assert_eq!(
            checked(source_text, false),
            [
                "7:5: `pair` returns a tuple; start `# Returns` with \"A tuple containing:\"",
                "16:9: `set_up` returns a `Result` of a tuple; start `# Returns` with \"A tuple \
                 on success, containing:\"",
            ]
        );
    }

    #[test]
    fn diagnostics_are_anchored_to_the_item_with_its_header() {
        let source_text = concat!(
            "impl S {\n",
            "    /// Splits.\n",
            "    ///\n",
            "    /// # Returns\n",
            "    ///\n",
            "    /// The parts.\n",
            "    #[must_use]\n",
            "    fn split(self) -> (A, B) {\n",
            "        todo!()\n",
            "    }\n",
            "}\n",
        );
        let diagnostics = diagnostics(source_text, false);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| (
                    diagnostic.anchor().to_string(),
                    diagnostic.fn_header().map(|header| header.to_string())
                ))
                .collect::<Vec<_>>(),
            [("2-10".to_owned(), Some("2-8".to_owned()))]
        );
    }
}
