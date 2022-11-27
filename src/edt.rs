//! Editor and related utilities.

use crate::buf::Buf;
use crate::cmd::Cmd;
use crossterm::event::{Event, KeyCode};
use crossterm::{cursor, event, execute, queue};
use std::io::{stdout, Result, Write};

/// Editor that accepts single-line and multi-line structured user input.
#[derive(Default)]
pub struct Editor;

impl Editor {
    /// Create a new instance of editor.
    pub fn new() -> Self {
        Self
    }

    /// Read input into a [command][Cmd] and return it.
    pub fn read(&self) -> Result<Cmd> {
        let mut stdout = stdout();
        execute!(stdout, cursor::SavePosition)?;

        let mut buf = Buf::new();

        loop {
            self.show(&buf)?;
            if let Event::Key(evt) = event::read()? {
                match evt.code {
                    KeyCode::Char(chr) => {
                        buf.push(chr);
                        self.show(&buf)?;
                    }
                    KeyCode::Enter => {
                        break; // Exit the loop and return the finalized command.
                    }
                    KeyCode::Esc => todo!(),
                    KeyCode::Backspace => todo!(),
                    KeyCode::Delete => todo!(),
                    KeyCode::Left => todo!(),
                    KeyCode::Right => todo!(),
                    KeyCode::Up => todo!(),
                    KeyCode::Down => todo!(),
                    KeyCode::Home => todo!(),
                    KeyCode::End => todo!(),
                    KeyCode::PageUp => todo!(),
                    KeyCode::PageDown => todo!(),
                    KeyCode::Tab => todo!(),
                    KeyCode::BackTab => todo!(),

                    // Ignore all remaining keys
                    KeyCode::Insert => continue,
                    KeyCode::F(_) => continue,
                    KeyCode::Null => continue,
                    KeyCode::CapsLock => continue,
                    KeyCode::ScrollLock => continue,
                    KeyCode::NumLock => continue,
                    KeyCode::PrintScreen => continue,
                    KeyCode::Pause => continue,
                    KeyCode::Menu => continue,
                    KeyCode::KeypadBegin => continue,
                    KeyCode::Media(_) => continue,
                    KeyCode::Modifier(_) => continue,
                }
            }
        }

        // This is a parser stub. In a real implementation, the buffer's raw string should never
        // be accessed directly, and the buffer needs to be smarter in knowing how to render parsed
        // input.

        write!(stdout, "\r\n")?;

        let res = buf.raw.trim_end();

        if res.is_empty() {
            Ok(Cmd::Noop)
        } else if res == "quit" || res == "exit" {
            Ok(Cmd::Exit)
        } else if res == "help" {
            Ok(Cmd::Help(None))
        } else {
            Ok(Cmd::Fail(Default::default()))
        }
    }

    /// Show the editor's updated buffer on the screen.
    fn show(&self, buf: &Buf) -> Result<()> {
        let mut stdout = stdout();
        queue!(stdout, cursor::RestorePosition)?;
        write!(stdout, "{}", buf.render())?;
        stdout.flush()
    }
}
