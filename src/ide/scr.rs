//! Screen-specific structures and behaviors within the integrated development environment.

use crate::ide::Prompt;
use crossterm::style::{Color, Stylize};
use crossterm::{execute, queue, terminal};
use std::io::{stdout, Result, Write};

/// Virtual screen that provides the text-based user interface.
pub struct Screen {
    /// Indicates whether or not the screen has been initialized (and may hence require cleanup).
    pub init: bool,
}

impl Screen {
    /// Create a new screen object.
    pub fn new() -> Self {
        Screen { init: false }
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
        self.show_output(
            &format!(
                "{} {}\r\n",
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION")
            ),
            &Prompt::Success,
        )?;
        self.show_output(
            &format!(
                "type {} to exit, {} for assistance\r\n",
                ":quit ↩".with(Color::Red),
                ":help ↩".with(Color::Red)
            ),
            &Prompt::Success,
        )
    }

    /// Show the specified output with the specified prefix.
    fn show_output(&self, output: &str, prompt: &Prompt) -> Result<()> {
        let res = output
            .lines()
            .map(|s| format!("{} {}\r\n", prompt, s.trim_end()))
            .collect::<String>();
        write!(stdout(), "{}", res)
    }

    /// Execute a read-eval-print-loop to accept and process user input.
    fn repl(&self) -> Result<()> {
        Ok(())
    }
}

impl Default for Screen {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        if self.init {
            if let Err(e) = Screen::drop(self) {
                eprintln!("Errors encountered while exiting:");
                eprintln!("{}", e);
            }
        }
    }
}
