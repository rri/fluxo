//! Window-specific structures and behaviors within the integrated development environment.

use crossterm::style::{Color, StyledContent, Stylize};
use crossterm::{execute, queue, terminal};
use std::fmt::Display;
use std::io::{stdout, Result};

/// Window that provides the text-based user interface.
pub struct Window {
    /// Indicates whether or not the window has been initialized (and may hence require cleanup).
    pub init: bool,
}

/// Types of prompts that may be rendered to the user under various circumstances.
pub enum Prompt {
    /// System is ready for fresh input.
    Ready(),
    /// System is ready for input continuing the previously entered input (which was incomplete).
    ReadyToContinue(),
    /// System has generated the output that follows the prompt.
    OutputPrefix(),
    /// System has generated the error message that follows the prompt.
    FailedPrefix(),
}

impl Prompt {
    /// Render the prompt as styled content (such as a colored string).
    fn as_styled_content(&self) -> StyledContent<&'static str> {
        match self {
            Prompt::Ready() => "»".with(Color::Cyan),
            Prompt::ReadyToContinue() => "↳".with(Color::Cyan),
            Prompt::OutputPrefix() => "∴".with(Color::DarkGreen),
            Prompt::FailedPrefix() => "✗".with(Color::Red),
        }
    }
}

impl Display for Prompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_styled_content())
    }
}

impl Window {
    /// Create a new window object.
    pub fn new() -> Self {
        Window { init: false }
    }

    /// Run the integrated development environment and return a result when the user session ends.
    pub fn run(&mut self) -> Result<()> {
        self.init()?;
        self.repl()
    }

    /// Perform any initialization operations.
    fn init(&mut self) -> Result<()> {
        self.init = true;
        terminal::enable_raw_mode()?;
        queue!(stdout(), terminal::EnterAlternateScreen)?;
        self.show_banner()
    }

    /// Perform any cleanup operations such as resetting terminal state or restoring buffers.
    fn drop(&self) -> Result<()> {
        execute!(stdout(), terminal::LeaveAlternateScreen)?;
        terminal::disable_raw_mode()
    }

    /// Show a banner with basic information about the application and brief help on navigation.
    fn show_banner(&self) -> Result<()> {
        print!(
            "{} {} {}\r\n",
            Prompt::OutputPrefix(),
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION")
        );
        print!(
            "{} type {} ↩ to exit, {} ↩ for assistance\r\n",
            Prompt::OutputPrefix(),
            ":quit".with(Color::Red),
            ":help".with(Color::Red)
        );
        Ok(())
    }

    /// Execute a read-eval-print-loop to accept and process user input.
    fn repl(&self) -> Result<()> {
        Ok(())
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
            if let Err(e) = Window::drop(self) {
                eprintln!("Errors encountered while exiting:");
                eprintln!("{}", e);
            }
        }
    }
}
