//! The lines of the statements in a `syn` syntax tree.

use syn::visit::Visit;

use crate::syntax::item_lines;
use crate::syntax::item_lines::ItemLines;

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
pub fn collect(syntax_tree: &syn::File) -> Vec<ItemLines> {
    let mut statement_collector = StatementCollector {
        statement_lines: Vec::new(),
    };
    statement_collector.visit_file(syntax_tree);
    statement_collector.statement_lines
}

/// The `syn` visitor that collects the [`ItemLines`] of statements.
struct StatementCollector {
    statement_lines: Vec<ItemLines>,
}

impl<'ast> Visit<'ast> for StatementCollector {
    fn visit_stmt(&mut self, stmt: &'ast syn::Stmt) {
        let is_counted = match stmt {
            syn::Stmt::Local(_) | syn::Stmt::Expr(..) | syn::Stmt::Macro(_) => true,
            syn::Stmt::Item(_) => false,
        };
        if is_counted && let Some(statement_lines) = item_lines::of(stmt) {
            self.statement_lines.push(statement_lines);
        }
        syn::visit::visit_stmt(self, stmt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(source_text: &str) -> Vec<(String, String)> {
        let syntax_tree = syn::parse_file(source_text).expect("valid file");
        collect(&syntax_tree)
            .into_iter()
            .map(|statement_lines| {
                (
                    statement_lines.span.to_string(),
                    statement_lines.header.to_string(),
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
            .map(|(span, _)| span)
            .collect();
        assert_eq!(spans, ["2", "3-5", "6-8", "9-12", "10", "11", "14"]);
    }

    #[test]
    fn headers_run_from_the_first_attribute_to_the_statement() {
        let source_text = "fn f() {\n    #[allow(unused)]\n    let w = c(\n        1,\n    );\n}\n";
        assert_eq!(lines(source_text), [("2-5".to_owned(), "2-3".to_owned())]);
    }
}
