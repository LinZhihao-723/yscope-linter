# yscope-linter

`yscope-linter` checks the y-scope Rust style rules that neither rustfmt nor clippy can check:

* Comment paragraphs are filled greedily, so no line leaves room for the next line's first word.
* A fn's `# Errors` docstring section matches its body: it lists every error the fn returns directly
  and every function whose errors it forwards, and nothing else.
* The items of a module and of an inherent impl block follow the symbol ordering convention.
* A `# Type Parameters` section documents every generic parameter of an item.
* A `# Parameters` section documents every parameter of a trait method declaration, and appears
  nowhere else.
* A `# Returns` section of a fn that returns a tuple introduces the tuple with a fixed sentence and
  lists one bullet per element.

It reads the source only, without building the crate, so it can lint a whole repository, the
packages of a cargo workspace, or only the lines a pull request changes.

## Rules

| Code | Name | What it checks |
| ---- | ---- | -------------- |
| `DOC001` | [`comment-greedy-wrap`](docs/rules/comment-greedy-wrap.md) | Every `//`, `///` and `//!` comment paragraph is filled greedily to the width (100 columns by default). |
| `DOC002` | [`errors-doc`](docs/rules/errors-doc.md) | The `# Errors` section of a fn's docstring documents every error the fn returns directly and every function whose errors it forwards, and nothing it doesn't. |
| `DOC003` | [`type-params-doc`](docs/rules/type-params-doc.md) | A documented item with generic parameters has a `# Type Parameters` section with one bullet per type, lifetime and const parameter, and nothing else. |
| `DOC004` | [`trait-params-doc`](docs/rules/trait-params-doc.md) | A documented trait method declaration has a `# Parameters` section with one bullet per parameter, and no other item has a `# Parameters` section. |
| `DOC006` | [`tuple-returns-doc`](docs/rules/tuple-returns-doc.md) | The `# Returns` section of a documented fn that returns a tuple, or a `Result` of one, starts with "A tuple containing:" or "A tuple on success, containing:" and lists one bullet per element. |
| `ORG001` | [`symbol-order`](docs/rules/symbol-order.md) | The items of each module are grouped by visibility (`pub` first, private last), and within a group ordered consts, statics, types, then fns, with each type directly followed by its impl blocks. Inside an inherent impl, associated consts come before fns, and `pub` fns before private ones. |

Each rule has a stable code: a set prefix, `DOC` for comments and docstrings or `ORG` for code
organization, and a number. A code is never renumbered or reused, and a removed rule is deprecated
instead. See the [rule index](docs/rules/README.md) for the naming standard and the deprecation
policy.

A rule can be silenced for a line, a comment block, an item, a region or a file with a suppression
comment that gives a reason. See [Suppression comments](docs/suppression.md).

The design and the internals are in [`docs/design.md`](docs/design.md).

## Install

```text
cargo install --locked --git https://github.com/y-scope/yscope-linter yscope-linter
```

This installs two binaries:

