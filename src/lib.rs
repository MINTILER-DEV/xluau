mod ast;
mod compiler;
mod config;
mod diagnostic;
mod emitter;
mod error;
mod formatter;
mod lexer;
mod parser;
mod source;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use compiler::{BuildSummary, CheckSummary, Compiler};
use config::XLuauConfig;
use error::{Result, XLuauError};

#[derive(Debug, Parser)]
#[command(
    name = "xluau",
    version,
    about = "A Phase 1 XLuau compiler pipeline implemented in Rust."
)]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Build {
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
    },
    Check {
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
    },
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let cwd = std::env::current_dir()?;
    let config = XLuauConfig::load_or_default(&cwd, cli.config.as_deref())?;
    let compiler = Compiler::new(cwd, config)?;

    match cli.command {
        Command::Build { paths } => print_build_summary(compiler.build(&paths)?),
        Command::Check { paths } => print_check_summary(compiler.check(&paths)?),
    }

    Ok(())
}

fn print_build_summary(summary: BuildSummary) {
    println!(
        "Built {} file(s); wrote {} output file(s).",
        summary.checked_files, summary.written_files
    );
}

fn print_check_summary(summary: CheckSummary) {
    println!(
        "Checked {} file(s); no diagnostics emitted.",
        summary.checked_files
    );
}

pub fn invalid_input(message: impl Into<String>) -> XLuauError {
    XLuauError::Validation(message.into())
}
