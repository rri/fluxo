use fluxo::app;
use std::process;

/// Main function and entry-point for the operating system process.
fn main() {
    process::exit(match app::run() {
        Ok(()) => 0,
        Err(_) => 1,
    })
}
