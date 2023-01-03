//! Smart buffer and related utilities for processing and parsing structured user input.

use crate::edt::Cmd;
use crate::key::{DefKeyMap, EscKeyMap, FixKeyMap, KeyMap};
use crossterm::event;
use crossterm::event::Event;
use crossterm::style::{Color, Stylize};
use std::fmt::{Display, Formatter};
use std::io::Result;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Escape character.
pub const ESC: char = '\\';

/// Gutter width.
pub const GUTTER_WIDTH: usize = 3;

/// Prompt rendered when input is being accepted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Prompt {
    /// Prompt representing ready for input.
    Ready,
    /// Prompt representing continued input.
    Contd,
}

/// Smart buffer for processing and parsing structured user input.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Buf {
    /// Raw string backing this buffer.
    pub raw: String,
    /// Index of cursor associated with the raw string.
    pub idx: usize,
    /// Represents the state of the buffer being in 'escape mode' which calls for special behavior.
    pub esc: bool,
    /// Indicates whether the buffer is in a terminal state.
    pub trm: bool,
}

/// Command object that represents possible buffer inputs derived from user input.
#[derive(Clone, Eq, PartialEq)]
pub enum Inp {
    /// Perform no operation.
    Noop,
    /// Exit the current read loop.
    Exit,
    /// Set 'escape mode' (which calls for special behavior for the next key pressed).
    Esc(bool),
    /// Perform the given commands in sequence, executing the second only if the first succeeds.
    Compose(Box<Inp>, Box<Inp>),
    /// Repeat the given command multiple times.
    Repeat(Box<Inp>, usize),
    /// Push a character into the buffer (if the buffer isn't full).
    Push(char),
    /// Delete the character immediately **after** the cursor (if there is one).
    Delete,
    /// Clear the buffer.
    Clear,
    /// Move the cursor a row up (if possible).
    MoveUp,
    /// Move the cursor a row down (if possible).
    MoveDn,
    /// Move the cursor a column left (if possible).
    MoveLt,
    /// Move the cursor a column right (if possible).
    MoveRt,
}

impl Prompt {
    /// Prefix styled prompts to the given buffer value (even if the value is empty).
    pub fn prefix_to(val: &str) -> String {
        format!(
            "{}{}{}",
            &Prompt::Ready,
            " ".repeat(GUTTER_WIDTH - 1),
            val.replace(
                '\n',
                &format!("\r\n{}{}", Prompt::Contd, " ".repeat(GUTTER_WIDTH - 1),)
            )
        )
    }
}

impl Display for Prompt {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Prompt::Ready => write!(f, "{}", "»".with(Color::Cyan)),
            Prompt::Contd => write!(f, "{}", "↳".with(Color::Cyan)),
        }
    }
}

impl Buf {
    /// Create a new instance.
    pub fn new() -> Self {
        Self {
            raw: String::new(),
            idx: 0,
            esc: false,
            trm: false,
        }
    }

    /// Read structured input into a [buffer input][Inp] and return it.
    pub fn read(&mut self) -> Result<Inp> {
        // TODO: Add more keymaps.
        if let Event::Key(evt) = event::read()? {
            Ok(Option::None
                .or_else(|| EscKeyMap::convert(self, &evt))
                .or_else(|| FixKeyMap::convert(self, &evt))
                .or_else(|| DefKeyMap::convert(self, &evt))
                .unwrap_or(Inp::Noop))
        } else {
            Ok(Inp::Noop)
        }
    }

    /// Return a displayable string rendering the contents of the buffer.
    pub fn render(&self) -> String {
        // TODO: Render the sequence with the highest level of meaning.
        let mut res = self.raw.clone();
        if self.esc {
            let (lt, rt) = res.split_at(self.idx);
            res = format!("{}{}{}", lt, ESC.with(Color::Cyan), rt);
        }
        Prompt::prefix_to(&res)
    }

