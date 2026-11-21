//! The inline Markdown of a paragraph: code spans, links, and the tokens that filling the paragraph
//! keeps whole.

/// The kind of inline element that a line break can fall in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineKind {
    /// An inline code span, such as `` `a b` ``.
    CodeSpan,

    /// A link, such as `[text](dest)`, `[text][ref]` or ``[`Foo::bar`]``.
    Link,
}

/// The text of one paragraph, with its lines joined by one space each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParagraphText {
    text: String,

    /// The byte offset in `text` where each line starts.
    line_starts: Vec<usize>,
}

impl ParagraphText {
    /// Factory function.
    ///
    /// # Returns
    ///
    /// The text of a paragraph whose lines are `lines`, each without the line's prefix.
    ///
    /// # Panics
    ///
    /// Panics if a line has leading or trailing whitespace.
    pub fn new(lines: &[&str]) -> Self {
        let mut text = String::new();
        let mut line_starts = Vec::new();
        for &line in lines {
            assert_eq!(line, line.trim(), "paragraph lines must be trimmed");
            if !line_starts.is_empty() {
                text.push(' ');
            }
            line_starts.push(text.len());
            text.push_str(line);
        }
        Self { text, line_starts }
    }

    /// Splits the text into tokens. A token is a maximal run of text without whitespace, except
    /// that whitespace inside these doesn't end it:
    ///
    /// * An inline code span: a run of backticks up to the next run of exactly the same length.
    /// * A link: `[text](dest)`, `[text][ref]` or `[text]`. The text's `]` is the one that balances
    ///   its `[`, skipping brackets inside code spans.
    ///
    /// A backslash-escaped backtick or bracket opens neither. A backtick run without a closing run,
    /// or a `[` without a matching `]`, is plain text.
    ///
    /// # Returns
    ///
    /// The tokens, in order.
    pub fn tokens(&self) -> Vec<Token<'_>> {
        let mut tokens = Vec::new();
        let mut index = 0;
        while let Some(start) = self.text[index..]
            .find(|current: char| !current.is_whitespace())
            .map(|skipped| index + skipped)
        {
            let token = self.token_at(start);
            index = token.end;
            tokens.push(token);
        }
        tokens
    }

    /// Scans the token that starts at the non-whitespace character at `start`.
    ///
    /// # Returns
    ///
    /// The token.
    fn token_at(&self, start: usize) -> Token<'_> {
        let mut index = start;
        let mut split_kind = None;
        while let Some(current) = self.text[index..].chars().next()
            && !current.is_whitespace()
        {
            let (kind, end) = match current {
                '\\' => {
                    let escaped = self.text[index + 1..]
                        .chars()
                        .next()
                        .filter(char::is_ascii_punctuation);
                    (None, index + 1 + escaped.map_or(0, char::len_utf8))
                }
                '`' => CodeSpan::parse(&self.text, index).map_or_else(
                    || (None, index + backtick_run_length(&self.text[index..])),
                    |code_span| (Some(InlineKind::CodeSpan), code_span.end),
                ),
                '[' => link_end(&self.text, index)
                    .map_or((None, index + 1), |end| (Some(InlineKind::Link), end)),
                _ => (None, index + current.len_utf8()),
            };
            if split_kind.is_none() && self.spans_lines(index, end) {
                split_kind = kind;
            }
            index = end;
        }
        Token {
            paragraph_text: self,
            start,
            end: index,
            split_kind,
        }
    }

    /// # Returns
    ///
    /// Whether the non-empty byte range from `start` to `end` starts and ends on different lines.
    fn spans_lines(&self, start: usize, end: usize) -> bool {
        self.position(start).line_index != self.end_position(end).line_index
    }

    /// # Returns
    ///
    /// The position of the character at the byte offset `offset` of the text.
    fn position(&self, offset: usize) -> TextPosition {
        let line_index = self
            .line_starts
            .partition_point(|line_start| *line_start <= offset)
            - 1;
        TextPosition {
            line_index,
            offset: offset - self.line_starts[line_index],
        }
    }

    /// # Returns
    ///
    /// The position just past the character that ends at the byte offset `end` of the text, on that
    /// character's line.
    fn end_position(&self, end: usize) -> TextPosition {
        let line_index = self.position(end - 1).line_index;
        TextPosition {
            line_index,
            offset: end - self.line_starts[line_index],
        }
    }
}

