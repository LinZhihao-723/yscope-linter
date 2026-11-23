//! `yscope-linter`: lints the y-scope Rust comment and docstring rules that rustfmt and clippy
//! can't check.
//!
//! The library holds the whole tool, behind the entry points of its two binaries: `yscope-linter`,
//! which lints the paths it is given, and `cargo-yscope-linter`, which lints the packages of a
//! cargo workspace.
//!
//! Exit codes:
//!
//! * 0: no diagnostics.
//! * 1: at least one diagnostic.
//! * 2: a usage error, an invalid target, diff, workspace or setting, or an I/O failure.

mod cargo;
mod cli;
mod config;
mod diagnostic;
mod diff;
mod doc;
mod driver;
mod error;
mod file_context;
mod lexer;
mod line_ranges;
mod markdown;
mod rules;
mod source;
mod suppression;
mod syntax;

use std::io::Write;
use std::process::ExitCode;

use clap::Parser;

/// Runs `yscope-linter` on the process's arguments.
///
/// # Returns
///
/// The exit code.
pub fn main() -> ExitCode {
    let cli = cli::Cli::parse();
    print_warnings(&cli.warnings());
    let config = match cli.into_config() {
        Ok(config) => config,
        Err(error) => return report_fatal_error(&error),
    };
    let format = config.format;
    match driver::run(config) {
        Ok(report) => print_report(&report, format),
        Err(error) => report_fatal_error(&error),
    }
}

/// Runs `cargo-yscope-linter` on the process's arguments, which start with `yscope-linter` when
/// cargo runs it.
///
/// # Returns
///
/// The exit code.
pub fn cargo_main() -> ExitCode {
    let cli::CargoCli::YscopeLinter(cargo_args) = cli::CargoCli::parse();
    print_warnings(&cargo_args.warnings());
    let config = cargo_args.into_config();
    let format = config.format;
    let plan = match cargo::Plan::new(config) {
        Ok(plan) => plan,
        Err(error) => return report_fatal_error(&error),
    };
    print_warnings(&plan.warnings);
    match driver::check(plan.file_groups) {
        Ok(report) => print_report(&report, format),
        Err(error) => report_fatal_error(&error),
    }
}

fn print_warnings(warnings: &[String]) {
    for warning in warnings {
        eprintln!("yscope-linter: warning: {warning}");
    }
}

/// Prints the diagnostics to stdout, and the summary to stderr.
///
/// # Returns
///
/// The exit code of the report.
fn print_report(report: &driver::Report, format: config::OutputFormat) -> ExitCode {
    let mut stdout = std::io::stdout().lock();
    for file_diagnostic in &report.diagnostics {
        if writeln!(stdout, "{}", file_diagnostic.render(format)).is_err() {
            return ExitCode::from(2);
        }
    }
    eprintln!(
        "yscope-linter: {} in {} checked, {} skipped",
        diagnostic::counted(report.diagnostics.len(), "diagnostic"),
        diagnostic::counted(report.num_files_checked, "file"),
        diagnostic::counted(report.num_generated_files, "generated file")
    );
    if report.diagnostics.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// Prints a fatal error to stderr.
///
/// # Returns
///
/// The exit code of a fatal error.
fn report_fatal_error(error: &error::Error) -> ExitCode {
    eprintln!("yscope-linter: error: {error}");
    ExitCode::from(2)
}
