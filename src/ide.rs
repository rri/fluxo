//! Integrated Development Environment (IDE) and associated structures.

use crate::ast::Ctx;
use crate::cmd::Status;
use crate::edt::Editor;
use crossterm::style::{Color, Stylize};
use crossterm::{execute, queue, terminal};
use std::io::{stdout, Result, Write};

/// Integrated Development Environment (IDE) that provides a text-based user interface.
#[derive(Default)]
pub struct IDE {
    /// Indicates whether the IDE has been initialized (and would hence require cleanup).
    pub init: bool,
}

impl IDE {
    /// Create a new instance.
    pub fn new() -> Self {
        IDE { init: false }
    }

    /// Run the IDE and return a result when the user session ends.
    pub fn run() -> Result<()> {
        let mut ide = Self::new();
        ide.init()?;
        ide.repl()
    }

    /// Perform any initialization operations.
    fn init(&mut self) -> Result<()> {
        self.init = true;

        let mut stdout = stdout();

        terminal::enable_raw_mode()?;
        queue!(
            stdout,
            terminal::EnterAlternateScreen,
            terminal::DisableLineWrap
        )?;
        write!(
            stdout,
            "{}",
            &Status::Content.prefix_to(&format!(
                "{} {}",
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION")
            ))
        )?;
        write!(
            stdout,
            "{}",
            &Status::Content.prefix_to(&format!(
                "type {} for assistance, {} to exit",
                "help ↩".with(Color::Red),
                "quit ↩".with(Color::Red),
            ))
        )?;

        stdout.flush()
    }

    /// Perform any cleanup operations such as resetting terminal state or restoring buffers.
    fn drop(&self) -> Result<()> {
        execute!(stdout(), terminal::LeaveAlternateScreen)?;
        terminal::disable_raw_mode()
    }

    /// Execute a read-eval-print-loop to accept and process user input.
    fn repl(&self) -> Result<()> {
        let mut stdout = stdout();
        let ctx = Ctx::new();
        let edt = Editor::new();

        loop {
            let cmd = edt.read()?;
            let out = cmd.eval(&ctx);

            // Log generated messages to the user's screen.
            for (sts, msg) in out.log {
                write!(stdout, "{}", sts.prefix_to(&msg))?;
            }

            // Terminate the application if requested.
            if out.trm {
                return Ok(());
            }
        }
    }
}

impl Drop for IDE {
    fn drop(&mut self) {
        if self.init {
            if let Err(e) = IDE::drop(self) {
                eprint!("{}", &Status::Failure.prefix_to(&e.to_string()));
            }
        }
    }
}
