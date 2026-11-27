# `tuple-returns-doc`

Code: `DOC006`, in the `DOC` set. See the [rule index](README.md) for the codes of all rules.

A documented fn that returns a tuple, or a `Result` of one, must document the tuple in its
`# Returns` section with a fixed intro and one bullet per element, in order. The intro is `A tuple
containing:` for a plain tuple, and `A tuple on success, containing:` for a `Result`.

```rust
/// Resolves the terminal state for a job with the given outcome and current status.
///
/// # Returns
///
/// On success, a tuple containing:
///
/// * The resolved status.
/// * The error message to store.
fn resolve_terminal_state(
    outcome: JobOutcome,
    current_status: QueryJobStatus,
) -> Result<(QueryJobStatus, Option<String>), Error> {
    todo!()
}

/// Splits the submission into its parts.
///
/// # Returns
///
/// A tuple containing:
///
/// * The task graph.
/// * The job inputs.
fn into_parts(
    submission: Submission,
) -> (TaskGraph, TaskGraphInput, ValidatedCompressedJobSubmission) {
    todo!()
}
```

```console
$ yscope-linter example.rs
example.rs:5:1: DOC006 tuple-returns-doc: `resolve_terminal_state` returns a `Result` of a tuple; start `# Returns` with "A tuple on success, containing:"
example.rs:22:1: DOC006 tuple-returns-doc: `into_parts` returns a tuple of 3 elements, but `# Returns` lists 2 elements; add a bullet for each element, in order
```

The first section starts with the intro for a `Result`, and the second lists all three elements:

```rust
/// Resolves the terminal state for a job with the given outcome and current status.
///
/// # Returns
///
/// A tuple on success, containing:
///
/// * The resolved status.
/// * The error message to store.
fn resolve_terminal_state(
    outcome: JobOutcome,
    current_status: QueryJobStatus,
) -> Result<(QueryJobStatus, Option<String>), Error> {
    todo!()
}

