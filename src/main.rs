//! The `yscope-linter` binary, which lints the files and directories its arguments name.

use std::process::ExitCode;

fn main() -> ExitCode {
    yscope_linter::main()
}
