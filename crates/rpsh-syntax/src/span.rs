/// Half-open byte range `[start, end)` into the script text.
///
/// Upstream counterpart: the offsets of `IScriptExtent`. Line and column are
/// derived from the source on demand instead of being stored per node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    /// The tokenizer rejects sources longer than `u32::MAX` bytes, so offsets
    /// produced by it always fit.
    pub(crate) fn new(start: usize, end: usize) -> Self {
        Self {
            start: start as u32,
            end: end as u32,
        }
    }

    /// The span from the start of `self` to the end of `other`.
    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start,
            end: other.end,
        }
    }

    /// The source text this span covers.
    pub fn text(self, source: &str) -> &str {
        &source[self.start as usize..self.end as usize]
    }
}
