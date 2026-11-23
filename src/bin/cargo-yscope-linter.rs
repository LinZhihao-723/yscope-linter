//! The `cargo-yscope-linter` binary, which cargo runs for `cargo yscope-linter` to lint the
//! packages of a cargo workspace.

use std::process::ExitCode;

fn main() -> ExitCode {
    yscope_linter::cargo_main()
}
