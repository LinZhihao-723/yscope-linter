//! The lines of the statements in a `syn` syntax tree.

use quote::ToTokens;
use syn::visit::Visit;

use crate::diagnostic::Position;
use crate::syntax::item_lines;
use crate::syntax::item_lines::ItemLines;
use crate::syntax::span;

/// The lines of one statement, and where it starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatementLines {
    pub lines: ItemLines,

    /// The position of the statement's first token, which is the `#` of its first outer attribute
    /// if it has one.
    pub start: Position,
}

/// Collects the lines of every statement of `syntax_tree`, including the statements nested in
/// blocks, closures and the bodies of nested items. These count as statements:
///
/// * `let` statements.
/// * Expression statements, with or without a `;`, including a block's tail expression.
/// * Macro statements.
///
/// An item in a block, such as a nested fn, isn't a statement.
///
/// # Returns
///
/// The lines of the statements, in source order.
pub fn collect(syntax_tree: &syn::File) -> Vec<StatementLines> {
    let mut statement_collector = StatementCollector {
        statement_lines: Vec::new(),
    };
    statement_collector.visit_file(syntax_tree);
    statement_collector.statement_lines
}

/// The `syn` visitor that collects the [`StatementLines`] of statements.
struct StatementCollector {
    statement_lines: Vec<StatementLines>,
}

impl<'ast> Visit<'ast> for StatementCollector {
    fn visit_stmt(&mut self, stmt: &'ast syn::Stmt) {
        let is_counted = match stmt {
            syn::Stmt::Local(_) | syn::Stmt::Expr(..) | syn::Stmt::Macro(_) => true,
            syn::Stmt::Item(_) => false,
        };
        if is_counted
            && let Some(lines) = item_lines::of(stmt)
            && let Some(first_token) = stmt.to_token_stream().into_iter().next()
        {
            self.statement_lines.push(StatementLines {
                lines,
                start: span::start_position(first_token.span()),
            });
        }
        syn::visit::visit_stmt(self, stmt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(source_text: &str) -> Vec<(String, String, (usize, usize))> {
        let syntax_tree = syn::parse_file(source_text).expect("valid file");
        collect(&syntax_tree)
            .into_iter()
            .map(|statement_lines| {
                (
                    statement_lines.lines.span.to_string(),
                    statement_lines.lines.header.to_string(),
                    (statement_lines.start.line, statement_lines.start.column),
                )
            })
            .collect()
    }

    #[test]
    fn statements_of_every_kind_are_collected_with_nested_ones() {
        let source_text = concat!(
            "fn f() {\n",
            "    let a = 1;\n",
            "    g(\n",
            "        a,\n",
            "    )?;\n",
            "    m!(\n",
            "        x\n",
            "    );\n",
            "    let c = || {\n",
            "        h()?;\n",
            "        Ok(())\n",
            "    };\n",
            "    fn nested() {}\n",
            "    Ok(())\n",
            "}\n",
        );
        let spans: Vec<_> = lines(source_text)
            .into_iter()
            .map(|(span, ..)| span)
            .collect();
        assert_eq!(spans, ["2", "3-5", "6-8", "9-12", "10", "11", "14"]);
    }

    #[test]
    fn headers_and_starts_are_at_the_first_attribute() {
        let source_text = "fn f() {\n    #[allow(unused)]\n    let w = c(\n        1,\n    );\n}\n";
        assert_eq!(
            lines(source_text),
            [("2-5".to_owned(), "2-3".to_owned(), (2, 5))]
        );
        assert_eq!(
            lines("fn f() { let a = 1; }\n"),
            [("1".to_owned(), "1".to_owned(), (1, 10))]
        );
    }
}