    /// Return the current cursor location.
    ///
    /// The cursor location is in the format '(col_idx, row_idx)', where:
    ///
    /// * 'col_idx' is the index of the column (starting from 0).
    /// * 'row_idx' is the index of the row (starting from 0).
    /// * Rows and columns are relative to the location of the editor.
    pub fn cursor(&self) -> (/* col_idx */ usize, /* row_idx */ usize) {
        let mut col_idx = 0;
        let mut row_idx = 0;
        let mut cur = false;
        let mut cnt = 0;

        for s in self.raw.graphemes(true) {
            if cnt == self.idx {
                cur = true;
            } else {
                cnt += s.len();
                if !cur {
                    // Update the cursor location unless already finalized.
                    if s == "\n" {
                        row_idx += 1;
                        col_idx = 0;
                    } else {
                        col_idx += s.width();
                    }
                }
            }
        }

        (col_idx + GUTTER_WIDTH, row_idx)
    }

    /// Return the current parsed value from the buffer.
    pub fn value(&self) -> Cmd {
        // TODO: Parse the sequence with the highest level of meaning (or use the existing value if parsing is already done).
        if self.raw.is_empty() {
            Cmd::Noop
        } else if self.raw == "exit" || self.raw == "quit" {
            Cmd::Exit
        } else {
            Cmd::Help(None)
        }
    }
}

impl Inp {
    /// Compose this command with another to return a composed command.
    pub fn compose(self, other: Inp) -> Inp {
        Inp::Compose(Box::new(self), Box::new(other))
    }

    /// Create a repeated command.
    pub fn repeat(self, times: usize) -> Inp {
        Inp::Repeat(Box::new(self), times)
    }

    /// Evaluate the command and return the generated result.
    pub fn eval(self, buf: &mut Buf) -> bool {
        match self {
            Inp::Noop => true,
            Inp::Exit => {
                buf.trm = true;
                true
            }
            Inp::Esc(val) => {
                buf.esc = val;
                true
            }
            Inp::Compose(fst, snd) => fst.eval(buf) && snd.eval(buf),
            Inp::Repeat(cmd, cnt) => {
                let mut ans = true;
                for _ in 0..cnt {
                    ans = cmd.clone().eval(buf);
                    if !ans {
                        break;
                    }
                }
                ans
            }
            Inp::Push(chr) => {
                if buf.raw.len() < usize::MAX {
                    // Remember the current length of the raw string before updating it.
                    let old_len = buf.raw.len();
                    // Insert the character at the current location.
                    buf.raw.insert(buf.idx, chr);
                    // Increment the index by the same amount that the raw string's length has increased by.
                    buf.idx += buf.raw.len() - old_len;
                    true
                } else {
                    false
                }
            }
            Inp::Delete => {
                if buf.raw.len() > buf.idx {
                    let nxt = &buf.raw[buf.idx..];
                    let wid = nxt
                        .graphemes(true)
                        .next()
                        .map(|g| g.chars().count())
                        .unwrap_or(0);
                    for _ in 0..wid {
                        buf.raw.remove(buf.idx);
                    }
                    true
                } else {
                    false
                }
            }
            Inp::Clear => {
                *buf = Buf::new();
                true
            }
            Inp::MoveUp => todo!(), // TODO: Implement vertical movement.
            Inp::MoveDn => todo!(), // TODO: Implement vertical movement.
            Inp::MoveLt => {
                if buf.idx == 0 {
                    false
                } else {
                    let mut cnt = 0;
                    let mut rev = 0;
                    for s in buf.raw.graphemes(true) {
                        if buf.idx == cnt {
                            break;
                        } else {
                            rev = s.len();
                            cnt += rev;
                        }
                    }
                    buf.idx = cnt - rev;
                    true
                }
            }
            Inp::MoveRt => {
                if buf.idx == buf.raw.len() {
                    false
                } else {
                    let mut cnt = 0;
                    let mut lst = false;
                    for s in buf.raw.graphemes(true) {
                        if lst {
                            break;
                        }
                        if buf.idx == cnt {
                            lst = true;
                        }
                        cnt += s.len();
                    }
                    buf.idx = cnt;
                    true
                }
            }
        }
    }
}
