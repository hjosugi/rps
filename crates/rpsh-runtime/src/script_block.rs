use crate::PsString;
use rpsh_syntax::{ParseError, ScriptBlockAst, parse};
use std::{cell::OnceCell, fmt, rc::Rc};

/// A script block retains its defining file and shares parsed code among
/// references to the same block. Independently created blocks have distinct identity.
#[derive(Clone, Debug)]
pub struct ScriptBlock(Rc<Block>);

#[derive(Debug)]
struct Block {
    source: PsString,
    file: Option<PsString>,
    syntax: OnceCell<ScriptBlockAst>,
}

impl ScriptBlock {
    pub fn new(source: PsString, file: Option<PsString>) -> Self {
        Self(Rc::new(Block {
            source,
            file,
            syntax: OnceCell::new(),
        }))
    }

    pub fn source(&self) -> &PsString {
        &self.0.source
    }

    pub fn file(&self) -> Option<&PsString> {
        self.0.file.as_ref()
    }

    pub fn syntax(&self) -> Result<&ScriptBlockAst, ParseError> {
        if let Some(ast) = self.0.syntax.get() {
            return Ok(ast);
        }
        let ast = parse(&self.0.source.to_string_lossy())?;
        Ok(self.0.syntax.get_or_init(|| ast))
    }
}

impl From<&str> for ScriptBlock {
    fn from(source: &str) -> Self {
        Self::new(source.into(), None)
    }
}

impl PartialEq for ScriptBlock {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl fmt::Display for ScriptBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.source().fmt(f)
    }
}
