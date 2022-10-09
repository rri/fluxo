//! Request structures created from user input.

use super::cmd::Cmd;

/// Request object created as a result of parsing user input.
///
/// User input is parsed incrementally as it is entered, so the request object may represent an
/// instruction that isn't yet fully formed, and requires further input to complete construction.
#[derive(Clone)]
pub struct Req {
    pub cmd: Option<Cmd>,
}

impl Req {
    /// Create a new request object.
    pub fn new() -> Self {
        Req { cmd: None }
    }

    /// Create a new request object with the command specified.
    pub fn cmd(&self, cmd: Cmd) -> Self {
        let mut req = self.clone();
        req.cmd = Some(cmd);
        req
    }
}

impl Default for Req {
    fn default() -> Self {
        Self::new()
    }
}
