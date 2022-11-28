//! Mapping traits and implementations from keyboard input events to [buffer commands][BufCmd].

use crate::buf::{Buf, BufCmd, ESC};
use crossterm::event::{KeyCode, KeyEvent};

/// Number of rows that represent a single 'page'.
const PAGE_SIZE: usize = 10;

/// Conversion trait to go from a keyboard input to a [buffer command][BufCmd].
pub trait KeyMap {
    /// Convert a [keyboard event][KeyEvent] to a [buffer command][BufCmd].
    fn convert(buf: &Buf, evt: &KeyEvent) -> Option<BufCmd>;
}

/// Default [KeyMap] implementation for standard characters.
pub struct DefKeyMap;

/// [KeyMap] implementation to handle the case of auto-correct "fixes".
pub struct FixKeyMap;

/// [KeyMap] implementation to handle all cases where the buffer is in 'escape mode'.
pub struct EscKeyMap;

impl KeyMap for DefKeyMap {
    fn convert(_: &Buf, evt: &KeyEvent) -> Option<BufCmd> {
        match evt.code {
            KeyCode::Char(chr) => {
                if chr == ESC {
                    Some(BufCmd::Esc(true))
                } else {
                    Some(BufCmd::Push(chr))
                }
            }
            KeyCode::Delete => Some(BufCmd::Delete),
            KeyCode::Backspace => Some(BufCmd::MoveLt).map(|cmd| cmd.compose(BufCmd::Delete)),
            KeyCode::Left => Some(BufCmd::MoveLt),
            KeyCode::Right => Some(BufCmd::MoveRt),
            KeyCode::Up => Some(BufCmd::MoveUp),
            KeyCode::Down => Some(BufCmd::MoveDn),
            KeyCode::Home => Some(BufCmd::MoveLt).map(|cmd| cmd.repeat(usize::MAX)),
            KeyCode::End => Some(BufCmd::MoveRt).map(|cmd| cmd.repeat(usize::MAX)),
            KeyCode::PageUp => Some(BufCmd::MoveUp).map(|cmd| cmd.repeat(PAGE_SIZE)),
            KeyCode::PageDown => Some(BufCmd::MoveDn).map(|cmd| cmd.repeat(PAGE_SIZE)),
            KeyCode::Esc => Some(BufCmd::Clear),
            KeyCode::Enter => Some(BufCmd::Exit),
            KeyCode::Tab => todo!(),     // TODO: Make the Tab key work.
            KeyCode::BackTab => todo!(), // TODO: Make the Backtab key work.
            _ => None,
        }
    }
}

impl KeyMap for FixKeyMap {
    fn convert(_: &Buf, _: &KeyEvent) -> Option<BufCmd> {
        // TODO: Implement the keymap.
        None
    }
}

impl KeyMap for EscKeyMap {
    fn convert(buf: &Buf, evt: &KeyEvent) -> Option<BufCmd> {
        if buf.esc {
            match evt.code {
                // Special characters.
                KeyCode::Char('l') => Some(BufCmd::Push('λ')),
                KeyCode::Char('p') => Some(BufCmd::Push('Π')),
                KeyCode::Char('*') => Some(BufCmd::Push('□')),

                // Escape the escape character.
                KeyCode::Char(ESC) => Some(BufCmd::Push(ESC)),

                // Enter a literal newline within the editor.
                KeyCode::Enter => Some(BufCmd::Push('\n')),

                // Ignore all other keyboard inputs
                _ => Some(BufCmd::Noop),
            }
            .map(|cmd| cmd.compose(BufCmd::Esc(false)))
        } else {
            None
        }
    }
}
