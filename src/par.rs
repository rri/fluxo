//! Parsing utilities.

/// Categories of [tokens][Tkn].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Cat {
    /// Representation of the cursor.
    Cur,
    /// String of characters with no implied meaning.
    Str,
}

/// Token that represents a word in the language being parsed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tkn {
    /// Input string slice that forms the basis of this token (if any).
    pub inp: Option<String>,
    /// Category of this token.
    pub cat: Cat,
}

impl Tkn {
    /// Create a new instance of a token.
    pub fn new(inp: &str, cat: Cat) -> Self {
        Self {
            inp: Some(inp.to_string()),
            cat,
        }
    }
}
