//! Adopting a Claude config directory that already exists.
//!
//! A login is keyed to its config directory's path, so the only way to take
//! over a hand-rolled profile without signing in again is to manage it where
//! it already sits. That directory is someone's live working state — hundreds
//! of megabytes of sessions, plugins and history — so adoption is built to
//! change nothing in it: every resource already present becomes `own`, every
//! absent one becomes `ignore`, and the next apply writes only the wrapper
//! and the shim.

use crate::config::{ResourceKey, ResourceMode};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum AdoptError {
    #[error("{0} does not exist")]
    Missing(PathBuf),

    #[error("{0} is not a directory")]
    NotADirectory(PathBuf),

    #[error("{0} does not look like a Claude config directory (no .claude.json, settings.json, or projects/)")]
    NotAConfigDir(PathBuf),

    #[error("{0} is the source directory; profiles are built from it, not adopted from it")]
    IsSourceDir(PathBuf),

    #[error("could not derive a profile name from {0}; pass one explicitly")]
    NoName(PathBuf),
}

/// What adopting a directory would produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adoption {
    pub name: String,
    pub dir: PathBuf,
    pub resources: BTreeMap<ResourceKey, ResourceMode>,
    /// What was found, in the order a person would want to read it.
    pub found: Vec<String>,
}

/// Turn a directory name into a profile name: `.claude-hd` becomes `hd`.
pub fn derive_name(dir: &Path) -> Option<String> {
    let file_name = dir.file_name()?.to_str()?;
    let stripped = file_name
        .strip_prefix(".claude-")
        .unwrap_or_else(|| file_name.trim_start_matches('.'));
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_string())
    }
}

/// Whether `dir` carries the marks of a Claude config directory.
fn looks_like_config_dir(dir: &Path) -> bool {
    [".claude.json", "settings.json", "projects"]
        .iter()
        .any(|entry| dir.join(entry).exists())
}

/// Inspect `dir` and decide how each resource should be treated.
///
/// Everything present becomes `own` and everything absent becomes `ignore`,
/// so applying the resulting profile writes only the wrapper and the shim.
pub fn inspect(dir: &Path, source_dir: &Path, name: Option<&str>) -> Result<Adoption, AdoptError> {
    let meta = std::fs::symlink_metadata(dir)
        .map_err(|_| AdoptError::Missing(dir.to_path_buf()))?;
    if !meta.is_dir() {
        return Err(AdoptError::NotADirectory(dir.to_path_buf()));
    }

    let canonical = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let canonical_source = source_dir
        .canonicalize()
        .unwrap_or_else(|_| source_dir.to_path_buf());
    if canonical == canonical_source {
        return Err(AdoptError::IsSourceDir(dir.to_path_buf()));
    }

    if !looks_like_config_dir(dir) {
        return Err(AdoptError::NotAConfigDir(dir.to_path_buf()));
    }

    let name = match name {
        Some(name) => name.to_string(),
        None => derive_name(dir).ok_or_else(|| AdoptError::NoName(dir.to_path_buf()))?,
    };

    let mut resources = BTreeMap::new();
    let mut found = Vec::new();
    for key in ResourceKey::ALL {
        let present = dir.join(key.target_name()).exists();
        resources.insert(
            key,
            if present {
                ResourceMode::Own
            } else {
                ResourceMode::Ignore
            },
        );
        if present {
            found.push(key.target_name().to_string());
        }
    }

    Ok(Adoption {
        name,
        dir: dir.to_path_buf(),
        resources,
        found,
    })
}

/// Every directory under `home` that looks adoptable.
///
/// The source directory and cpx's own root are excluded: one is what profiles
/// are built from, the other is where they already live.
pub fn candidates(home: &Path, source_dir: &Path, cpx_root: &Path) -> Vec<Adoption> {
    let Ok(entries) = std::fs::read_dir(home) else {
        return Vec::new();
    };

    let mut found: Vec<Adoption> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path != cpx_root)
        .filter_map(|path| inspect(&path, source_dir, None).ok())
        .collect();

    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}
