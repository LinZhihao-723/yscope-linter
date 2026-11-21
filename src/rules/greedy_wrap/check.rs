//! The entry point of the `comment-greedy-wrap` rule: the fit test and its diagnostics.

use crate::config::Width;
use crate::diagnostic::Diagnostic;
use crate::diagnostic::Position;
use crate::diagnostic::counted;
use crate::file_context::FileContext;
use crate::lexer::CommentLine;
use crate::line_ranges::LineRanges;
use crate::line_ranges::LineSpan;
use crate::markdown::BlockStart;
use crate::markdown::InlineKind;
use crate::markdown::ParagraphText;
use crate::markdown::Preceding;
use crate::markdown::Token;
use crate::rules::RuleId;
use crate::rules::greedy_wrap::line_kind::Body;
use crate::rules::greedy_wrap::paragraph;
use crate::rules::greedy_wrap::paragraph::ProseParagraph;
use crate::rules::greedy_wrap::rustfmt_lists;

/// The `comment-greedy-wrap` rule, with the width of a run.
#[derive(Debug)]
pub struct Checker {
    width: Width,
}

impl Checker {
    /// Factory function.
    ///
    /// # Returns
    ///
    /// A checker that fills comment paragraphs to `width` columns.
    pub const fn new(width: Width) -> Self {
        Self { width }
    }

    /// Checks that every comment paragraph of one file is filled greedily to the width. Suppression
    /// comments are not part of any paragraph.
    ///
    /// A move that would fill a line to exactly the width gets a hint when the line is in a field
    /// or parameter list, where rustfmt wraps comments one column early. A file that fails to parse
    /// gets no hint.
    ///
    /// # Returns
    ///
    /// A `comment-greedy-wrap` diagnostic for every line that leaves room for the leading tokens of
    /// the next line, and for every token that is split across lines although it fits on one.
    pub fn check(&self, context: &FileContext<'_>) -> Vec<Diagnostic> {
        let findings: Vec<_> = paragraph::blocks(context.comment_lines())
            .flat_map(paragraph::prose_paragraphs)
            .flat_map(|prose_paragraph| Finding::find_all(prose_paragraph, self.width))
            .collect();
        let early_wrap_lines = findings
            .iter()
            .any(|finding| {
                matches!(finding, Finding::Underfilled(underfilled_line)
                    if underfilled_line.fills_width(self.width))
            })
            .then(|| context.optional_syntax_tree())
            .flatten()
            .map(rustfmt_lists::early_wrap_lines);
        findings
            .into_iter()
            .map(|finding| finding.diagnostic(self.width, early_wrap_lines.as_ref()))
            .collect()
    }
}

