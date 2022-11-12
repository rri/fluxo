//! Command structures for executing based on user input.

/// Command object that represents possible instructions derived from user input.
#[derive(Clone)]
pub enum Cmd {
    /// Exit the integrated development environment.
    Exit,
}
