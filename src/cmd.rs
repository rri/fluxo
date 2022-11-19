//! Command structures for executing based on user input.

use crate::err::Trace;

/// Command object that represents possible instructions derived from user input.
#[derive(Clone)]
pub enum Cmd {
    /// Exit the integrated development environment.
    Exit,
    /// Fail upon execution.
    Fail(Trace),
    /// Show help information.
    Help,
    /// Perform no operation.
    Noop,
}