/// Splits the submission into its parts.
///
/// # Returns
///
/// A tuple containing:
///
/// * The task graph.
/// * The job inputs.
/// * The compressed serializations.
fn into_parts(
    submission: Submission,
) -> (TaskGraph, TaskGraphInput, ValidatedCompressedJobSubmission) {
    todo!()
}
```

## Contents

* [Which fns are checked](#which-fns-are-checked)
* [Which return types are tuples](#which-return-types-are-tuples)
* [The section format](#the-section-format)
* [Diagnostics](#diagnostics)
* [Known limits and when to suppress](#known-limits-and-when-to-suppress)

## Which fns are checked

A fn is checked when it has a docstring with a `# Returns` section and returns a
[tuple](#which-return-types-are-tuples). Fns of every kind are checked:

* Free fns, including fns nested in other fns.
* Inherent methods, in an `impl Type` block.
* Trait method declarations, with or without a default body.
* Trait impl methods, in an `impl Trait for Type` block.

These are not checked:

* A fn without a docstring, or with only blank doc lines.
* A fn whose docstring has no `# Returns` section. The rule checks the sections that exist; it
  doesn't require one.
* Test code, if you pass `--skip-tests` or set `skip-tests = true` in the [`Cargo.toml`
  settings](../../README.md#settings): fns with a `test` attribute such as `#[test]` or
  `#[tokio::test]`, items marked `#[cfg(test)]` with everything inside them, and module files whose
  parent declares them with `#[cfg(test)] mod tests;`. See
  [`errors-doc`](errors-doc.md#which-fns-are-checked) for how such a parent is found.

The rule needs the file's syntax tree. A file that `syn` can't parse gets one `parse-error`
diagnostic that names every skipped rule, and the rule skips the file.

## Which return types are tuples

The rule reads the return type as written, so it sees a tuple only where the signature spells one
out:

* A tuple of two or more elements, such as `(A, B)`. A nested tuple counts as one element, so `(A,
  (B, C))` has two.
* A `Result` of such a tuple: a path whose last segment ends with `Result`, as in
  [`errors-doc`](errors-doc.md#which-fns-are-checked), and whose first type argument is the tuple.
  So `Result<(A, B), E>`, `anyhow::Result<(A, B)>` and `DbResult<(A, B)>` are all checked.
* Either of these as the `Output` of an `impl Future<Output = ..>` return type. An `async fn` is
  checked by its declared return type.

These are not checked:

* The unit type `()` and a one-element tuple `(A,)`, and a `Result` of either.
* A tuple inside another type, such as `Option<(A, B)>`, `Vec<(A, B)>`, `Box<(A, B)>` or `impl
  Iterator<Item = (A, B)>`.
* A tuple behind a type alias, such as `type Pair = (A, B);`, or a `Result` alias without type
  arguments.
* A `Pin<Box<dyn Future<Output = (A, B)>>>`.

```rust
/// Finds the first pair.
///
/// # Returns
///
/// The first pair, if any.
fn first_pair() -> Option<(Key, Value)> {
    todo!()
}

/// Loads the job.
///
/// # Returns
///
/// The job and its tasks.
async fn load() -> anyhow::Result<(Job, Vec<Task>)> {
    todo!()
}
```

```console
$ yscope-linter example.rs
example.rs:14:1: DOC006 tuple-returns-doc: `load` returns a `Result` of a tuple; start `# Returns` with "A tuple on success, containing:"
```

## The section format

The section follows the y-scope Rust style guide:

```text
/// # Returns
///
/// A tuple containing:
///
/// * <The first element.>
/// * <The second element.>
```

* The section starts at the `Returns` header of the smallest level and ends at the next header of
  the same or a smaller level, as the `# Errors` section does in
  [`errors-doc`](errors-doc.md#section-selection). The header's title is `Returns`, exactly:
  `# Return` isn't recognized.
* The first paragraph is exactly the intro: `A tuple containing:`, or `A tuple on success,
  containing:` for a `Result`. It may wrap over several lines.
* The intro is followed by a list with one top-level bullet per element of the tuple, in order. A
  blank line between the intro and the list is optional. Numbered items, such as `1.`, count as
  bullets too.
* A bullet may continue on the next lines, have later paragraphs indented under it, and have nested
  bullets. Nested bullets never count as elements. A nested tuple element needs one top-level bullet,
  which may list the inner elements as nested bullets.
* Text may follow the list in a later paragraph, and so may other lists, fenced code blocks and
  deeper headers such as `## Errors`. Only the list right after the intro is counted.
* A non-prose block between the intro and the list, such as a fenced code block, is ignored: the
  list after it is still the element list, and no finding points at the block.

```rust
/// Splits the request.
///
/// # Returns
///
/// A tuple containing:
///
/// * The header.
/// * The body and its trailer:
///   * The body.
///   * The trailer.
///
/// Both are owned by the caller.
fn split(request: Request) -> (Header, (Body, Trailer)) {
    todo!()
}

/// Splits the response.
///
/// # Returns
///
/// A tuple containing:
///
/// Both are owned by the caller.
///
/// * The header.
/// * The body.
fn split_response(response: Response) -> (Header, Body) {
    todo!()
}
```

```console
$ yscope-linter example.rs
example.rs:21:1: DOC006 tuple-returns-doc: `split_response` returns a tuple of 2 elements, but no list follows the `# Returns` intro; move the element bullets right after it
```

## Diagnostics

Every diagnostic has the code `tuple-returns-doc`. `f` is the fn's name, `N` the number of
elements of its tuple, and `M` the number of top-level bullets in its list.

| Message | Cause | Fix |
| ------- | ----- | --- |
| `` `f` returns a tuple; start `# Returns` with "A tuple containing:" `` | The fn returns a tuple, and the section's first paragraph isn't the intro. | Start the section with the intro. |
| `` `f` returns a `Result` of a tuple; start `# Returns` with "A tuple on success, containing:" `` | The fn returns a `Result` of a tuple, and the section's first paragraph isn't the intro. | Start the section with the intro. |
| `` `f` returns a tuple of N elements, but `# Returns` lists M elements; add a bullet for each element, in order `` | The list after the intro has fewer top-level bullets than the tuple has elements. | Add the missing bullets. |
| `` `f` returns a tuple of N elements, but `# Returns` lists M elements; keep one bullet per element, in order `` | The list after the intro has more top-level bullets than the tuple has elements. | Merge or remove bullets, or make the extra ones nested bullets. |
| `` `f` returns a tuple of N elements, but no list follows the `# Returns` intro; move the element bullets right after it `` | The intro is right, but another paragraph comes between it and the section's bullets. | Move the bullets up, and the paragraph after them. |

For a `Result`, the count messages and the "no list follows the `# Returns` intro" message say
`` `f` returns a `Result` of a tuple of N elements ``.

Where each diagnostic is reported:

* The intro messages: at the section's first paragraph, or at its first bullet if it starts with
  one, or at the `# Returns` header if the section is empty.
* The count messages: at the first bullet of the list, or at the intro if there is no list.
* The "no list follows the `# Returns` intro" message: at the intro.

The bullets are counted when the intro is right, or when a list comes right after the first
paragraph, or the section starts with one. So a section that starts with a wrong intro and has no
list gets only the intro message, and a section that starts with bullets and lacks the intro gets
both messages at its first bullet if their number is wrong.

With `--diff` or `FILE:RANGES`, a diagnostic is kept when any line of its fn is selected, from its
first attribute or doc line to its last line.

## Known limits and when to suppress

* The return type is read as written. A tuple behind a type alias isn't seen, and any path whose
  last segment ends with `Result` is taken as a `Result` with the `Ok` type first, as in
  `errors-doc`.
* Only the number of top-level bullets is checked: not their wording, and not that they come in the
  order of the elements.
* If the element list is indented deeper than a later bullet of the section, its bullets are read
  as prose, so the fn gets a "no list follows the `# Returns` intro" finding, or only the intro
  finding if the intro is also wrong.
* A continuation line indented less than its bullet's text, such as an unindented (lazy)
  continuation or a line indented two spaces under `1. `, ends the list and starts a prose
  paragraph, although rustdoc renders it as part of the bullet. The bullets after it aren't
  counted, so the fn may get a count finding it doesn't deserve. Indent continuation lines to the
  bullet's text.
* A docstring built only from `#[doc = include_str!(..)]` counts as undocumented.
* Fns generated by macros are not seen.

When the rule is wrong for a fn, suppress it with a reason:

```rust
// yscope-linter: allow(tuple-returns-doc): the elements are documented on `Split`
/// Splits the request.
///
/// # Returns
///
/// The parts, as [`Split`] describes them.
fn split(request: Request) -> (Header, Body) {
    todo!()
}
```

An own-line `allow` above the fn's docstring, or anywhere from its first attribute or doc line
through the fn's first line, covers every finding of the fn. `allow-item` above the fn covers it
too. See [Suppression comments](../suppression.md).
