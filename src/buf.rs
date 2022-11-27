//! Editor buffer and related utilities.

use crate::pmt::Prompt;

const CHR_ESC: char = '\\';

/// Buffer object.
#[derive(Debug, Default)]
pub struct Buf {
    /// Raw data in the buffer.
    pub raw: String,
    /// Current location of the cursor within the buffer.
    pub idx: usize,
    /// Whether or not this buffer is currently in escape mode.
    pub esc: bool,
}

impl Buf {
    /// Create a new instance of the buffer.
    pub fn new() -> Self {
        Self {
            raw: String::new(),
            idx: 0,
            esc: false,
        }
    }

    /// Push a character into the buffer at the current location.
    pub fn push(&mut self, chr: char) {
        if self.esc {
            match chr {
                'l' => self.insert('λ'),
                'p' => self.insert('Π'),
                'u' => self.insert('□'),
                _ => self.insert(chr),
            }
            self.esc = false;
        } else {
            match chr {
                CHR_ESC => self.esc = true,
                _ => self.insert(chr),
            }
        }
    }

    /// Move the cursor left (if possible).
    pub fn move_left(&mut self) {}

    /// Move the cursor right (if possible).
    pub fn move_right(&mut self) {}

    /// Fetch a rendering of this buffer for display.
    ///
    /// The rendering consists of two things:
    ///
    /// * String to be displayed.
    /// * Index of the cursor relative to the start of the string.
    pub fn render(&self) -> String {
        format!(
            "{} {}",
            &Prompt::Ready,
            &self
                .raw
                .replace("\r\n", &Prompt::Continue.prefix_to("\r\n"))
        )
    }

    /// Insert a character at the current location and advance the cursor.
    fn insert(&mut self, chr: char) {
        // Remember the current length of the raw string before updating it.
        let old_len = self.raw.len();
        // Insert the character at the current location.
        self.raw.insert(self.idx, chr);
        // Increment the index by the same amount that the raw string's length has increased by.
        self.idx += self.raw.len() - old_len;
    }
}
