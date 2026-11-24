//! End-to-end tests of the `cargo-yscope-linter` binary.

mod support;

use std::path::Path;
use std::process::Command;
use std::process::Output;

use support::INVALID_SUPPRESSION;
use support::TEST_FN_WITH_A_STALE_FORWARD;
use support::stdout_lines;
use support::test_directory;
use support::write_file;

/// Runs the binary in `directory` as cargo runs it for `cargo yscope-linter <args>`, feeding
/// `stdin` to it.
///
/// # Returns
///
/// The finished process's output.
fn run(directory: &Path, args: &[&str], stdin: &str) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-yscope-linter"));
    command
        .arg("yscope-linter")
        .args(args)
        .env("CARGO_TERM_COLOR", "never");
    support::run_command(command, directory, stdin)
}

/// # Returns
///
/// The distinct paths of the diagnostics on stdout, in output order.
fn diagnostic_paths(output: &Output) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    for line in stdout_lines(output) {
        let path = line
            .split(':')
            .next()
            .expect("a diagnostic has a path")
            .to_owned();
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

/// # Returns
///
/// The `Cargo.toml` of a package named `name`, followed by `extra`.
fn package_manifest(name: &str, extra: &str) -> String {
    format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n{extra}")
}

/// Writes a virtual workspace with the members `a` and `b`, whose only default member is `a`, and
/// an invalid suppression in each member's `src/lib.rs`, and in `.rs` files that aren't in any
/// member: one outside the members, one in a nested package that isn't a member, and one in a
/// member's `target` directory.
///
/// # Returns
///
/// The workspace's directory.
fn virtual_workspace(test_name: &str) -> std::path::PathBuf {
    let directory = test_directory(test_name);
    write_file(
        &directory,
        "Cargo.toml",
        "[workspace]\nresolver = \"3\"\nmembers = [\"a\", \"b\"]\ndefault-members = [\"a\"]\n",
    );
    for member in ["a", "b"] {
        write_file(
            &directory,
            &format!("{member}/Cargo.toml"),
            &package_manifest(member, ""),
        );
        write_file(
            &directory,
            &format!("{member}/src/lib.rs"),
            INVALID_SUPPRESSION,
        );
    }
    write_file(&directory, "tmp/stray.rs", INVALID_SUPPRESSION);
    write_file(
        &directory,
        "a/fixtures/inner/Cargo.toml",
        &package_manifest("inner", "[workspace]\n"),
    );
    write_file(
        &directory,
        "a/fixtures/inner/src/lib.rs",
        INVALID_SUPPRESSION,
    );
    write_file(&directory, "a/target/debug/out.rs", INVALID_SUPPRESSION);
    directory
}

#[test]
fn default_selection_is_the_default_members_of_the_selected_manifest() {
    let directory =
        virtual_workspace("default_selection_is_the_default_members_of_the_selected_manifest");
    let output = run(&directory, &[], "");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(diagnostic_paths(&output), ["a/src/lib.rs"]);
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "yscope-linter: 1 diagnostic in 1 file checked, 0 generated files skipped\n"
    );

    let output = run(&directory.join("b/src"), &[], "");
    assert_eq!(diagnostic_paths(&output), ["lib.rs"]);

    let output = run(
        &directory.join("tmp"),
        &["--manifest-path", "../b/Cargo.toml"],
        "",
    );
    assert_eq!(diagnostic_paths(&output), ["../b/src/lib.rs"]);
}

#[test]
fn package_flags_select_workspace_members() {
    let directory = virtual_workspace("package_flags_select_workspace_members");
    let cases: [(&[&str], &[&str]); 5] = [
        (&["--workspace"], &["a/src/lib.rs", "b/src/lib.rs"]),
        (
            &["--workspace", "--exclude-package", "a"],
            &["b/src/lib.rs"],
        ),
        (&["-p", "b"], &["b/src/lib.rs"]),
        (
            &["--package", "b", "--package", "a"],
            &["a/src/lib.rs", "b/src/lib.rs"],
        ),
        (
            &["-p", "b", "--workspace"],
            &["a/src/lib.rs", "b/src/lib.rs"],
        ),
    ];
    for (args, expected_paths) in cases {
        let output = run(&directory, args, "");
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert_eq!(diagnostic_paths(&output), expected_paths, "{args:?}");
    }
}

