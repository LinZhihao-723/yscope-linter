//! Running `cargo metadata` and selecting the workspace members it lists.

use std::ffi::OsString;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;

use serde::Deserialize;

use crate::config::PackageSelection;
use crate::error::Error;

/// A cargo workspace, as `cargo metadata --no-deps` describes it.
#[derive(Debug, Deserialize)]
pub struct Workspace {
    /// The workspace members, which are the only packages listed without dependencies.
    packages: Vec<Package>,

    #[serde(rename = "workspace_default_members")]
    default_members: Vec<PackageId>,

    #[serde(rename = "workspace_root")]
    pub root: PathBuf,

    pub target_directory: PathBuf,

    /// The `[workspace.metadata]` table.
    metadata: Option<MetadataTable>,
}

impl Workspace {
    /// Runs `cargo metadata --no-deps` with the cargo that `$CARGO` names, or `cargo` if it is
    /// unset. Cargo's stderr is passed through.
    ///
    /// # Returns
    ///
    /// The workspace of `manifest_path`, or of the current directory if it is `None`, on success.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * [`Error::RunCargo`] if cargo can't be started.
    /// * [`Error::CargoMetadataFailed`] if `cargo metadata` exits with a failure.
    /// * Forwards [`Self::from_json`]'s return values on failure.
    pub fn load(manifest_path: Option<&Path>) -> Result<Self, Error> {
        let program = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
        let mut command = Command::new(&program);
        command.args(["metadata", "--no-deps", "--format-version", "1"]);
        if let Some(manifest_path) = manifest_path {
            command.arg("--manifest-path").arg(manifest_path);
        }
        let output = command
            .stdin(Stdio::null())
            .stderr(Stdio::inherit())
            .output()
            .map_err(|source| Error::RunCargo { program, source })?;
        if !output.status.success() {
            return Err(Error::CargoMetadataFailed {
                status: output.status,
            });
        }
        Self::from_json(&output.stdout)
    }

    /// # Returns
    ///
    /// The path of the workspace's root `Cargo.toml`.
    pub fn manifest_path(&self) -> ManifestPath {
        ManifestPath {
            path: self.root.join("Cargo.toml"),
            directory: self.root.clone(),
        }
    }

    /// # Returns
    ///
    /// The `[workspace.metadata.yscope-linter]` table, if there is one.
    pub fn settings_table(&self) -> Option<&serde_json::Value> {
        MetadataTable::settings_table(self.metadata.as_ref())
    }

    /// Selects workspace members as cargo does, except that an unknown name to exclude is an error
    /// rather than a warning.
    ///
    /// # Returns
    ///
    /// The selected members, in the order `cargo metadata` lists them, on success.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * [`Error::PackageNotFound`] if `selection` names a package that isn't a member.
    pub fn select(&self, selection: &PackageSelection) -> Result<Vec<&Package>, Error> {
        let names: &[String] = match selection {
            PackageSelection::Default => &[],
            PackageSelection::Packages { names } => names,
            PackageSelection::Workspace { excluded_names } => excluded_names,
        };
        if let Some(name) = names
            .iter()
            .find(|name| self.packages.iter().all(|package| package.name != **name))
        {
            return Err(Error::PackageNotFound {
                name: name.clone(),
                workspace_root: self.root.clone(),
            });
        }
        Ok(self
            .packages
            .iter()
            .filter(|package| match selection {
                PackageSelection::Default => self.default_members.contains(&package.id),
                PackageSelection::Packages { names } => names.contains(&package.name),
                PackageSelection::Workspace { excluded_names } => {
                    !excluded_names.contains(&package.name)
                }
            })
            .collect())
    }

    /// # Returns
    ///
    /// The workspace that the `cargo metadata` output `json` describes on success.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// * [`Error::InvalidCargoMetadata`] if `json` doesn't have the shape that cargo documents for
    ///   `cargo metadata --format-version 1`.
    fn from_json(json: &[u8]) -> Result<Self, Error> {
        serde_json::from_slice(json).map_err(|source| Error::InvalidCargoMetadata { source })
    }
}

/// A workspace member.
#[derive(Debug, Deserialize)]
pub struct Package {
    name: String,
    id: PackageId,
    pub manifest_path: ManifestPath,

    /// The `[package.metadata]` table.
    metadata: Option<MetadataTable>,
}

impl Package {
    /// # Returns
    ///
    /// The `[package.metadata.yscope-linter]` table, if there is one.
    pub fn settings_table(&self) -> Option<&serde_json::Value> {
        MetadataTable::settings_table(self.metadata.as_ref())
    }
}

/// The path of a `Cargo.toml`, which has a parent directory.
#[derive(Debug, Deserialize)]
#[serde(try_from = "PathBuf")]
pub struct ManifestPath {
    path: PathBuf,
    directory: PathBuf,
}

impl ManifestPath {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

impl TryFrom<PathBuf> for ManifestPath {
    type Error = ManifestPathWithoutParent;

