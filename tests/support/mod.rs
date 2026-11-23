//! Helpers shared by the end-to-end tests.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;

pub const INVALID_SUPPRESSION: &str = "// yscope-linter: allow(errors-doc)\n";

/// Creates an empty directory for one test under the target's temporary directory.
///
/// # Returns
///
/// The path of the directory.
///
/// # Panics
///
/// Panics if the directory can't be recreated.
pub fn test_directory(test_name: &str) -> PathBuf {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(env!("CARGO_CRATE_NAME"))
        .join(test_name);
    if directory.exists() {
        fs::remove_dir_all(&directory).expect("failed to clear the test directory");
    }
    fs::create_dir_all(&directory).expect("failed to create the test directory");
    directory
}

/// Writes `contents` to `relative_path` under `directory`, creating parent directories.
///
/// # Panics
///
/// Panics if the file can't be written.
pub fn write_file(directory: &Path, relative_path: &str, contents: &str) {
    let path = directory.join(relative_path);
    fs::create_dir_all(path.parent().expect("a file path has a parent"))
        .expect("failed to create the parent directory");
    fs::write(path, contents).expect("failed to write the test file");
}

/// Runs `command` in `directory`, feeding `stdin` to it.
///
/// # Returns
///
/// The finished process's output.
///
/// # Panics
///
/// Panics if the command can't be run.
pub fn run_command(mut command: Command, directory: &Path, stdin: &str) -> Output {
    let mut child = command
        .current_dir(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn the binary");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(stdin.as_bytes())
        .expect("failed to write stdin");
    child
        .wait_with_output()
        .expect("failed to wait for the binary")
}

pub fn stdout_lines(output: &Output) -> Vec<String> {
    String::from_utf8(output.stdout.clone())
        .expect("stdout is UTF-8")
        .lines()
        .map(str::to_owned)
        .collect()
}
