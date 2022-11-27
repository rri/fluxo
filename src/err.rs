//! Utilities related to error traces and diagnostics.

use crate::ast::{Exp, Var};
use std::error::Error;
use std::fmt::{Display, Formatter, Result};

/// Top-level error that represents a failure to type-check the program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypingErr {
    Generic(String),
    TypeCompatErr(TypeCompatErr),
    TypeUndefErr(TypeUndefErr),
    TypeUnknownErr(TypeUnknownErr),
    TypeRedeclErr(TypeRedeclErr),
}

impl From<TypeCompatErr> for TypingErr {
    fn from(e: TypeCompatErr) -> Self {
        TypingErr::TypeCompatErr(e)
    }
}

impl From<TypeUndefErr> for TypingErr {
    fn from(e: TypeUndefErr) -> Self {
        TypingErr::TypeUndefErr(e)
    }
}

impl From<TypeUnknownErr> for TypingErr {
    fn from(e: TypeUnknownErr) -> Self {
        TypingErr::TypeUnknownErr(e)
    }
}

impl From<TypeRedeclErr> for TypingErr {
    fn from(e: TypeRedeclErr) -> Self {
        TypingErr::TypeRedeclErr(e)
    }
}

/// Error that indicates that a expression has an unexpected type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeCompatErr {
    /// Expression that has an unexpected type.
    pub exp: Exp,
    /// Actual calculated type of the expression.
    pub typ: Exp,
    /// Expected type(s) of the expression (if known).
    pub acc: Vec<Exp>,
    /// Message explaining the compatibility error.
    pub msg: String,
}

/// Error that indicates that a expression doesn't have a well-defined type within the system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeUndefErr {
    /// Expression that has an undefined type.
    pub exp: Exp,
}

/// Error that indicates that a variable has no declared or inferred type in the current context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeUnknownErr {
    /// Variable whose type is not known.
    pub var: Var,
}

/// Error that indicates that a variable has a different previously declared or inferred type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeRedeclErr {
    /// Variable whose type is being re-declared.
    pub var: Var,
    /// Previously declared or inferred type of the variable.
    pub typ: Exp,
    /// Newly declared type of the variable.
    pub upd: Exp,
}

impl Error for TypeCompatErr {}

impl TypeCompatErr {
    pub fn new(exp: &Exp, typ: &Exp, acc: &[&Exp]) -> Self {
        TypeCompatErr {
            exp: exp.clone(),
            typ: typ.clone(),
            acc: acc.iter().copied().cloned().collect(),
            msg: format!(":type {} does not have the requisite form!", exp),
        }
    }
}

impl Display for TypeCompatErr {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        if self.acc.is_empty() {
            writeln!(f, "{}", self.msg)
        } else {
            writeln!(f, ":type {}", self.exp)?;
            writeln!(f, "    = {}", self.typ)?;
            writeln!(
                f,
                "    ∉ {{{}}}",
                self.acc
                    .iter()
                    .map(Exp::to_string)
                    .intersperse(", ".to_string())
                    .collect::<String>()
            )
        }
    }
}

impl Error for TypeUndefErr {}

impl TypeUndefErr {
    pub fn new(exp: &Exp) -> Self {
        TypeUndefErr { exp: exp.clone() }
    }
}

impl Display for TypeUndefErr {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        writeln!(f, ":type {}", self.exp)?;
        writeln!(f, "    = ⊥")?;
        Ok(())
    }
}

impl Error for TypeUnknownErr {}

impl TypeUnknownErr {
    pub fn new(var: &Var) -> Self {
        TypeUnknownErr { var: var.clone() }
    }
}

impl Display for TypeUnknownErr {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        writeln!(f, ":type {} = ?", self.var)?;
        Ok(())
    }
}

impl Error for TypeRedeclErr {}

impl TypeRedeclErr {
    pub fn new(var: &Var, typ: &Exp, upd: &Exp) -> Self {
        TypeRedeclErr {
            var: var.clone(),
            typ: typ.clone(),
            upd: upd.clone(),
        }
    }
}

impl Display for TypeRedeclErr {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        writeln!(f, ":type {}", self.var)?;
        writeln!(f, "    = {}", self.typ)?;
        writeln!(f, "    ≠ {}", self.upd)?;
        Ok(())
    }
}

impl Error for TypingErr {}

impl Default for TypingErr {
    fn default() -> Self {
        Self::Generic("generic typing error".to_string())
    }
}

impl Display for TypingErr {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            Self::Generic(s) => write!(f, "{}", s),
            Self::TypeCompatErr(e) => write!(f, "{}", e),
            Self::TypeUndefErr(e) => write!(f, "{}", e),
            Self::TypeUnknownErr(e) => write!(f, "{}", e),
            Self::TypeRedeclErr(e) => write!(f, "{}", e),
        }
    }
}
