//! Utilities related to error traces and diagnostics.

use std::fmt::{Display, Formatter, Result};

/// Trace object that represents details of a failure to execute.
#[derive(Clone, Default)]
pub struct Trace;

impl Trace {
    /// Create a new instance of this type.
    pub fn new() -> Self {
        Self
    }
}

impl Display for Trace {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "ERROR")
    }
}
