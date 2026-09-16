//! Adopting a Claude or Codex directory that already exists.
//!
//! A login is keyed to its directory's path, so the only way to take over a
//! hand-rolled profile without signing in again is to manage it where it
//! already sits. That directory is someone's live working state — hundreds
//! of megabytes of sessions, plugins and history — so adoption is built to
//! change nothing in it: every resource already present becomes `own`, every
//! absent one becomes `ignore`, and the next apply writes only the wrapper
//! and the shim.

use crate::config::{Config, Provider, ResourceKey, ResourceMode};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum AdoptError {
    #[error("{0} does not exist")]
    Missing(PathBuf),

    #[error("{0} is not a directory")]
    NotADirectory(PathBuf),

    #[error("{0} does not look like a Claude config directory (no .claude.json, settings.json, or projects/) or a Codex home (no auth.json or config.toml)")]
    NotAConfigDir(PathBuf),

    #[error("{0} is a source directory; profiles are built from it, not adopted from it")]
    IsSourceDir(PathBuf),

    #[error("could not derive a profile name from {0}; pass one explicitly")]
    NoName(PathBuf),
}

/// What adopting a directory would produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adoption {
    pub name: String,
    pub provider: Provider,
    pub dir: PathBuf,
    pub resources: BTreeMap<ResourceKey, ResourceMode>,
    /// What was found, in the order a person would want to read it.
    pub found: Vec<String>,
}

/// Turn a directory name into a profile name: `.claude-hd` and `.codex-hd`
/// both become `hd`.
pub fn derive_name(dir: &Path) -> Option<String> {
    let file_name = dir.file_name()?.to_str()?;
    let stripped = Provider::ALL
        .iter()
        .find_map(|p| file_name.strip_prefix(p.dir_prefix()))
        .unwrap_or_else(|| file_name.trim_start_matches('.'));
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_string())
    }
}

/// Written by Claude Code and by nothing else, so one is conclusive.
const CLAUDE_CONCLUSIVE: &[&str] = &[".claude.json"];

/// Individually inconclusive — other tools keep a `settings.json` or a
/// `projects/` too — so two are required together.
const CLAUDE_SUPPORTING: &[&str] = &[
    "projects",
    "history.jsonl",
    "sessions",
    "shell-snapshots",
    "todos",
    "statsig",
    ".credentials.json",
    "settings.json",
];

/// Codex's login file, which nothing else keeps at the top of a directory.
const CODEX_CONCLUSIVE: &[&str] = &["auth.json"];

/// A `config.toml` alone could be anyone's; with sessions or a version
/// stamp beside it, it is Codex's.
const CODEX_SUPPORTING: &[&str] = &[
    "config.toml",
    "sessions",
    "history.jsonl",
    "version.json",
    "installation_id",
    "log",
];

fn has_marks(dir: &Path, conclusive: &[&str], supporting: &[&str]) -> bool {
    if conclusive.iter().any(|entry| dir.join(entry).exists()) {
        return true;
    }
    supporting
        .iter()
        .filter(|entry| dir.join(entry).exists())
        .count()
        >= 2
}

/// Which tool, if any, `dir` belongs to.
///
/// Deliberately strict: a home directory is full of dot-directories, and
/// `~/.cursor` has a `projects/` while `~/.gemini` has a `settings.json`.
/// Offering to adopt those as profiles is worse than missing one. Claude
/// is checked first: its marks are the more specific.
pub fn provider_of_dir(dir: &Path) -> Option<Provider> {
    if has_marks(dir, CLAUDE_CONCLUSIVE, CLAUDE_SUPPORTING) {
        return Some(Provider::Claude);
    }
    if has_marks(dir, CODEX_CONCLUSIVE, CODEX_SUPPORTING) {
        return Some(Provider::Codex);
    }
    None
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Inspect `dir` and decide how each resource should be treated.
///
/// Everything present becomes `own` and everything absent becomes `ignore`,
/// so applying the resulting profile writes only the wrapper and the shim.
pub fn inspect(dir: &Path, config: &Config, name: Option<&str>) -> Result<Adoption, AdoptError> {
    let meta = std::fs::symlink_metadata(dir)
        .map_err(|_| AdoptError::Missing(dir.to_path_buf()))?;
    if !meta.is_dir() {
        return Err(AdoptError::NotADirectory(dir.to_path_buf()));
    }

    let here = canonical(dir);
    if config.source_dirs().iter().any(|s| canonical(s) == here) {
        return Err(AdoptError::IsSourceDir(dir.to_path_buf()));
    }

    let provider = provider_of_dir(dir).ok_or_else(|| AdoptError::NotAConfigDir(dir.to_path_buf()))?;

    let name = match name {
        Some(name) => name.to_string(),
        None => derive_name(dir).ok_or_else(|| AdoptError::NoName(dir.to_path_buf()))?,
    };

    let mut resources = BTreeMap::new();
    let mut found = Vec::new();
    for key in ResourceKey::for_provider(provider) {
        let present = dir.join(key.target_name()).exists();
        resources.insert(
            *key,
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
        provider,
        dir: dir.to_path_buf(),
        resources,
        found,
    })
}

/// Every directory under `home` that looks adoptable.
///
/// The source directories and cpx's own root are excluded: one is what
/// profiles are built from, the other is where they already live. When a
/// Claude directory and a Codex directory would take the same name, the
/// Codex one is suffixed, so `~/.claude-work` and `~/.codex-work` become
/// `work` and `work-codex` rather than colliding.
pub fn candidates(home: &Path, config: &Config, cpx_root: &Path) -> Vec<Adoption> {
    let Ok(entries) = std::fs::read_dir(home) else {
        return Vec::new();
    };

    let mut found: Vec<Adoption> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path != cpx_root)
        .filter_map(|path| inspect(&path, config, None).ok())
        .collect();

    found.sort_by(|a, b| a.provider.cmp(&b.provider).then(a.name.cmp(&b.name)));
    let mut taken: Vec<String> = Vec::new();
    for adoption in &mut found {
        if taken.contains(&adoption.name) {
            adoption.name = format!("{}-{}", adoption.name, adoption.provider.as_str());
        }
        taken.push(adoption.name.clone());
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}