#[test]
fn single_package_lints_every_target_but_not_its_target_directory() {
    let directory =
        test_directory("single_package_lints_every_target_but_not_its_target_directory");
    write_file(
        &directory,
        "Cargo.toml",
        &package_manifest("single", "[workspace]\n"),
    );
    let checked_paths = [
        "benches/bench.rs",
        "build.rs",
        "examples/example.rs",
        "src/bin/tool.rs",
        "src/lib.rs",
        "src/main.rs",
        "tests/test.rs",
    ];
    for path in checked_paths {
        write_file(&directory, path, INVALID_SUPPRESSION);
    }
    write_file(
        &directory,
        "out/debug/build/generated.rs",
        INVALID_SUPPRESSION,
    );
    let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-yscope-linter"));
    command.arg("yscope-linter").env("CARGO_TARGET_DIR", "out");
    let output = support::run_command(command, &directory, "");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(diagnostic_paths(&output), checked_paths);

    let output = run(&directory, &[], "");
    assert!(
        diagnostic_paths(&output).contains(&"out/debug/build/generated.rs".to_owned()),
        "{output:?}"
    );
}

#[test]
fn package_settings_override_workspace_settings_and_the_command_line_overrides_both() {
    let directory = test_directory(
        "package_settings_override_workspace_settings_and_the_command_line_overrides_both",
    );
    write_file(
        &directory,
        "Cargo.toml",
        "[workspace]\nresolver = \"3\"\nmembers = [\"a\", \
         \"b\"]\n\n[workspace.metadata.yscope-linter]\nwidth = 10\nexclude = \
         [\"a/../b/src/skipped.rs\"]\n\n[workspace.metadata.other-tool]\nkey = 1\n",
    );
    write_file(
        &directory,
        "a/Cargo.toml",
        &package_manifest(
            "a",
            "\n[package.metadata.yscope-linter]\nwidth = 11\nexclude = [\"src/generated\"]\n",
        ),
    );
    write_file(
        &directory,
        "b/Cargo.toml",
        &package_manifest(
            "b",
            "\n[package.metadata.yscope-linter]\nrules = [\"ORG\"]\n",
        ),
    );
    let under_filled = "// aaaa\n// bbb\n";
    write_file(&directory, "a/src/lib.rs", under_filled);
    write_file(&directory, "a/src/generated/mod.rs", INVALID_SUPPRESSION);
    write_file(&directory, "b/src/lib.rs", under_filled);
    write_file(&directory, "b/src/skipped.rs", INVALID_SUPPRESSION);

    let output = run(&directory, &[], "");
    assert_eq!(
        stdout_lines(&output),
        [
            "a/src/lib.rs:1:1: DOC001 comment-greedy-wrap: line is 7 columns wide and the next \
             line's first word `bbb` (3 columns) fits within 11; move it up"
        ]
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "yscope-linter: 1 diagnostic in 2 files checked, 0 generated files skipped\n"
    );

    let output = run(&directory, &["--width", "10"], "");
    assert_eq!(output.status.code(), Some(0));

    let output = run(&directory, &["--width", "11", "--rule", "DOC001"], "");
    assert_eq!(diagnostic_paths(&output), ["a/src/lib.rs", "b/src/lib.rs"]);

    let output = run(&directory, &["--exclude", "a/src/lib.rs"], "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn command_line_excludes_are_relative_to_the_current_directory() {
    let directory =
        virtual_workspace("command_line_excludes_are_relative_to_the_current_directory");
    let output = run(
        &directory.join("a"),
        &["--workspace", "--exclude", "src/lib.rs"],
        "",
    );
    assert_eq!(diagnostic_paths(&output), ["../b/src/lib.rs"]);
}

#[test]
fn command_line_overrides_the_skip_tests_setting_either_way() {
    let directory = test_directory("command_line_overrides_the_skip_tests_setting_either_way");
    let write_settings = |settings: &str| {
        write_file(
            &directory,
            "Cargo.toml",
            &package_manifest(
                "single",
                &format!("[workspace]\n\n[package.metadata.yscope-linter]\n{settings}"),
            ),
        );
    };
    write_file(&directory, "src/lib.rs", TEST_FN_WITH_A_STALE_FORWARD);
    let expected_lines = [
        "src/lib.rs:7:1: DOC002 errors-doc: \"Forwards [`load`]\" names a function whose errors \
         `checks` never forwards",
    ];
    write_settings("");
    let output = run(&directory, &[], "");
    assert_eq!(stdout_lines(&output), expected_lines);

    write_settings("skip-tests = true\n");
    let output = run(&directory, &[], "");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout_lines(&output), Vec::<String>::new());
    let output = run(&directory, &["--no-skip-tests"], "");
    assert_eq!(stdout_lines(&output), expected_lines);

    write_settings("skip-tests = false\n");
    let output = run(&directory, &[], "");
    assert_eq!(stdout_lines(&output), expected_lines);
    let output = run(&directory, &["--skip-tests"], "");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout_lines(&output), Vec::<String>::new());
}