/// A position in a [`ParagraphText`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextPosition {
    /// The 0-based index of the paragraph line.
    pub line_index: usize,

    /// The byte offset in that line.
    pub offset: usize,
}

/// A token of a [`ParagraphText`], as [`ParagraphText::tokens`] finds it.
///
/// # Type Parameters
///
/// * `'paragraph` - The lifetime of the paragraph text the token is in.
#[derive(Debug, Clone, Copy)]
pub struct Token<'paragraph> {
    paragraph_text: &'paragraph ParagraphText,
    start: usize,
    end: usize,

    /// The kind of the first code span or link of the token that a line break falls in.
    split_kind: Option<InlineKind>,
}

impl<'paragraph> Token<'paragraph> {
    /// # Returns
    ///
    /// The token's text, with each line break inside it as one space.
    pub fn text(self) -> &'paragraph str {
        &self.paragraph_text.text[self.start..self.end]
    }

    /// # Returns
    ///
    /// The number of Unicode scalar values in [`Self::text`].
    pub fn width(self) -> usize {
        self.text().chars().count()
    }

    /// # Returns
    ///
    /// The position of the token's first character.
    pub fn start(self) -> TextPosition {
        self.paragraph_text.position(self.start)
    }

    /// # Returns
    ///
    /// The position just past the token's last character, on that character's line.
    pub fn end(self) -> TextPosition {
        self.paragraph_text.end_position(self.end)
    }

    /// # Returns
    ///
    /// The kind of the first code span or link of the token that a line break falls in, or [`None`]
    /// if the token is on one line.
    pub const fn split_kind(self) -> Option<InlineKind> {
        self.split_kind
    }

    /// # Returns
    ///
    /// The whitespace-separated pieces of the token, in order. Each piece is on one line.
    pub fn pieces(self) -> Vec<Self> {
        let mut pieces = Vec::new();
        let mut piece_start = None;
        for (offset, current) in self
            .text()
            .char_indices()
            .chain([(self.end - self.start, ' ')])
        {
            if !current.is_whitespace() {
                piece_start.get_or_insert(offset);
            } else if let Some(start) = piece_start.take() {
                pieces.push(Self {
                    paragraph_text: self.paragraph_text,
                    start: self.start + start,
                    end: self.start + offset,
                    split_kind: None,
                });
            }
        }
        pieces
    }
}

/// An inline code span.
///
/// # Type Parameters
///
/// * `'text` - The lifetime of the text the code span is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeSpan<'text> {
    /// The code between the backtick runs, without surrounding whitespace.
    pub code: &'text str,

    /// The byte offset just past the closing backtick run.
    pub end: usize,
}

impl<'text> CodeSpan<'text> {
    /// Parses the code span that opens with the backtick run at the byte offset `start` of `text`.
    ///
    /// # Returns
    ///
    /// The code span, or [`None`] if the run has no closing run of the same length.
    pub fn parse(text: &'text str, start: usize) -> Option<Self> {
        let run_length = backtick_run_length(&text[start..]);
        let mut index = start + run_length;
        while let Some(skipped) = text[index..].find('`') {
            let closing_start = index + skipped;
            let closing_length = backtick_run_length(&text[closing_start..]);
            if closing_length == run_length {
                return Some(Self {
                    code: text[start + run_length..closing_start].trim(),
                    end: closing_start + closing_length,
                });
            }
            index = closing_start + closing_length;
        }
        None
    }
}

