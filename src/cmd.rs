//! Command structures for executing based on user input.

use crate::ast::Exp;
use crate::err::TypingErr;

/// Command object that represents possible instructions derived from user input.
#[derive(Clone)]
pub enum Cmd {
    /// Exit the integrated development environment.
    Exit,
    /// Fail with the associated [typing error][TypingErr].
    Fail(TypingErr),
    /// Show help information.
    Help,
    /// Perform no operation.
    Noop,
    /// Show the associated [expression][Exp].
    Show(Exp),
    /// Show the type of the associated expression.
    Type(Exp),
}