#[test]
fn skip_tests_list_of_a_package_replaces_the_workspace_list() {
    let directory = test_directory("skip_tests_list_of_a_package_replaces_the_workspace_list");
    write_file(
        &directory,
        "Cargo.toml",
        "[workspace]\nresolver = \"3\"\nmembers = [\"a\", \
         \"b\"]\n\n[workspace.metadata.yscope-linter]\nskip-tests = [\"type-params-doc\"]\n",
    );
    write_file(
        &directory,
        "a/Cargo.toml",
        &package_manifest(
            "a",
            "\n[package.metadata.yscope-linter]\nskip-tests = [\"ORG\"]\n",
        ),
    );
    write_file(&directory, "b/Cargo.toml", &package_manifest("b", ""));
    let test_module = concat!(
        "#[cfg(test)]\n",
        "mod tests {\n",
        "    /// A wrapper.\n",
        "    pub struct Wrapper<T>(T);\n",
        "    fn a() {}\n",
        "    pub fn b() {}\n",
        "}\n",
    );
    write_file(&directory, "a/src/lib.rs", test_module);
    write_file(&directory, "b/src/lib.rs", test_module);
    let findings = |member: &str| {
        [
            format!(
                "{member}/src/lib.rs:3:5: DOC003 type-params-doc: struct `Wrapper` has generic \
                 parameters but no `# Type Parameters` section; add one with a bullet for each of \
                 `T`"
            ),
            format!(
                "{member}/src/lib.rs:6:5: ORG001 symbol-order: pub fn `b` should come after pub \
                 struct `Wrapper`: pub items come before private items"
            ),
        ]
    };
    let [a_type_params_doc, a_symbol_order] = findings("a");
    let [b_type_params_doc, b_symbol_order] = findings("b");

    let output = run(&directory, &["--workspace", "--no-skip-tests"], "");
    assert_eq!(
        stdout_lines(&output),
        [
            a_type_params_doc.clone(),
            a_symbol_order.clone(),
            b_type_params_doc,
            b_symbol_order.clone()
        ]
    );

    let output = run(&directory, &["--workspace"], "");
    assert_eq!(
        stdout_lines(&output),
        [a_type_params_doc, b_symbol_order.clone()]
    );

    let output = run(&directory, &["--workspace", "--skip-tests=DOC003"], "");
    assert_eq!(stdout_lines(&output), [a_symbol_order, b_symbol_order]);

    write_file(
        &directory,
        "b/Cargo.toml",
        &package_manifest(
            "b",
            "\n[package.metadata.yscope-linter]\nskip-tests = [\"LNT001\"]\n",
        ),
    );
    let output = run(&directory, &["--workspace"], "");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("b/Cargo.toml`: `LNT001` is not a rule and can't be selected"),
        "{stderr}"
    );
}

#[test]
fn diff_checks_only_the_changed_lines_of_the_selected_packages() {
    let directory =
        virtual_workspace("diff_checks_only_the_changed_lines_of_the_selected_packages");
    let two_suppressions = format!("{INVALID_SUPPRESSION}{INVALID_SUPPRESSION}");
    write_file(&directory, "a/src/lib.rs", &two_suppressions);
    write_file(&directory, "b/src/lib.rs", &two_suppressions);
    let diff_text = "\
--- a/a/src/lib.rs
+++ b/a/src/lib.rs
@@ -2 +2 @@
-old
+// yscope-linter: allow(errors-doc)
--- a/b/src/lib.rs
+++ b/b/src/lib.rs
@@ -1 +1 @@
-old
+// yscope-linter: allow(errors-doc)
--- a/tmp/stray.rs
+++ b/tmp/stray.rs
@@ -1 +1 @@
-old
+// yscope-linter: allow(errors-doc)
";
    let locations = |output: &Output| -> Vec<String> {
        stdout_lines(output)
            .into_iter()
            .map(|line| {
                line.split(": ")
                    .next()
                    .expect("a diagnostic has a location")
                    .to_owned()
            })
            .collect()
    };
    let output = run(&directory, &["--diff", "-"], diff_text);
    assert_eq!(locations(&output), ["a/src/lib.rs:2:1"]);
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "yscope-linter: 1 diagnostic in 1 file checked, 0 generated files skipped\n"
    );

    let output = run(&directory, &["--diff", "-", "--workspace"], diff_text);
    assert_eq!(locations(&output), ["a/src/lib.rs:2:1", "b/src/lib.rs:1:1"]);

    let output = run(&directory, &["--diff", "-", "-p", "b"], "");
    assert_eq!(output.status.code(), Some(0));

    let subdirectory_diff_text = "\
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -2 +2 @@
-old
+// yscope-linter: allow(errors-doc)
";
    let output = run(
        &directory.join("a"),
        &["--diff", "-"],
        subdirectory_diff_text,
    );
    assert_eq!(locations(&output), ["src/lib.rs:2:1"]);
}