/// A violation in one prose paragraph.
///
/// # Type Parameters
///
/// * `'comment` - The lifetime of the comment lines.
enum Finding<'comment> {
    Underfilled(UnderfilledLine<'comment>),
    SplitToken(SplitToken<'comment>),
}

impl<'comment> Finding<'comment> {
    /// Checks one prose paragraph. Its text is split into tokens, and a token that fits on a line
    /// of its own is never broken. A wider token is broken at its whitespace.
    ///
    /// # Returns
    ///
    /// The paragraph's tokens that are split across lines although they fit on one, then its
    /// under-filled lines.
    fn find_all(prose_paragraph: ProseParagraph<'comment>, width: Width) -> Vec<Self> {
        let lines: Vec<_> = prose_paragraph
            .lines
            .iter()
            .map(|comment_line| ParagraphLine::new(comment_line, width))
            .collect();
        let texts: Vec<_> = lines.iter().map(|line| line.text).collect();
        let paragraph_text = ParagraphText::new(&texts);
        let tokens = paragraph_text.tokens();
        let split_tokens = tokens
            .iter()
            .filter_map(|token| SplitToken::find(*token, &lines, prose_paragraph.is_list_item))
            .map(Self::SplitToken);
        let underfilled_lines = (1..lines.len())
            .filter_map(|next_index| {
                UnderfilledLine::find(
                    &lines,
                    &tokens,
                    next_index,
                    prose_paragraph.preceding,
                    width,
                )
            })
            .map(Self::Underfilled);
        split_tokens.chain(underfilled_lines).collect()
    }

    /// Builds the finding's diagnostic. An under-filled line gets the rustfmt hint when its move
    /// fills it to exactly `width` and it is in `early_wrap_lines`.
    ///
    /// # Returns
    ///
    /// The diagnostic.
    fn diagnostic(self, width: Width, early_wrap_lines: Option<&LineRanges>) -> Diagnostic {
        match self {
            Self::Underfilled(underfilled_line) => {
                let is_wrapped_early = underfilled_line.fills_width(width)
                    && early_wrap_lines.is_some_and(|line_ranges| {
                        line_ranges.intersects(LineSpan::single(underfilled_line.line.line))
                    });
                underfilled_line.diagnostic(width, is_wrapped_early)
            }
            Self::SplitToken(split_token) => split_token.diagnostic(width),
        }
    }
}

/// One line of a prose paragraph.
///
/// # Type Parameters
///
/// * `'comment` - The lifetime of the comment line.
struct ParagraphLine<'comment> {
    comment_line: &'comment CommentLine,

    /// The line's body without leading and trailing whitespace.
    text: &'comment str,

    /// The width of the widest token that fits on the line alone: the width minus the line's
    /// prefix, which is its indentation, its comment marker and the whitespace before its text.
    capacity: usize,
}

impl<'comment> ParagraphLine<'comment> {
    /// Factory function.
    ///
    /// # Returns
    ///
    /// The paragraph line of `comment_line`, in a run filled to `width` columns.
    fn new(comment_line: &'comment CommentLine, width: Width) -> Self {
        let text = Body::new(&comment_line.text).trimmed();
        let prefix_width = comment_line.width - text.chars().count();
        Self {
            comment_line,
            text,
            capacity: width.columns().saturating_sub(prefix_width),
        }
    }
}

/// A line that leaves room for the leading tokens of the next line of its paragraph.
///
/// # Type Parameters
///
/// * `'comment` - The lifetime of the comment lines.
struct UnderfilledLine<'comment> {
    line: &'comment CommentLine,
    next_line: &'comment CommentLine,

    /// The number of the next line's leading tokens in [`Self::moved`].
    num_tokens: usize,

    /// The fewest leading tokens of the next line that make a valid move, as the next line's text
    /// holds them.
    moved: &'comment str,
}

