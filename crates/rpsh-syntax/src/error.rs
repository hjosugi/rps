use std::fmt;

use crate::Span;

/// Why a script could not be turned into an AST.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The input is, or may be, valid PowerShell that rpsh does not parse yet.
    ///
    /// `feature` is a stable label: the harness counts refusals by it to show
    /// which missing feature blocks the most test and benchmark cases.
    ///
    /// Real syntax errors are reported this way too for now. Reproducing
    /// upstream's `ParseError` ids and messages is its own piece of work.
    NotImplemented { feature: &'static str, span: Span },
}

impl ParseError {
    pub(crate) fn not_implemented(feature: &'static str, span: Span) -> Self {
        Self::NotImplemented { feature, span }
    }

    /// The label of the missing feature.
    pub fn feature(&self) -> &'static str {
        match self {
            Self::NotImplemented { feature, .. } => feature,
        }
    }

    /// Where in the source the unsupported input starts and ends.
    pub fn span(&self) -> Span {
        match self {
            Self::NotImplemented { span, .. } => *span,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotImplemented { feature, span } => {
                write!(f, "{feature} (at offset {})", span.start)
            }
        }
    }
}

impl std::error::Error for ParseError {}