/// # Returns
///
/// The byte offset in `text` of the bracket that closes the `open` bracket at `start`, skipping
/// nested pairs, backslash escapes, code spans and unclosed backtick runs, or [`None`] if there is
/// none.
pub fn closing_bracket(text: &str, start: usize, open: u8, close: u8) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0_usize;
    let mut index = start;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'\\' {
            index += 2;
            continue;
        }
        if byte == b'`' {
            index = CodeSpan::parse(text, index).map_or_else(
                || index + backtick_run_length(&text[index..]),
                |code_span| code_span.end,
            );
            continue;
        }
        if byte == open {
            depth += 1;
        } else if byte == close {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
        index += 1;
    }
    None
}

/// Finds the end of the link whose text starts with the `[` at `start`. A `[ref]` after the text is
/// a link of its own, glued to it.
///
/// # Returns
///
/// The byte offset just past the link's `(dest)` if one follows its text, or else just past its
/// text. [`None`] if the text has no closing `]`.
fn link_end(text: &str, start: usize) -> Option<usize> {
    let after_text = closing_bracket(text, start, b'[', b']')? + 1;
    Some(
        text[after_text..]
            .starts_with('(')
            .then(|| closing_bracket(text, after_text, b'(', b')'))
            .flatten()
            .map_or(after_text, |tail_end| tail_end + 1),
    )
}

