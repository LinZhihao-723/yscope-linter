//! The `tuple-returns-doc` rule: the `# Returns` section of a fn that returns a tuple, or a
//! `Result` of one, introduces the tuple with a fixed sentence and lists one bullet per element.

mod check;

pub use check::Checker;
