//! Styled prompts for various input and output scenarios.

use crossterm::style::{Color, StyledContent, Stylize};
use std::fmt::Display;

/// Types of prompts that may be rendered to the user under various circumstances.
pub enum Prompt {
    /// System is ready for fresh input.
    Ready,
    /// System is ready to resume after previously entered input (which was incomplete).
    Continue,
    /// System has generated the success message that follows the prompt.
    Success,
    /// System has generated the failure message that follows the prompt.
    Failure,
    /// System is providing diagnostic information.
    Diagnostics,
}

impl Prompt {
    /// Prefix this prompt to the specified output.
    pub fn prefix_to(&self, output: &str) -> String {
        output
            .lines()
            .map(|s| format!("{} {}\r\n", self, s.trim_end()))
            .collect::<String>()
    }

    /// Render the prompt as styled content (such as a colored string).
    fn as_styled_content(&self) -> StyledContent<&'static str> {
        match self {
            Prompt::Ready => "»".with(Color::Cyan),
            Prompt::Continue => "↳".with(Color::Cyan),
            Prompt::Success => "∴".with(Color::DarkGreen),
            Prompt::Failure => "✗".with(Color::Red),
            Prompt::Diagnostics => "☼".with(Color::Yellow),
        }
    }
}

impl Display for Prompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_styled_content())
    }
}