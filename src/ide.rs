//! Text-based user interface that serves as an integrated development environment.

use crossterm::style::{Color, StyledContent, Stylize};
use crossterm::{execute, queue, terminal};
use std::io::{stdout, Result};

/// Window that provides a text-based user interface.
///
/// Create a window object and invoke its [run] method to launch a new interactive session. The
/// window object tracks its lifecycle and automatically performs cleanup operation when dropped.
pub struct Window {
    init: bool,
}

impl Window {
    /// Create a new window object.
    pub fn new() -> Self {
        Window { init: false }
    }

    /// Run the integrated development environment and return a result when the user session ends.
    pub fn run(&mut self) -> Result<()> {
        let mut out = stdout();

        self.init()?;
        terminal::enable_raw_mode()?;
        queue!(out, terminal::EnterAlternateScreen)?;
        self.print_banner()?;

        Ok(())
    }

    /// Perform any initialization operations.
    fn init(&mut self) -> Result<()> {
        self.init = true;
        Ok(())
    }

    /// Perform any cleanup operations such as resetting terminal state or restoring buffers.
    fn cleanup(&self) -> Result<()> {
        let mut out = stdout();
        execute!(out, terminal::LeaveAlternateScreen)?;
        terminal::disable_raw_mode()
    }

    /// Print a banner with basic information about the application and brief help on navigation.
    fn print_banner(&self) -> Result<()> {
        print!(
            "{} {} {}\r\n{} type {} to exit, {} for assistance\r\n",
            self.prefix(),
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
            self.prefix(),
            ":quit".with(Color::Red),
            ":help".with(Color::Red)
        );
        Ok(())
    }

    /// Return the prefix to be used on output lines.
    fn prefix(&self) -> StyledContent<&'static str> {
        "∴".with(Color::Green)
    }
}

impl Default for Window {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        if self.init {
            if let Err(e) = self.cleanup() {
                eprintln!("Errors while exiting:");
                eprintln!("{}", e);
            }
        }
    }
}
