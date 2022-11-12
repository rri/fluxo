//! Core application module and primary entry-point for *fluxo*.
//!
//! The binary application delegates directly to this module, which is responsible for parsing
//! command-line options and determining what features to launch. This module delegates to the
//! [integrated development environment][crate::ide] for interactive program development.

use crate::scr::Screen;
use clap::{AppSettings, Parser};
use std::io::Result;

#[derive(Parser)]
#[clap(about, long_about = None, version)]
#[clap(global_setting(AppSettings::ArgRequiredElseHelp))]
#[clap(global_setting(AppSettings::ColorAuto))]
#[clap(global_setting(AppSettings::DeriveDisplayOrder))]
#[clap(global_setting(AppSettings::DisableColoredHelp))]
#[clap(global_setting(AppSettings::DisableVersion))]
struct Args {
    /// Open an interactive development environment.
    #[clap(short, long)]
    interactive: bool,

    /// Print version information and exit.
    #[clap(short, long)]
    version: bool,
}

/// Run the application, parsing arguments supplied to the binary during invocation.
pub fn run() -> Result<()> {
    let args = Args::parse();

    if args.version {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        println!("{}", env!("CARGO_PKG_DESCRIPTION"));
        return Ok(());
    }

    if args.interactive {
        Screen::run()?;
    }

    Ok(())
}
