use fluxo::app;
use fluxo::pmt::Prompt;
use std::process;

/// Main function and entry-point for the operating system process.
fn main() {
    process::exit(match app::run() {
        Ok(()) => exitcode::OK,
        Err(e) => {
            eprint!("{}", Prompt::Failure.prefix_to("I/O error:"));
            eprint!("{}", Prompt::Diagnostics.prefix_to(&format!("{}", e)));
            exitcode::IOERR
        }
    })
}
