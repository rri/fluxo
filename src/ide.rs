//! Text-based user interface that serves as an integrated development environment.

use std::io::Result;

/// Window that provides a text-based user interface.
///
/// Create a window object and invoke its [run] method to launch a new interactive session. The
/// window object tracks its lifecycle and automatically performs cleanup operation when dropped.
pub struct Window {
    init: bool,
}

impl Window {
    /// Create a new window object.
    pub fn new() -> Self {
        Window { init: false }
    }

    /// Run the integrated development environment and return a result when the user session ends.
    pub fn run(&mut self) -> Result<()> {
        self.init = true;
        Ok(())
    }

    /// Perform any cleanup operations such as resetting terminal state or restoring buffers.
    pub fn cleanup(&mut self) -> Result<()> {
        Ok(())
    }
}

impl Default for Window {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        if self.init {
            if let Err(e) = self.cleanup() {
                eprintln!("Errors while exiting:");
                eprintln!("{}", e);
            }
        }
    }
}
