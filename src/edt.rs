//! Editor and related utilities.

use crate::ast::{Ctx, Exp};
use crate::buf::Buf;
use crate::cmd::Status;
use crossterm::style::{Color, Stylize};
use crossterm::terminal::{Clear, ClearType};
use crossterm::{cursor, execute, queue};
use std::io::{stdout, Result, Write};
use unicode_width::UnicodeWidthStr;

/// Default width of help key column.
const DEF_KEY_WIDTH: usize = 10;

/// Editor that accepts single-line and multi-line structured user input.
#[derive(Default)]
pub struct Editor;

/// Command object that represents possible editor instructions derived from user input.
#[derive(Clone, Eq, PartialEq)]
pub enum Cmd {
    /// Perform no operation.
    Noop,
    /// Exit the application.
    Exit,
    /// Show help information, either general or specific to the associated command.
    Help(Option<Box<Cmd>>),
    /// Show the normalized form of the associated [expression][Exp].
    Show(Exp),
    /// Calculate the type of the associated [expression][Exp].
    Type(Exp),
    /// Execute the program denoted by the associated expression.
    Exec(Exp),
}

/// Output generated by the evaluation of a [command][Cmd].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Out {
    /// Log of messages generated.
    pub log: Vec<(Status, String)>,
    /// Flag that signals the parent process to terminate.
    pub trm: bool,
}

impl Editor {
    /// Create a new instance.
    pub fn new() -> Self {
        Self
    }

    /// Read structured input into a [command][Cmd] and return it.
    pub fn read(&self) -> Result<Cmd> {
        let mut stdout = stdout();
        let mut buf = Buf::new();

        execute!(stdout, cursor::SavePosition)?;

        // Initially render an empty buffer to the screen.
        self.render(&buf, false)?;

        loop {
            let inp = buf.read()?;
            let out = inp.eval(&mut buf);

            // Refresh the rendering of the buffer on the terminal.
            self.render(&buf, out.trm)?;

            // Terminate the loop if requested.
            if out.trm {
                return Ok(buf.value());
            }
        }
    }

    /// Render the buffer onto the screen.
    fn render(&self, buf: &Buf, trm: bool) -> Result<()> {
        let mut stdout = stdout();

        let out = buf.render();
        let (col_idx, row_idx) = buf.cursor();

        queue!(
            stdout,
            cursor::RestorePosition,
            Clear(ClearType::FromCursorDown)
        )?;

        write!(stdout, "{}", out)?;

        // Move the cursor to the correct location relative to the editor.
        // TODO: Keep the text visible when on the right-most (visible) column.
        // TODO: Fix the rendering when on the last (visible) row.
        queue!(stdout, cursor::RestorePosition)?;

        if col_idx > 0 {
            // 0 is treated as 1 by most terminals, hence the 'if' condition.
            queue!(stdout, cursor::MoveRight(col_idx as u16))?;
        }
        if row_idx > 0 {
            // 0 is treated as 1 by most terminals, hence the 'if' condition.
            queue!(stdout, cursor::MoveDown(row_idx as u16))?;
        }

        // Write a final newline if terminating.
        if trm {
            write!(stdout, "\r\n")?;
        }

        stdout.flush()
    }
}

impl Cmd {
    /// Evaluate the command and return the generated [output][Out].
    pub fn eval(self, ctx: &Ctx) -> Out {
        match self {
            Cmd::Noop => Out::new(),
            Cmd::Exit => Out::trm(),
            Cmd::Help(cmd) => {
                let mut msg = String::new();

                msg.push_str("COMMAND REFERENCE:\n");

                let commands = vec![
                    Cmd::Help(None),
                    Cmd::Exit,
                    Cmd::Show(Default::default()),
                    Cmd::Type(Default::default()),
                    Cmd::Exec(Default::default()),
                ];

                let targets: Vec<&Cmd> = commands
                    .iter()
                    .filter(|tgt| {
                        if let Some(val) = &cmd {
                            **tgt == **val
                        } else {
                            true
                        }
                    })
                    .collect();

                let max = targets
                    .iter()
                    .flat_map(|tgt| tgt.help())
                    .map(|(key, _)| key.width())
                    .max()
                    .unwrap_or(DEF_KEY_WIDTH);

                for tgt in targets {
                    tgt.help().iter().for_each(|(key, val)| {
                        msg.push_str(&format!(
                            "‣ {} {}.... {}\r\n",
                            key.split_once(' ')
                                .map(|(tgt, args)| format!("{} {}", tgt.with(Color::Red), args))
                                .unwrap_or_else(|| format!("{}", key.with(Color::Red))),
                            ".".repeat(max - key.width()),
                            val
                        ))
                    });
                }

                Out::msg(Status::Content, &msg)
            }
            Cmd::Show(exp) => match exp.reduce(ctx) {
                Ok(exp) => Out::msg(Status::Success, &exp.to_string()),
                Err(ex) => Out::msg(Status::Failure, &ex.to_string()),
            },
            Cmd::Type(exp) => match exp.calculate_type(ctx) {
                Ok(exp) => Out::msg(Status::Success, &exp.to_string()),
                Err(ex) => Out::msg(Status::Failure, &ex.to_string()),
            },
            Cmd::Exec(exp) => match exp.reduce(ctx) {
                // TODO: Implement expression execution.
                Ok(exp) => Out::msg(Status::Content, &exp.to_string()),
                Err(ex) => Out::msg(Status::Failure, &ex.to_string()),
            },
        }
    }

    /// Fetch help information for the command.
    pub fn help(&self) -> Vec<(&'static str, &'static str)> {
        let mut res = vec![];
        match self {
            Cmd::Noop => {
                res.push(("noop", "Do nothing (not user-invocable)"));
            }
            Cmd::Exit => {
                res.push(("exit", "Exit the integrated development environment"));
                res.push(("quit", "Alias for “exit”"));
            }
            Cmd::Help(_) => {
                res.push(("help", "Print this help message"));
            }
            Cmd::Show(_) => {
                res.push(("show EXP", "Show the normalized form of the expression EXP"));
            }
            Cmd::Type(_) => {
                res.push(("type EXP", "Show the type of the expression EXP"));
            }
            Cmd::Exec(_) => {
                res.push(("exec EXP", "Execute the program denoted by the expression"));
            }
        }
        res
    }
}

impl Out {
    /// Create a new instance.
    pub fn new() -> Self {
        Self {
            log: vec![],
            trm: false,
        }
    }

    /// Create a new instance with the 'trm' flag set to true.
    pub fn trm() -> Self {
        Self {
            log: vec![],
            trm: true,
        }
    }

    /// Create a new instance with the given status and message.
    pub fn msg(sts: Status, val: &str) -> Self {
        let mut res = Self::new();
        res.append(sts, val);
        res
    }

    /// Append a log message to an instance.
    pub fn append(&mut self, sts: Status, val: &str) {
        self.log.push((sts, val.to_string()));
    }
}
