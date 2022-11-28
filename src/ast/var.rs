//! Variable in the expression language and related structures.

use std::cmp::Ordering;
use std::fmt::{Display, Formatter, Result};

/// Structure that represents a variable, either symbolic or indexed against a parent binder.
#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub enum VarIdx {
    Var(Var),
    Idx(Idx),
}

/// Structure that represents a symbolic variable.
#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub struct Var(pub String);

/// Structure that represents a variable indexed against a parent binder.
#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub struct Idx(pub usize, pub Var);

impl VarIdx {
    /// Create a new instance that contains a symbolic variable.
    pub fn new_var(var: Var) -> VarIdx {
        VarIdx::Var(var)
    }

    /// Create a new instance that contains an indexed variable.
    pub fn new_idx(idx: Idx) -> VarIdx {
        VarIdx::Idx(idx)
    }

    /// Get the variable contained in this structure.
    pub fn get_var(&self) -> &Var {
        match self {
            Self::Var(var) => var,
            Self::Idx(Idx(_, var)) => var,
        }
    }
}

impl Display for VarIdx {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            Self::Var(var) => var.fmt(f),
            Self::Idx(idx) => idx.fmt(f),
        }
    }
}

impl Var {
    /// Create a new instance of a symbolic variable.
    pub fn new(val: &str) -> Self {
        Var(val.to_string())
    }
}

impl Display for Var {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{}", self.0) // render the variable
    }
}

impl Idx {
    /// Create a new instance of an indexed variable.
    pub fn new(var: &Var) -> Self {
        Idx(0, var.clone())
    }

    /// Increment the index value.
    pub fn inc(&self) -> Self {
        Idx(self.0 + 1, self.1.clone())
    }

    /// Decrement the index value.
    pub fn dec(&self) -> Self {
        Idx(self.0 - 1, self.1.clone())
    }
}

impl Display for Idx {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{}", self.1) // render the variable, indexing is an implementation detail
    }
}

impl PartialOrd for Idx {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.0.partial_cmp(&other.0)
    }
}

impl Ord for Idx {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}
