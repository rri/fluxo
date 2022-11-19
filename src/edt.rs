//! Editor-specific structures and behaviors within the integrated development environment.

use crate::pmt::Prompt;
use crossterm::style::{Color, Stylize};
use crossterm::{execute, queue, terminal};
use std::io::{stdout, Result, Write};

/// Virtual editor that provides the text-based user interface.
pub struct Editor {
    /// Indicates whether or not the editor has been initialized (and may hence require cleanup).
    pub init: bool,
}

impl Editor {
    /// Create a new editor object.
    pub fn new() -> Self {
        Editor { init: false }
    }

    /// Run the integrated development environment and return a result when the user session ends.
    pub fn run() -> Result<()> {
        let mut edt = Self::new();
        edt.init()?;
        edt.repl()
    }

    /// Perform any initialization operations.
    fn init(&mut self) -> Result<()> {
        self.init = true;
        terminal::enable_raw_mode()?;
        queue!(
            stdout(),
            terminal::EnterAlternateScreen,
            terminal::DisableLineWrap
        )?;
        self.show_banner()
    }

    /// Perform any cleanup operations such as resetting terminal state or restoring buffers.
    fn drop(&self) -> Result<()> {
        execute!(stdout(), terminal::LeaveAlternateScreen)?;
        terminal::disable_raw_mode()
    }

    /// Show a banner with basic information about the application and brief help on navigation.
    fn show_banner(&self) -> Result<()> {
        let mut stdout = stdout();
        write!(
            stdout,
            "{}",
            Prompt::Success.prefix_to(&format!(
                "{} {}\r\n",
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION")
            ),)
        )?;
        write!(
            stdout,
            "{}",
            Prompt::Success.prefix_to(&format!(
                "type {} to exit, {} for assistance\r\n",
                ":quit ↩".with(Color::Red),
                ":help ↩".with(Color::Red)
            ),)
        )
    }

    /// Execute a read-eval-print-loop to accept and process user input.
    fn repl(&self) -> Result<()> {
        Ok(())
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        if self.init {
            if let Err(e) = Editor::drop(self) {
                eprint!("{}", Prompt::Failure.prefix_to("I/O error:"));
                eprint!("{}", Prompt::Diagnostics.prefix_to(&format!("{}", e)));
            }
        }
    }
}