impl<'comment> UnderfilledLine<'comment> {
    /// Applies the fit test to the paragraph line at `next_index` and the line before it: the line
    /// is under-filled if the next line's first `k` tokens, for some `k` of at least 1, would still
    /// end within `width` columns after one space, and what is left of the next line is a valid
    /// continuation. It is valid when it is empty or opens no Markdown block that interrupts a
    /// paragraph, which `preceding` describes, so the move can't turn it into a list item, a
    /// heading or another block.
    ///
    /// The next line's tokens are taken from `tokens`, the paragraph's tokens:
    ///
    /// * A token that fits on the next line alone moves as a whole.
    /// * A wider token is replaced by its whitespace-separated pieces.
    /// * The search ends at a token that fits on a line alone but is split across the next line and
    ///   another, which [`SplitToken`] reports.
    ///
    /// # Returns
    ///
    /// The under-filled line with the fewest tokens that make a valid move, or [`None`] if the line
    /// isn't under-filled.
    fn find(
        lines: &[ParagraphLine<'comment>],
        tokens: &[Token<'_>],
        next_index: usize,
        preceding: Preceding,
        width: Width,
    ) -> Option<Self> {
        let line = lines[next_index - 1].comment_line;
        let next_line = &lines[next_index];
        let room = width.columns().checked_sub(line.width + 1)?;
        tokens
            .iter()
            .skip_while(|token| token.end().line_index < next_index)
            .flat_map(|token| {
                if token.width() <= next_line.capacity {
                    vec![*token]
                } else {
                    token.pieces()
                }
            })
            .skip_while(|unit| unit.end().line_index < next_index)
            .map_while(|unit| {
                (unit.start().line_index == next_index && unit.end().line_index == next_index)
                    .then(|| next_line.text.split_at(unit.end().offset))
            })
            .take_while(|(moved, _)| moved.chars().count() <= room)
            .enumerate()
            .find(|(_, (_, rest))| {
                let rest = rest.trim_start();
                rest.is_empty() || BlockStart::parse(rest, preceding).is_none()
            })
            .map(|(index, (moved, _))| Self {
                line,
                next_line: next_line.comment_line,
                num_tokens: index + 1,
                moved,
            })
    }

    /// # Returns
    ///
    /// Whether the move would make the line exactly `width` columns wide.
    fn fills_width(&self, width: Width) -> bool {
        self.line.width + 1 + self.moved.chars().count() == width.columns()
    }

    /// Builds the line's diagnostic, which names the tokens to move. With `is_wrapped_early`, it
    /// adds that rustfmt wraps the comment one column early, so the paragraph needs rewording.
    ///
    /// The diagnostic points at the line's marker. Its anchor span covers both lines, since a
    /// change to either can create the violation.
    ///
    /// # Returns
    ///
    /// The diagnostic.
    ///
    /// # Panics
    ///
    /// Panics if the next line comes before the line, which [`UnderfilledLine::find`] never
    /// produces.
    fn diagnostic(self, width: Width, is_wrapped_early: bool) -> Diagnostic {
        let Self {
            line,
            next_line,
            num_tokens,
            moved,
        } = self;
        let moved_width = counted(moved.chars().count(), "column");
        let mut message = if num_tokens == 1 {
            format!(
                "line is {} wide and the next line's first word `{moved}` ({moved_width}) fits \
                 within {}; move it up",
                counted(line.width, "column"),
                width.columns()
            )
        } else {
            format!(
                "line is {} wide and the next line's first {num_tokens} words `{moved}` \
                 ({moved_width}) fit within {}; move them up",
                counted(line.width, "column"),
                width.columns()
            )
        };
        if is_wrapped_early {
            message.push_str(
                ", but rustfmt wraps this comment one column early (rust-lang/rustfmt#6180), so \
                 reword the paragraph instead",
            );
        }
        let position = Position {
            line: line.line,
            column: line.marker_column,
        };
        let anchor = LineSpan::new(line.line, next_line.line)
            .expect("an under-filled line's next line follows it");
        Diagnostic::new(RuleId::CommentGreedyWrap.into(), position, message).with_anchor(anchor)
    }
}

/// A token that is split across lines, although it fits on a line alone.
///
/// # Type Parameters
///
/// * `'comment` - The lifetime of the comment lines.
struct SplitToken<'comment> {
    first_line: &'comment CommentLine,
    last_line: &'comment CommentLine,

    /// The kind of the code span or link that the first line break of the token falls in.
    kind: InlineKind,

    /// The token's text, with each line break as one space.
    text: String,

    /// Whether the token is in a list item's first paragraph, which rustfmt fills at whitespace.
    is_in_list_item: bool,
}

impl<'comment> SplitToken<'comment> {
    /// Checks one token of the paragraph with `lines`, which is a list item's first paragraph if
    /// `is_list_item`. The token fits on a line alone when it fits on the line after the one it
    /// starts on.
    ///
    /// # Returns
    ///
    /// The split token, or [`None`] if the token is on one line or is wider than a line.
    fn find(
        token: Token<'_>,
        lines: &[ParagraphLine<'comment>],
        is_list_item: bool,
    ) -> Option<Self> {
        let kind = token.split_kind()?;
        let start_index = token.start().line_index;
        (token.width() <= lines[start_index + 1].capacity).then(|| Self {
            first_line: lines[start_index].comment_line,
            last_line: lines[token.end().line_index].comment_line,
            kind,
            text: token.text().to_owned(),
            is_in_list_item: is_list_item,
        })
    }

    /// Builds the token's diagnostic, which names the token and asks to keep it whole. In a list
    /// item, it adds that rustfmt fills the item at whitespace, so the item needs rewording.
    ///
    /// The diagnostic points at the marker of the line that the token starts on. Its anchor span
    /// covers every line of the token.
    ///
    /// # Returns
    ///
    /// The diagnostic.
    ///
    /// # Panics
    ///
    /// Panics if the token's last line comes before its first line, which [`SplitToken::find`]
    /// never produces.
    fn diagnostic(self, width: Width) -> Diagnostic {
        let Self {
            first_line,
            last_line,
            kind,
            text,
            is_in_list_item,
        } = self;
        let kind = match kind {
            InlineKind::CodeSpan => "code span",
            InlineKind::Link => "link",
        };
        let mut message = format!(
            "{kind} `{text}` ({}) is split across lines but fits within {} on one line; keep it \
             whole",
            counted(text.chars().count(), "column"),
            width.columns()
        );
        if is_in_list_item {
            message.push_str(
                ", but rustfmt fills list items at whitespace, so reword the item instead",
            );
        }
        let position = Position {
            line: first_line.line,
            column: first_line.marker_column,
        };
        let anchor = LineSpan::new(first_line.line, last_line.line)
            .expect("a split token's last line follows its first line");
        Diagnostic::new(RuleId::CommentGreedyWrap.into(), position, message).with_anchor(anchor)
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;
    use std::path::Path;

    use super::*;
    use crate::lexer;
    use crate::syntax::test_code::TestModuleCache;

    fn diagnostics(source_text: &str, columns: usize) -> Vec<Diagnostic> {
        assert_eq!(lexer::lex(source_text).error, None, "{source_text:?}");
        let width = Width::new(NonZeroUsize::new(columns).expect("test widths are non-zero"));
        let test_module_cache = TestModuleCache::default();
        Checker::new(width).check(&FileContext::new(
            Path::new("a.rs"),
            source_text,
            &test_module_cache,
        ))
    }

    #[test]
    fn fit_test_is_exact_at_the_width() {
        // `// aaaa` is 7 columns wide, and ` bbb` brings it to 11.
        let source_text = "// aaaa\n// bbb\n";
        assert_eq!(diagnostics(source_text, 10), []);
        let flagged = diagnostics(source_text, 11);
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0].position(), Position { line: 1, column: 1 });
        assert_eq!(flagged[0].kind(), RuleId::CommentGreedyWrap.into());
        assert_eq!(
            flagged[0].message(),
            "line is 7 columns wide and the next line's first word `bbb` (3 columns) fits within \
             11; move it up"
        );
    }

