//! Planning a `cargo yscope-linter` run: the files of the selected packages, each group checked
//! with the settings of its package.

use std::collections::BTreeMap;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use crate::cargo::Settings;
use crate::cargo::SettingsTable;
use crate::cargo::metadata::Workspace;
use crate::config::CargoConfig;
use crate::diff;
use crate::driver::FileGroup;
use crate::error::Error;
use crate::line_ranges::LineRanges;
use crate::source;
use crate::source::LineFilter;
use crate::source::SourceFile;

/// The files a `cargo yscope-linter` run checks.
#[derive(Debug)]
pub struct Plan {
    /// One group per selected package, with the package's settings.
    pub file_groups: Vec<FileGroup>,

    /// A warning for each deprecated rule that a `rules` setting names, since it never runs.
    pub warnings: Vec<String>,
}

impl Plan {
    /// Factory function.
    ///
    /// Plans the run of `config` from the current directory:
    ///
    /// * Each selected package's directory is walked for `.rs` files, as a directory target is. The
    ///   workspace's target directory and the directories with their own `Cargo.toml`, which hold
    ///   other packages, are skipped too.
    /// * A package's files are checked with the command-line settings, then the package's settings,
    ///   then the workspace's settings, and then the defaults. Each setting comes from the first of
    ///   them that sets it, but excludes add up.
    /// * With a diff, only the package files that it changes are checked, on their changed lines.
    /// * Paths are relative to the current directory.
    ///
    /// # Returns
    ///
    /// The plan on success.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * [`Error::Io`] if the current directory can't be read.
    /// * Forwards [`Workspace::load`]'s return values on failure.
    /// * Forwards [`Workspace::select`]'s return values on failure.
    /// * Forwards [`crate::config::DiffInput::read`]'s return values on failure.
    /// * Forwards [`diff::parse`]'s return values on failure.
    /// * Forwards [`Settings::from_table`]'s return values on failure.
    /// * Forwards [`source::walk_directory`]'s return values on failure.
    pub fn new(config: &CargoConfig) -> Result<Self, Error> {
        let current_directory = std::env::current_dir().map_err(|source| Error::Io {
            path: PathBuf::from("."),
            source,
        })?;
        let workspace = Workspace::load(config.manifest_path.as_deref())?;
        let packages = workspace.select(&config.package_selection)?;
        let changed_lines: Option<BTreeMap<PathBuf, LineRanges>> = match &config.diff_input {
            Some(diff_input) => Some(
                diff::parse(&diff_input.read()?)?
                    .into_iter()
                    .map(|(path, line_ranges)| (resolve(&current_directory, &path), line_ranges))
                    .collect(),
            ),
            None => None,
        };
        let target_directory = resolve(&current_directory, &workspace.target_directory);
        let is_skipped = |directory: &Path| {
            directory == target_directory || directory.join("Cargo.toml").is_file()
        };

        let workspace_manifest_path = workspace.manifest_path();
        let workspace_settings = Settings::from_table(
            workspace.settings_table(),
            SettingsTable::Workspace,
            &workspace_manifest_path,
        )?;
        let mut warnings = workspace_settings
            .deprecation_warnings(SettingsTable::Workspace, &workspace_manifest_path);

        let mut file_groups = Vec::new();
        for package in packages {
            let package_settings = Settings::from_table(
                package.settings_table(),
                SettingsTable::Package,
                &package.manifest_path,
            )?;
            warnings.extend(
                package_settings
                    .deprecation_warnings(SettingsTable::Package, &package.manifest_path),
            );
            let settings = workspace_settings
                .clone()
                .overridden_by(package_settings)
                .overridden_by(config.overrides.clone());
            let excludes: Vec<PathBuf> = settings
                .excludes
                .iter()
                .map(|exclude| resolve(&current_directory, exclude))
                .collect();

            let package_directory = resolve(&current_directory, package.manifest_path.directory());
            let mut walked_files = Vec::new();
            source::walk_directory(&package_directory, &is_skipped, &mut walked_files)?;
            let source_files = walked_files
                .into_iter()
                .filter(|path| !excludes.iter().any(|exclude| path.starts_with(exclude)))
                .filter_map(|path| {
                    let line_filter = match &changed_lines {
                        Some(changed_lines) => {
                            LineFilter::Ranges(changed_lines.get(&path)?.clone())
                        }
                        None => LineFilter::All,
                    };
                    Some(SourceFile {
                        path: relative_to(&path, &current_directory),
                        line_filter,
                    })
                })
                .collect();
            file_groups.push(FileGroup {
                check_config: settings.check_config(),
                source_files,
            });
        }
        Ok(Self {
            file_groups,
            warnings,
        })
    }
}

/// Joins `path` to `base_directory` and resolves its `.` and `..` components lexically.
///
/// # Returns
///
/// The resolved path, absolute if `base_directory` is.
fn resolve(base_directory: &Path, path: &Path) -> PathBuf {
    let mut resolved = PathBuf::new();
    for component in base_directory.join(path).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                resolved.push(component);
            }
        }
    }
    resolved
}

/// # Returns
///
/// `path` relative to `base_directory`, through `..` components where `path` is outside it, or
/// `path` itself if the two share no root. Both are resolved, absolute paths.
fn relative_to(path: &Path, base_directory: &Path) -> PathBuf {
    let path_components: Vec<Component<'_>> = path.components().collect();
    let base_components: Vec<Component<'_>> = base_directory.components().collect();
    let num_shared_components = path_components
        .iter()
        .zip(&base_components)
        .take_while(|(path_component, base_component)| path_component == base_component)
        .count();
    if num_shared_components == 0 {
        return path.to_path_buf();
    }
    let relative: PathBuf = std::iter::repeat_n(
        Component::ParentDir,
        base_components.len() - num_shared_components,
    )
    .chain(path_components[num_shared_components..].iter().copied())
    .collect();
    if relative.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        relative
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_joins_and_drops_dot_components_lexically() {
        let base_directory = Path::new("/ws/a");
        assert_eq!(
            resolve(base_directory, Path::new("./src/../b/./c.rs")),
            PathBuf::from("/ws/a/b/c.rs")
        );
        assert_eq!(
            resolve(base_directory, Path::new("../../../x")),
            PathBuf::from("/x")
        );
        assert_eq!(
            resolve(base_directory, Path::new("/abs/./y")),
            PathBuf::from("/abs/y")
        );
    }

    #[test]
    fn relative_to_walks_up_to_the_shared_ancestor() {
        let base_directory = Path::new("/ws/a/src");
        for (path, expected) in [
            ("/ws/a/src/lib.rs", "lib.rs"),
            ("/ws/a/src", "."),
            ("/ws/a/tests/t.rs", "../tests/t.rs"),
            ("/ws/b/src/lib.rs", "../../b/src/lib.rs"),
        ] {
            assert_eq!(
                relative_to(Path::new(path), base_directory),
                PathBuf::from(expected),
                "{path}"
            );
        }
    }
}
