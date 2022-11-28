use fluxo::app;
use fluxo::pmt::Prompt;
use std::process;

/// Main function and entry-point for the operating system process.
fn main() {
    process::exit(match app::run() {
        Ok(()) => exitcode::OK,
        Err(e) => {
            eprint!("{}", Prompt::show_failure(&e.to_string()));
            exitcode::IOERR
        }
    })
}