    #[test]
    fn one_column_first_word_is_singular_in_the_message() {
        let flagged = diagnostics("// a\n// b\n", 100);
        let messages: Vec<_> = flagged.iter().map(Diagnostic::message).collect();
        assert_eq!(
            messages,
            [
                "line is 4 columns wide and the next line's first word `b` (1 column) fits within \
                 100; move it up"
            ]
        );
    }

    #[test]
    fn width_counts_the_whole_source_line_in_scalars() {
        // `    /// éé` is 10 columns wide, and ` x` brings it to 12.
        let source_text = "    /// éé\n    /// x\n";
        assert_eq!(diagnostics(source_text, 11), []);
        let flagged = diagnostics(source_text, 12);
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0].position(), Position { line: 1, column: 5 });
    }

    #[test]
    fn anchor_covers_the_line_and_the_next_line() {
        let flagged = diagnostics("/// a\n/// b\n", 100);
        let anchors: Vec<_> = flagged.iter().map(Diagnostic::anchor).collect();
        assert_eq!(anchors, [LineSpan::new(1, 2).expect("valid span")]);
    }

    #[test]
    fn move_that_would_start_a_list_item_is_not_flagged() {
        // The first line is 97 columns wide, so `n` fits but leaves `- 1` as a list item.
        let source_text = format!("/// {}\n/// n - 1, in order.\n", "a".repeat(93));
        assert_eq!(diagnostics(&source_text, 100), []);
    }

    #[test]
    fn fewest_words_that_leave_a_valid_continuation_are_named() {
        // `// aaaa` is 7 columns wide. Moving `x` would leave `- y` as a list item, so `x -` (3
        // columns) must move together, which brings the line to 11.
        let source_text = "// aaaa\n// x - y z\n";
        assert_eq!(diagnostics(source_text, 10), []);
        let messages: Vec<_> = diagnostics(source_text, 11)
            .iter()
            .map(|diagnostic| diagnostic.message().to_owned())
            .collect();
        assert_eq!(
            messages,
            [
                "line is 7 columns wide and the next line's first 2 words `x -` (3 columns) fit \
                 within 11; move them up"
            ]
        );
    }

    #[test]
    fn continuation_of_a_lazy_paragraph_must_not_open_any_list_item() {
        let source_text = "// 1. An item\n// lazy aaaa\n// x 2. y\n";
        let messages: Vec<_> = diagnostics(source_text, 100)
            .iter()
            .map(|diagnostic| diagnostic.message().to_owned())
            .collect();
        assert_eq!(
            messages,
            [
                "line is 12 columns wide and the next line's first 2 words `x 2.` (4 columns) fit \
                 within 100; move them up"
            ]
        );
    }

    #[test]
    fn continuation_that_opens_no_interrupting_block_is_flagged() {
        for next_line in [
            "// x 2. y",
            "// x * ",
            "// x <span>",
            "// x [a]: b",
            "// x -1",
            "// x",
        ] {
            let source_text = format!("// aaaa\n{next_line}\n");
            let messages: Vec<_> = diagnostics(&source_text, 100)
                .iter()
                .map(|diagnostic| diagnostic.message().to_owned())
                .collect();
            assert_eq!(
                messages,
                [
                    "line is 7 columns wide and the next line's first word `x` (1 column) fits \
                     within 100; move it up"
                ],
                "{next_line:?}"
            );
        }
    }

    #[test]
    fn continuation_that_opens_an_interrupting_block_needs_more_words_moved() {
        for (next_line, moved) in [
            ("// x - y", "x -"),
            ("// x 1. y", "x 1."),
            ("// x # y", "x #"),
            ("// x > y", "x >"),
            ("// x ```", "x ```"),
            ("// x ***", "x ***"),
            ("// x ---", "x ---"),
            ("// x ==", "x =="),
            ("// x <div>", "x <div>"),
            ("// x [^1]: y", "x [^1]:"),
        ] {
            let source_text = format!("// aaaa\n{next_line}\n");
            let messages: Vec<_> = diagnostics(&source_text, 100)
                .iter()
                .map(|diagnostic| diagnostic.message().to_owned())
                .collect();
            assert_eq!(
                messages,
                [format!(
                    "line is 7 columns wide and the next line's first 2 words `{moved}` ({} \
                     columns) fit within 100; move them up",
                    moved.chars().count()
                )],
                "{next_line:?}"
            );
            assert_eq!(diagnostics(&source_text, 9), [], "{next_line:?}");
        }
    }

    /// # Returns
    ///
    /// Whether each diagnostic of `source_text` at `columns` has the rustfmt hint.
    fn rustfmt_hints(source_text: &str, columns: usize) -> Vec<bool> {
        diagnostics(source_text, columns)
            .iter()
            .map(|diagnostic| diagnostic.message().contains("rust-lang/rustfmt#6180"))
            .collect()
    }

    #[test]
    fn move_that_fills_a_field_or_parameter_comment_to_the_width_gets_the_rustfmt_hint() {
        // Each comment line is 17 columns wide, and ` cc` brings it to 20.
        let sources = [
            "struct S {\n    /// aaaa bbbb\n    /// cc dd\n    a: u8,\n}\n",
            "struct S(\n    /// aaaa bbbb\n    /// cc dd\n    u8,\n);\n",
            "union U {\n    // aaaa bbbbb\n    // cc dd\n    a: u8,\n}\n",
            "enum E {\n    V {\n        /// aaaaa\n        /// cc dd\n        a: u8,\n    },\n}\n",
            "enum E {\n    V(\n        /// aaaaa\n        /// cc dd\n        u8,\n    ),\n}\n",
            "fn f(\n    /// aaaa bbbb\n    /// cc dd\n    a: u8,\n) {}\n",
            "trait T {\n    fn f(\n        // aaaaaa\n        // cc dd\n        a: u8,\n    \
             );\n}\n",
        ];
        for source_text in sources {
            assert_eq!(rustfmt_hints(source_text, 20), [true], "{source_text:?}");
            assert_eq!(rustfmt_hints(source_text, 21), [false], "{source_text:?}");
        }
        let messages: Vec<_> = diagnostics(sources[0], 20)
            .iter()
            .map(|diagnostic| diagnostic.message().to_owned())
            .collect();
        assert_eq!(
            messages,
            [
                "line is 17 columns wide and the next line's first word `cc` (2 columns) fits \
                 within 20; move it up, but rustfmt wraps this comment one column early \
                 (rust-lang/rustfmt#6180), so reword the paragraph instead"
            ]
        );
    }

    #[test]
    fn move_that_fills_another_comment_to_the_width_gets_no_rustfmt_hint() {
        let sources = [
            "enum E {\n    /// aaaa bbbb\n    /// cc dd\n    V { a: u8 },\n}\n",
            "/// aaaa bbbb ccc\n/// cc dd\nstruct S {\n    a: u8,\n}\n",
            "fn f() {\n    // aaaa bbbbb\n    // cc dd\n    let s = S {\n        a: 1,\n    \
             };\n}\n",
            "struct S {\n    /// aaaa bbbb\n    /// cc dd\n    a: u8,\n\n",
        ];
        for source_text in sources {
            assert_eq!(rustfmt_hints(source_text, 20), [false], "{source_text:?}");
        }
    }

    #[test]
    fn slashes_in_literals_and_block_comments_are_not_checked() {
        let source_text = "let a = \"\n// a\n// b\n\";\n/*\n// c\n// d\n*/\n";
        assert_eq!(diagnostics(source_text, 100), []);
    }

    /// # Returns
    ///
    /// The message of each diagnostic of `source_text` at `columns`.
    fn messages(source_text: &str, columns: usize) -> Vec<String> {
        diagnostics(source_text, columns)
            .iter()
            .map(|diagnostic| diagnostic.message().to_owned())
            .collect()
    }

    #[test]
    fn link_that_does_not_fit_whole_at_the_end_of_the_line_is_not_flagged() {
        // The first line is 42 columns wide, and the link with its `.` is 63.
        let source_text = "/// [`storage::ReadyTasks`] converted into\n/// [`Result<(SessionId, \
                           Vec<InboundEntry>), StorageClientError>`].\n";
        assert_eq!(diagnostics(source_text, 105), []);
        assert_eq!(
            messages(source_text, 106),
            [
                "line is 42 columns wide and the next line's first word `[`Result<(SessionId, \
                 Vec<InboundEntry>), StorageClientError>`].` (63 columns) fits within 106; move \
                 it up"
            ]
        );
    }

    #[test]
    fn code_span_moves_as_a_whole_with_its_glued_characters() {
        let source_text =
            "/// The connector factory is an\n/// `AsyncFnOnce() -> DbConnectorType` closure.\n";
        assert_eq!(diagnostics(source_text, 65), []);
        assert_eq!(
            messages(source_text, 66),
            [
                "line is 31 columns wide and the next line's first word ``AsyncFnOnce() -> \
                 DbConnectorType`` (34 columns) fits within 66; move it up"
            ]
        );
        let glued = "// aaaa\n// (`b c`). d\n";
        assert_eq!(diagnostics(glued, 15), []);
        assert_eq!(
            messages(glued, 16),
            [
                "line is 7 columns wide and the next line's first word `(`b c`).` (8 columns) \
                 fits within 16; move it up"
            ]
        );
    }

    #[test]
    fn rest_after_a_token_must_open_no_interrupting_block() {
        assert_eq!(
            messages("// aaaa\n// `x y` - z\n", 100),
            [
                "line is 7 columns wide and the next line's first 2 words ``x y` -` (7 columns) \
                 fit within 100; move them up"
            ]
        );
    }

    #[test]
    fn split_token_that_fits_on_one_line_is_reported_instead_of_its_lines() {
        let source_text = "/// [`storage::ReadyTasks`] converted into [`Result<(SessionId, \
                           Vec<InboundEntry>),\n/// StorageClientError>`].\n";
        let flagged = diagnostics(source_text, 100);
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0].position(), Position { line: 1, column: 1 });
        assert_eq!(flagged[0].kind(), RuleId::CommentGreedyWrap.into());
        assert_eq!(
            flagged[0].anchor(),
            LineSpan::new(1, 2).expect("valid span")
        );
        assert_eq!(
            flagged[0].message(),
            "link `[`Result<(SessionId, Vec<InboundEntry>), StorageClientError>`].` (63 columns) \
             is split across lines but fits within 100 on one line; keep it whole"
        );

        let flagged = diagnostics("fn f() {\n    // a `b\n    // c\n    // d` e\n}\n", 100);
        let reported: Vec<_> = flagged
            .iter()
            .map(|diagnostic| {
                (
                    diagnostic.position(),
                    diagnostic.anchor(),
                    diagnostic.message().to_owned(),
                )
            })
            .collect();
        assert_eq!(
            reported,
            [(
                Position { line: 2, column: 5 },
                LineSpan::new(2, 4).expect("valid span"),
                "code span ``b c d`` (7 columns) is split across lines but fits within 100 on one \
                 line; keep it whole"
                    .to_owned()
            )]
        );
    }

    #[test]
    fn split_token_in_a_list_item_gets_the_rustfmt_hint() {
        let hint = ", but rustfmt fills list items at whitespace, so reword the item instead";
        for (source_text, has_hint) in [
            ("/// * a `b\n///   c`\n", true),
            ("// 1. a `b\n//    c`\n", true),
            ("/// * a\n///\n///   b `c\n///   d`\n", false),
            ("/// a `b\n/// c`\n", false),
        ] {
            let messages = messages(source_text, 100);
            assert_eq!(messages.len(), 1, "{source_text:?}");
            assert_eq!(messages[0].ends_with(hint), has_hint, "{source_text:?}");
        }
    }

    #[test]
    fn token_fits_on_a_line_within_the_width_minus_the_line_prefix() {
        // The prefix `    // ` is 7 columns wide, and the code span is 7.
        let source_text = "fn f() {\n    // a `b\n    // c d`\n}\n";
        assert_eq!(
            messages(source_text, 14),
            [
                "code span ``b c d`` (7 columns) is split across lines but fits within 14 on one \
                 line; keep it whole"
            ]
        );
        assert_eq!(
            messages(source_text, 13),
            [
                "line is 11 columns wide and the next line's first word `c` (1 column) fits \
                 within 13; move it up"
            ]
        );
    }

    #[test]
    fn token_wider_than_a_line_is_filled_at_its_spaces() {
        // The code span is 23 columns wide, more than the 17 that fit after `// `.
        let source_text = "// `a b c d e f g\n// h i j k` l\n";
        assert_eq!(
            messages(source_text, 20),
            [
                "line is 17 columns wide and the next line's first word `h` (1 column) fits \
                 within 20; move it up"
            ]
        );
        assert_eq!(
            messages("// a\n// [the docs](https://example.com/a/b/c)\n", 30),
            [
                "line is 4 columns wide and the next line's first word `[the` (4 columns) fits \
                 within 30; move it up"
            ]
        );
    }

    #[test]
    fn piece_without_whitespace_wider_than_a_line_is_not_flagged() {
        for source_text in [
            "// a\n// https://example.com/a/b/c\n",
            "// a\n// [docs](https://example.com/a/b/c)\n",
            "// a\n// `https://example.com/a/b/c` d\n",
            "// https://example.com/a/b/c\n// d\n",
        ] {
            assert_eq!(diagnostics(source_text, 20), [], "{source_text:?}");
        }
    }
}
