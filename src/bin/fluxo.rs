//! Main module for the command-line application launcher.

use fluxo::app;
use fluxo::cmd::Status;
use std::process;

/// Main function and entry-point for the operating system process.
fn main() {
    process::exit(match app::run() {
        Ok(()) => exitcode::OK,
        Err(e) => {
            eprint!("{}", &Status::Failure.prefix_to(&e.to_string()));
            exitcode::IOERR
        }
    })
}