    fn try_from(path: PathBuf) -> Result<Self, Self::Error> {
        let Some(directory) = path.parent() else {
            return Err(ManifestPathWithoutParent { path });
        };
        Ok(Self {
            directory: directory.to_path_buf(),
            path,
        })
    }
}

/// Why a path from `cargo metadata` is not a [`ManifestPath`].
#[derive(Debug)]
pub struct ManifestPathWithoutParent {
    path: PathBuf,
}

impl fmt::Display for ManifestPathWithoutParent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "manifest path `{}` has no parent directory",
            self.path.display()
        )
    }
}

/// A `[workspace.metadata]` or `[package.metadata]` table. Its other tools' keys are ignored.
#[derive(Debug, Deserialize)]
struct MetadataTable {
    /// The `yscope-linter` table, parsed later so that an error can name its manifest.
    #[serde(rename = "yscope-linter")]
    yscope_linter: Option<serde_json::Value>,
}

impl MetadataTable {
    /// # Returns
    ///
    /// The `yscope-linter` table of `metadata_table`, if both exist.
    fn settings_table(metadata_table: Option<&Self>) -> Option<&serde_json::Value> {
        metadata_table.and_then(|metadata_table| metadata_table.yscope_linter.as_ref())
    }
}

/// A package ID, which `cargo metadata` uses to list the default members.
#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
struct PackageId {
    id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds the `cargo metadata` output of a workspace with members `a`, `b` and `c`, whose
    /// default members are `default_members`.
    ///
    /// # Returns
    ///
    /// The parsed workspace.
    ///
    /// # Panics
    ///
    /// Panics if the output can't be parsed.
    fn workspace(default_members: &[&str]) -> Workspace {
        let package = |name: &str| {
            format!(
                r#"{{"name": "{name}", "id": "id-{name}", "manifest_path": "/ws/{name}/Cargo.toml",
                    "metadata": null, "version": "0.1.0"}}"#
            )
        };
        let ids = |names: &[&str]| {
            names
                .iter()
                .map(|name| format!(r#""id-{name}""#))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let json = format!(
            r#"{{"packages": [{}, {}, {}],
                "workspace_default_members": [{}], "workspace_root": "/ws",
                "target_directory": "/ws/target", "metadata": null, "version": 1}}"#,
            package("a"),
            package("b"),
            package("c"),
            ids(default_members),
        );
        Workspace::from_json(json.as_bytes()).expect("valid metadata")
    }

    fn selected_names(workspace: &Workspace, selection: &PackageSelection) -> Vec<String> {
        workspace
            .select(selection)
            .expect("valid selection")
            .into_iter()
            .map(|package| package.name.clone())
            .collect()
    }

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn default_selection_is_the_default_members() {
        assert_eq!(
            selected_names(&workspace(&["c", "a"]), &PackageSelection::Default),
            ["a", "c"]
        );
        assert_eq!(
            selected_names(&workspace(&["a", "b", "c"]), &PackageSelection::Default),
            ["a", "b", "c"]
        );
    }

    #[test]
    fn package_selection_is_the_named_members_only() {
        let selection = PackageSelection::Packages {
            names: names(&["c", "b", "c"]),
        };
        assert_eq!(selected_names(&workspace(&["a"]), &selection), ["b", "c"]);
    }

    #[test]
    fn workspace_selection_is_every_member_but_the_excluded_ones() {
        let workspace = workspace(&["a"]);
        assert_eq!(
            selected_names(
                &workspace,
                &PackageSelection::Workspace {
                    excluded_names: Vec::new()
                }
            ),
            ["a", "b", "c"]
        );
        assert_eq!(
            selected_names(
                &workspace,
                &PackageSelection::Workspace {
                    excluded_names: names(&["b"])
                }
            ),
            ["a", "c"]
        );
    }

    #[test]
    fn unknown_package_name_is_rejected() {
        let workspace = workspace(&["a"]);
        for selection in [
            PackageSelection::Packages {
                names: names(&["a", "nope"]),
            },
            PackageSelection::Workspace {
                excluded_names: names(&["nope"]),
            },
        ] {
            let error = workspace
                .select(&selection)
                .expect_err("an unknown package name");
            assert!(
                matches!(
                    &error,
                    Error::PackageNotFound { name, workspace_root }
                        if name == "nope" && workspace_root == Path::new("/ws")
                ),
                "{selection:?}: {error:?}"
            );
        }
    }

    #[test]
    fn manifest_path_without_a_parent_is_invalid_metadata() {
        let manifest_path = ManifestPath::try_from(PathBuf::from("/ws/a/Cargo.toml"))
            .expect("a manifest path with a parent");
        assert_eq!(manifest_path.path(), Path::new("/ws/a/Cargo.toml"));
        assert_eq!(manifest_path.directory(), Path::new("/ws/a"));

        for path in ["", "/"] {
            let json = format!(
                r#"{{"packages": [{{"name": "a", "id": "id-a", "manifest_path": "{path}",
                    "metadata": null}}], "workspace_default_members": [], "workspace_root": "/ws",
                    "target_directory": "/ws/target", "metadata": null}}"#
            );
            let error = Workspace::from_json(json.as_bytes())
                .expect_err("a manifest path without a parent");
            let Error::InvalidCargoMetadata { source } = &error else {
                panic!("{path:?}: unexpected error: {error:?}");
            };
            assert!(
                source
                    .to_string()
                    .contains(&format!("manifest path `{path}` has no parent directory")),
                "{path:?}: {source}"
            );
        }
    }
}
