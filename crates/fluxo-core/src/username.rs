//! Usernames: how people find and invite each other. They can't be changed once taken.
//!
//! Rules (decided 2026-11-16): 3–32 characters of `a–z`, `0–9`, `.`, `-` and `_`; starting and
//! ending with a letter or digit; no two separators in a row; and not a reserved name.
//! Input is trimmed and lowercased before checking, so `Ada` and `ada` are the same name.

pub const MIN: usize = 3;
pub const MAX: usize = 32;

/// Names that could pass for fluxo itself or for its staff, or that match app paths.
const RESERVED: &[&str] = &[
    "admin",
    "admins",
    "administrator",
    "administrators",
    "api",
    "focus",
    "fluxo",
    "goals",
    "help",
    "join",
    "login",
    "logout",
    "me",
    "moderator",
    "null",
    "owner",
    "root",
    "security",
    "settings",
    "setup",
    "signup",
    "staff",
    "static",
    "support",
    "system",
    "undefined",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    TooShort,
    TooLong,
    BadCharacter,
    BadEdge,
    DoubleSeparator,
    Reserved,
}

impl Problem {
    /// What went wrong and how to fix it, for showing next to the field.
    pub fn message(self) -> &'static str {
        match self {
            Problem::TooShort => "Use at least 3 characters.",
            Problem::TooLong => "Use 32 characters or fewer.",
            Problem::BadCharacter => "Use only lowercase letters, numbers, dots, hyphens and underscores.",
            Problem::BadEdge => "Start and end with a letter or number.",
            Problem::DoubleSeparator => "Don’t put two dots, hyphens or underscores next to each other.",
            Problem::Reserved => "That name is reserved. Pick another.",
        }
    }
}

/// Normalize `input` and check it. Returns the username to store.
pub fn parse(input: &str) -> Result<String, Problem> {
    let name = input.trim().to_lowercase();
    let len = name.chars().count();
    if len < MIN {
        return Err(Problem::TooShort);
    }
    if len > MAX {
        return Err(Problem::TooLong);
    }
    let sep = |c: char| matches!(c, '.' | '-' | '_');
    if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || sep(c)) {
        return Err(Problem::BadCharacter);
    }
    if name.starts_with(sep) || name.ends_with(sep) {
        return Err(Problem::BadEdge);
    }
    if name.as_bytes().windows(2).any(|w| sep(w[0] as char) && sep(w[1] as char)) {
        return Err(Problem::DoubleSeparator);
    }
    if RESERVED.contains(&name.as_str()) {
        return Err(Problem::Reserved);
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_and_normalizes() {
        assert_eq!(parse("  Ada  "), Ok("ada".into()));
        assert_eq!(parse("ada.lovelace"), Ok("ada.lovelace".into()));
        assert_eq!(parse("k-o_f.i"), Ok("k-o_f.i".into()));
        assert_eq!(parse("a1b"), Ok("a1b".into()));
        assert_eq!(parse(&"a".repeat(32)), Ok("a".repeat(32)));
    }

    #[test]
    fn rejects_with_a_reason() {
        assert_eq!(parse("ab"), Err(Problem::TooShort));
        assert_eq!(parse(&"a".repeat(33)), Err(Problem::TooLong));
        assert_eq!(parse("ada lovelace"), Err(Problem::BadCharacter));
        assert_eq!(parse("adé"), Err(Problem::BadCharacter));
        assert_eq!(parse(".ada"), Err(Problem::BadEdge));
        assert_eq!(parse("ada-"), Err(Problem::BadEdge));
        assert_eq!(parse("ada..x"), Err(Problem::DoubleSeparator));
        assert_eq!(parse("ada-_x"), Err(Problem::DoubleSeparator));
        assert_eq!(parse("Administrator"), Err(Problem::Reserved));
        assert_eq!(parse("fluxo"), Err(Problem::Reserved));
    }
}
