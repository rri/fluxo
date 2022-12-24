//! Mapping traits and implementations from keyboard input events to [buffer inputs][Inp].

use crate::buf::{Buf, Inp, ESC};
use crossterm::event::{KeyCode, KeyEvent};

/// Number of rows that represent a single 'page'.
const PAGE_SIZE: usize = 10;

/// Conversion trait to go from a keyboard input to a [buffer input][Inp].
pub trait KeyMap {
    /// Convert a [keyboard event][KeyEvent] to a [buffer input][Inp].
    fn convert(buf: &Buf, evt: &KeyEvent) -> Option<Inp>;
}

/// Default [KeyMap] implementation for standard characters.
pub struct DefKeyMap;

/// [KeyMap] implementation to handle the case of auto-correct "fixes".
pub struct FixKeyMap;

/// [KeyMap] implementation to handle all cases where the buffer is in 'escape mode'.
pub struct EscKeyMap;

impl KeyMap for DefKeyMap {
    fn convert(_: &Buf, evt: &KeyEvent) -> Option<Inp> {
        match evt.code {
            KeyCode::Char(chr) => {
                if chr == ESC {
                    Some(Inp::Esc(true))
                } else {
                    Some(Inp::Push(chr))
                }
            }
            KeyCode::Delete => Some(Inp::Delete),
            KeyCode::Backspace => Some(Inp::MoveLt).map(|cmd| cmd.compose(Inp::Delete)),
            KeyCode::Left => Some(Inp::MoveLt),
            KeyCode::Right => Some(Inp::MoveRt),
            KeyCode::Up => Some(Inp::MoveUp),
            KeyCode::Down => Some(Inp::MoveDn),
            KeyCode::Home => Some(Inp::MoveLt).map(|cmd| cmd.repeat(usize::MAX)),
            KeyCode::End => Some(Inp::MoveRt).map(|cmd| cmd.repeat(usize::MAX)),
            KeyCode::PageUp => Some(Inp::MoveUp).map(|cmd| cmd.repeat(PAGE_SIZE)),
            KeyCode::PageDown => Some(Inp::MoveDn).map(|cmd| cmd.repeat(PAGE_SIZE)),
            KeyCode::Esc => Some(Inp::Clear),
            KeyCode::Enter => Some(Inp::Exit),
            KeyCode::Tab => todo!(),     // TODO: Make the Tab key work.
            KeyCode::BackTab => todo!(), // TODO: Make the Backtab key work.
            _ => None,
        }
    }
}

impl KeyMap for FixKeyMap {
    fn convert(_: &Buf, _: &KeyEvent) -> Option<Inp> {
        // TODO: Implement the keymap.
        None
    }
}

impl KeyMap for EscKeyMap {
    fn convert(buf: &Buf, evt: &KeyEvent) -> Option<Inp> {
        if buf.esc {
            match evt.code {
                // Special characters.
                KeyCode::Char('l') => Some(Inp::Push('λ')),
                KeyCode::Char('p') => Some(Inp::Push('Π')),
                KeyCode::Char('*') => Some(Inp::Push('□')),

                // Escape the escape character.
                KeyCode::Char(ESC) => Some(Inp::Push(ESC)),

                // Enter a literal newline within the editor.
                KeyCode::Enter => Some(Inp::Push('\n')),

                // Ignore all other keyboard inputs
                _ => Some(Inp::Noop),
            }
            .map(|cmd| cmd.compose(Inp::Esc(false)))
        } else {
            None
        }
    }
}
