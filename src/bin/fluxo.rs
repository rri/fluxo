use fluxo::cli;
use std::process;

fn main() {
    process::exit(match cli::run() {
        Ok(()) => 0,
        Err(_) => 1,
    })
}