* `yscope-linter`, which lints the files and directories it is given. See [Usage](#usage).
* `cargo-yscope-linter`, which cargo runs for `cargo yscope-linter` to lint the packages of a cargo
  workspace. See [Cargo integration](#cargo-integration).

## Usage

```text
yscope-linter [OPTIONS] [TARGET]...
```

| Argument             | Meaning                                                                                  |
| -------------------- | ---------------------------------------------------------------------------------------- |
| `TARGET`             | A file, a directory, or `FILE:RANGES` (for example `src/a.rs:10-20,30,40-45`).           |
| `--rule <ID>`        | Enable only this rule, code (`DOC003`) or set (`DOC`). Repeatable. Default: every rule.  |
| `--width <N>`        | The fill width for `comment-greedy-wrap`. Default: 100.                                  |
| `--skip-tests`       | Skip test code in every rule but `comment-greedy-wrap`. Default: test code is checked.   |
| `--no-skip-tests`    | Check test code, overriding a `skip-tests` setting.                                      |
| `--exclude <PATH>`   | Skip this file or directory, matched as a path prefix by whole components. Repeatable.   |
| `--diff <FILE>`      | Lint only the lines changed by a unified diff. `-` reads the diff from stdin.            |
| `--format <FORMAT>`  | `text` (default) or `github` (GitHub Actions `::error` workflow commands).               |

* Directories are walked recursively for `.rs` files, skipping `target/` and hidden directories.
* A file named explicitly is always checked.
* `--rule` matches case-sensitively: names are lowercase, and codes and sets uppercase. A set names
  every rule of the set that isn't deprecated. A deprecated rule named by name or code is skipped
  with a warning on stderr. See [Selecting rules by name, code or
  set](docs/rules/README.md#selecting-rules-by-name-code-or-set).
* A file with a plain `//` comment holding the word `@generated` in its first five lines is skipped.
  The summary on stderr counts it separately.
* Test code is a fn with an attribute whose last path segment is `test`, such as `#[test]` or
  `#[tokio::test]`, an item marked `#[cfg(test)]` with everything inside it, and a module file whose
  parent declares it with `#[cfg(test)] mod tests;`.
* Of `--skip-tests` and `--no-skip-tests`, the last one given wins.
* `--include-tests` is deprecated and has no effect, since test code is checked by default. It
  prints a warning on stderr.

> [!IMPORTANT]
> Test code is checked by default. Before, `errors-doc`, `symbol-order`, `trait-params-doc` and
> `type-params-doc` skipped it unless `--include-tests` was given. To keep the old output, pass
> `--skip-tests`, or set `skip-tests = true` in the [`Cargo.toml` settings](#settings).

Each diagnostic is one line on stdout, sorted by path, line and column:

```text
<path>:<line>:<column>: <code> <name>: <message>
```

> [!IMPORTANT]
> The code is new, so a parser of the old output needs updating. The old and new shapes are:
>
> ```text
> <path>:<line>:<column>: <name>: <message>
> <path>:<line>:<column>: <code> <name>: <message>
> ```
>
> The GitHub annotation title changes from `<name>` to `<code> <name>` in the same way.

For example:

```text
src/cache.rs:12:1: DOC003 type-params-doc: struct `Cache` has generic parameters but no `# Type Parameters` section; add one with a bullet for each of `Value`
```

With `--format github`, the title of each annotation is `<code> <name>` too.

Besides the rules, these meta codes are always on and can't be selected or suppressed:

* `LNT001 parse-error`: a file that can't be lexed, or that `syn` can't parse.
* `LNT002 invalid-suppression`: a malformed or misplaced suppression comment.
* `LNT003 unused-suppression`: a suppression comment that covered no finding, or that names a
  deprecated rule.

Exit codes:

* `0`: no diagnostics.
* `1`: at least one diagnostic.
* `2`: a usage error, an invalid target or diff, or an I/O failure. Nothing is printed to stdout.

`yscope-linter` doesn't read `Cargo.toml`, so the settings of
[Cargo integration](#cargo-integration) don't apply to it.

## Cargo integration

```text
cargo yscope-linter [OPTIONS]
```

`cargo yscope-linter` lints the packages of a cargo workspace. It finds them with `cargo metadata`,
so it lints the workspace members' files and nothing else in the repository, and it reads its
settings from their `Cargo.toml` files.

It takes the options of `yscope-linter`, which keep their meaning, and cargo's package selection
options, but no `TARGET`:

| Argument                   | Meaning                                                                     |
| -------------------------- | --------------------------------------------------------------------------- |
| `--manifest-path <PATH>`   | The `Cargo.toml` of the workspace or package. Default: found as cargo does. |
| `-p`, `--package <NAME>`   | Lint only this workspace member. Repeatable.                                |
| `--workspace`              | Lint every workspace member. It takes precedence over `-p`, as in cargo.    |
| `--exclude-package <NAME>` | With `--workspace`, skip this member. Repeatable.                           |

* Without `-p` or `--workspace`, the packages are the ones cargo selects for the manifest:
  * The manifest's package, if the manifest isn't the workspace root.
  * Else the workspace's `default-members`, if it sets them.
  * Else the root package, or every member of a virtual workspace.
* `--exclude-package` is named so to keep `--exclude <PATH>` for paths. Unlike cargo, which only
  warns, an unknown name in `-p` or `--exclude-package` is an error.
* Each selected package's directory is walked for `.rs` files as a directory `TARGET` is, so all its
  targets are linted: the library, binaries, tests, examples, benches and the build script. The walk
  also skips the workspace's target directory, and the directories with their own `Cargo.toml`,
  which hold other packages. A nested workspace member is linted once, as its own package, if it is
  selected.
* `--exclude <PATH>` and the paths of the diagnostics are relative to the current directory.
* With `--diff`, only the changed files of the selected packages are linted.
* A `TARGET` is a usage error. To lint some files only, run `yscope-linter` on them, or use
  `--diff`.

### Settings

A workspace and each package can set defaults for the options in their `Cargo.toml`:

```toml
[workspace.metadata.yscope-linter]
width = 100
skip-tests = true
exclude = ["components/proto/src/generated"]
rules = ["DOC", "symbol-order"]

[package.metadata.yscope-linter]
width = 80
```

| Key             | Type              | Meaning                                                                                |
| --------------- | ----------------- | -------------------------------------------------------------------------------------- |
| `width`         | positive integer  | As `--width`.                                                                          |
| `skip-tests`    | boolean           | As `--skip-tests` if `true`, and as `--no-skip-tests` if `false`.                      |
| `exclude`       | array of paths    | As `--exclude`, with paths relative to the directory of the `Cargo.toml` that sets it. |
| `rules`         | array of rule IDs | As `--rule`: names, codes or sets. An empty array enables no rule.                     |

* The `[workspace.metadata.yscope-linter]` table is read from the workspace root's `Cargo.toml`. A
  package's `[package.metadata.yscope-linter]` table applies to that package's files only.
* Each option comes from the first of the command line, the package's table and the workspace's
  table that sets it, and else from the default. `--skip-tests` and `--no-skip-tests` set
  `skip-tests` either way, so the command line overrides a table in both directions.
* Excludes add up: the command line's, the package's and the workspace's all apply.
* An unknown key, a value of the wrong type, and an unknown rule are errors that name the table and
  its `Cargo.toml`. The other keys of `[workspace.metadata]` and `[package.metadata]` belong to
  other tools and are ignored.

### Errors

These are fatal errors, with exit code `2` and nothing printed to stdout:

* Cargo can't be run. The subcommand runs the cargo that `$CARGO` names, as cargo sets it, or else
  `cargo`.
* `cargo metadata` fails. Its own error is printed to stderr first.
* An invalid setting, or an unknown package name.

## Using it in CI

To lint only the lines a branch changes, pipe a zero-context diff into `--diff -` from the
repository root:

```text
git diff -U0 origin/main -- '*.rs' | yscope-linter --diff -
```

On GitHub Actions, `--format github` turns each diagnostic into an annotation on the pull request:

```text
git diff -U0 origin/main -- '*.rs' | yscope-linter --diff - --format github
```

To lint only the changed lines of a cargo workspace's packages, with their `Cargo.toml` settings:

```text
git diff -U0 origin/main -- '*.rs' | cargo yscope-linter --workspace --diff - --format github
```

* The diff's `+++` paths must have the `b/` prefix. If your Git config changes the prefixes, add
  `--no-ext-diff --src-prefix=a/ --dst-prefix=b/` to the `git diff` command.
* A diff with context lines (without `-U0`) is accepted too, but its context lines are linted as if
  they had changed.
* A finding is reported when it touches a changed line. For `errors-doc`, that is any line of the
  fn, so deleting a bullet reports the forward it documented.
* Suppressions apply before the line filter. See [Suppressions with `--diff` and
  `FILE:RANGES`](docs/suppression.md#suppressions-with---diff-and-fileranges).