#[test]
fn github_format_paths_are_relative_to_the_current_directory() {
    let directory = virtual_workspace("github_format_paths_are_relative_to_the_current_directory");
    let output = run(&directory, &["--format", "github"], "");
    let lines = stdout_lines(&output);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("::error file=a/src/lib.rs,line=1,col=1,"),
        "{lines:?}"
    );
}

#[test]
fn fatal_errors_exit_two_with_empty_stdout() {
    let directory = virtual_workspace("fatal_errors_exit_two_with_empty_stdout");
    write_file(
        &directory,
        "invalid/Cargo.toml",
        "[workspace]\nmembers = [\"c\"]\n",
    );
    write_file(
        &directory,
        "invalid/c/Cargo.toml",
        &package_manifest("c", "\n[package.metadata.yscope-linter]\nwidht = 80\n"),
    );
    write_file(&directory, "invalid/c/src/lib.rs", "");
    write_file(
        &directory,
        "invalid-workspace/Cargo.toml",
        "[workspace]\nmembers = [\"d\"]\n\n[workspace.metadata.yscope-linter]\nwidht = 80\n",
    );
    write_file(
        &directory,
        "invalid-workspace/d/Cargo.toml",
        &package_manifest("d", ""),
    );
    write_file(&directory, "invalid-workspace/d/src/lib.rs", "");
    let cases: [(&[&str], &str); 5] = [
        (
            &["--manifest-path", "missing/Cargo.toml"],
            "yscope-linter: error: `cargo metadata` failed (exit status: 101)\n",
        ),
        (
            &["-p", "nope"],
            "yscope-linter: error: package `nope` not found in the workspace",
        ),
        (
            &["--workspace", "--exclude-package", "nope"],
            "yscope-linter: error: package `nope` not found in the workspace",
        ),
        (
            &["--manifest-path", "invalid/Cargo.toml"],
            "yscope-linter: error: invalid `[package.metadata.yscope-linter]` in",
        ),
        (
            &["--manifest-path", "invalid-workspace/Cargo.toml"],
            "yscope-linter: error: invalid `[workspace.metadata.yscope-linter]` in",
        ),
    ];
    for (args, message) in cases {
        let output = run(&directory, args, "");
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert_eq!(stdout_lines(&output), Vec::<String>::new(), "{args:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(message), "{args:?}: {stderr}");
    }

    let output = run(&directory, &["--manifest-path", "missing/Cargo.toml"], "");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.starts_with("error: manifest path"), "{stderr}");

    let output = run(&directory, &["--manifest-path", "invalid/Cargo.toml"], "");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid/c/Cargo.toml`: unknown field `widht`"),
        "{stderr}"
    );

    let output = run(
        &directory,
        &["--manifest-path", "invalid-workspace/Cargo.toml"],
        "",
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid-workspace/Cargo.toml`: unknown field `widht`"),
        "{stderr}"
    );

    let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-yscope-linter"));
    command
        .arg("yscope-linter")
        .env("CARGO", directory.join("missing-cargo"));
    let output = support::run_command(command, &directory, "");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("yscope-linter: error: failed to run `"),
        "{stderr}"
    );

    let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-yscope-linter"));
    command.arg("--workspace");
    let output = support::run_command(command, &directory, "");
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn version_flag_prints_the_package_version() {
    let directory = test_directory("version_flag_prints_the_package_version");
    let output = run(&directory, &["--version"], "");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        stdout_lines(&output),
        [format!("cargo-yscope-linter {}", env!("CARGO_PKG_VERSION"))]
    );
}