/// # Returns
///
/// The number of backticks that `text` starts with.
fn backtick_run_length(text: &str) -> usize {
    text.bytes().take_while(|byte| *byte == b'`').count()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// # Returns
    ///
    /// The text of each token of a paragraph with `lines`.
    fn token_texts(lines: &[&str]) -> Vec<String> {
        ParagraphText::new(lines)
            .tokens()
            .into_iter()
            .map(|token| token.text().to_owned())
            .collect()
    }

    #[test]
    fn text_without_inline_elements_splits_at_whitespace() {
        assert_eq!(token_texts(&["a  bb\tc", "d"]), ["a", "bb", "c", "d"]);
        assert_eq!(token_texts(&[""]), Vec::<String>::new());
    }

    #[test]
    fn code_span_and_link_are_one_token_with_their_glued_characters() {
        let text = "x [`TaskInputsSerializer`](spider_core::types::io::TaskInputsSerializer) [the \
                    coding guide](https://example.com), `AsyncFnOnce() -> DbConnectorType`. \
                    ([`Result<(SessionId, Vec<InboundEntry>), StorageClientError>`]). y";
        assert_eq!(
            token_texts(&[text]),
            [
                "x",
                "[`TaskInputsSerializer`](spider_core::types::io::TaskInputsSerializer)",
                "[the coding guide](https://example.com),",
                "`AsyncFnOnce() -> DbConnectorType`.",
                "([`Result<(SessionId, Vec<InboundEntry>), StorageClientError>`]).",
                "y",
            ]
        );
    }

    #[test]
    fn reference_and_shortcut_links_are_one_token() {
        assert_eq!(
            token_texts(&["[the pool][pool] or [the pool][] or [a b] or [^a b]"]),
            [
                "[the pool][pool]",
                "or",
                "[the pool][]",
                "or",
                "[a b]",
                "or",
                "[^a b]"
            ]
        );
        assert_eq!(
            token_texts(&["[a b] [c d]"]),
            ["[a b]", "[c d]"],
            "a link's `[ref]` follows its text directly"
        );
        assert_eq!(
            token_texts(&["[a b](https://a.b \"a title\") c [a b](c d"]),
            ["[a b](https://a.b \"a title\")", "c", "[a b](c", "d"]
        );
        assert_eq!(
            token_texts(&["[the pool][the pool] c"]),
            ["[the pool][the pool]", "c"]
        );
    }

    #[test]
    fn unclosed_backtick_run_and_unmatched_bracket_are_plain_text() {
        assert_eq!(token_texts(&["a `b c"]), ["a", "`b", "c"]);
        assert_eq!(token_texts(&["a ``b` c"]), ["a", "``b`", "c"]);
        assert_eq!(token_texts(&["see [the list"]), ["see", "[the", "list"]);
        assert_eq!(token_texts(&["a ``b` c` d"]), ["a", "``b` c`", "d"]);
    }

    #[test]
    fn code_span_closes_only_on_a_run_of_the_same_length() {
        assert_eq!(
            token_texts(&["``a ` b`` ```c `` d``` e"]),
            ["``a ` b``", "```c `` d```", "e"]
        );
    }

    #[test]
    fn brackets_inside_a_code_span_do_not_count() {
        assert_eq!(
            token_texts(&["[`a] b` c] d `[e f`"]),
            ["[`a] b` c]", "d", "`[e f`"]
        );
        assert_eq!(token_texts(&["[a [b c] d] e"]), ["[a [b c] d]", "e"]);
        assert_eq!(token_texts(&["[a `b] c"]), ["[a `b]", "c"]);
        assert_eq!(token_texts(&["[a ``b] `c` d"]), ["[a ``b]", "`c`", "d"]);
    }

    #[test]
    fn escaped_backtick_or_bracket_opens_nothing() {
        assert_eq!(token_texts(&["\\`a b` c"]), ["\\`a", "b`", "c"]);
        assert_eq!(token_texts(&["\\[a b] c"]), ["\\[a", "b]", "c"]);
        assert_eq!(token_texts(&["[a \\] b] c"]), ["[a \\] b]", "c"]);
        assert_eq!(token_texts(&["a\\ b \\é"]), ["a\\", "b", "\\é"]);
    }

    #[test]
    fn tokens_cross_lines_and_report_what_the_break_falls_in() {
        let paragraph_text = ParagraphText::new(&[
            "converted into [`Result<(SessionId, Vec<InboundEntry>),",
            "StorageClientError>`]. An `AsyncFnOnce() ->",
            "DbConnectorType` closure",
        ]);
        let tokens: Vec<_> = paragraph_text
            .tokens()
            .into_iter()
            .map(|token| (token.text(), token.split_kind(), token.start(), token.end()))
            .collect();
        let position = |line_index, offset| TextPosition { line_index, offset };
        assert_eq!(
            tokens,
            [
                ("converted", None, position(0, 0), position(0, 9)),
                ("into", None, position(0, 10), position(0, 14)),
                (
                    "[`Result<(SessionId, Vec<InboundEntry>), StorageClientError>`].",
                    Some(InlineKind::Link),
                    position(0, 15),
                    position(1, 22),
                ),
                ("An", None, position(1, 23), position(1, 25)),
                (
                    "`AsyncFnOnce() -> DbConnectorType`",
                    Some(InlineKind::CodeSpan),
                    position(1, 26),
                    position(2, 16),
                ),
                ("closure", None, position(2, 17), position(2, 24)),
            ]
        );
    }

    #[test]
    fn token_on_one_line_has_no_split_kind() {
        let paragraph_text = ParagraphText::new(&["`a b` [c d]", "`e`"]);
        let split_kinds: Vec<_> = paragraph_text
            .tokens()
            .into_iter()
            .map(Token::split_kind)
            .collect();
        assert_eq!(split_kinds, [None, None, None]);
    }

    #[test]
    fn width_counts_scalars_and_one_column_per_line_break() {
        let paragraph_text = ParagraphText::new(&["`é", "b`"]);
        let widths: Vec<_> = paragraph_text
            .tokens()
            .into_iter()
            .map(Token::width)
            .collect();
        assert_eq!(widths, [5]);
    }

    #[test]
    fn pieces_split_a_token_at_its_whitespace() {
        let paragraph_text = ParagraphText::new(&["x [a `b", "c` d](e)."]);
        let tokens = paragraph_text.tokens();
        let pieces: Vec<_> = tokens[1]
            .pieces()
            .into_iter()
            .map(|piece| (piece.text(), piece.start(), piece.end(), piece.split_kind()))
            .collect();
        let position = |line_index, offset| TextPosition { line_index, offset };
        assert_eq!(
            pieces,
            [
                ("[a", position(0, 2), position(0, 4), None),
                ("`b", position(0, 5), position(0, 7), None),
                ("c`", position(1, 0), position(1, 2), None),
                ("d](e).", position(1, 3), position(1, 9), None),
            ]
        );
    }
}
