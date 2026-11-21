//! The Markdown that the rules read comment and doc lines by: the blocks that lines start, and the
//! inline tokens of a paragraph.

mod block_start;
mod inline;
mod line;

pub use block_start::BlockStart;
pub use block_start::HtmlBlockCondition;
pub use block_start::Preceding;
pub use inline::CodeSpan;
pub use inline::InlineKind;
pub use inline::ParagraphText;
pub use inline::Token;
pub use inline::closing_bracket;
pub use line::Fence;
pub use line::FenceTracker;
pub use line::atx_header;
pub use line::bullet_marker;
pub use line::indent;
pub use line::link_definition;
