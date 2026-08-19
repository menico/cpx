//! Binding a directory to a profile.
//!
//! The registry at `<root>/bindings.toml` is the index; a managed block
//! inside the directory's `.envrc` is the mechanism. Everything outside the
//! block markers belongs to the user and is preserved byte-for-byte.

use crate::config::Config;
use crate::layout::Layout;
use crate::plan::{Action, Plan, Risk};
use crate::state::sha256_bytes;
use crate::wrapper::sh_quote;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const BLOCK_BEGIN_PREFIX: &str = "# >>> cpx:";
pub const BLOCK_END: &str = "# <<< cpx <<<";

/// A managed block that is opened but never closed. Refusing is the only
/// safe option: appending to it would leave two opens and one close.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the cpx block is opened but never closed")]
pub struct UnterminatedBlock;

#[derive(Debug, thiserror::Error)]
pub enum BindError {
    #[error("{path} has a cpx block that is opened but never closed; fix it by hand and retry")]
    UnterminatedBlock { path: PathBuf },

    #[error("no profile named `{0}`")]
    UnknownProfile(String),

    #[error("{0} is not a directory")]
    NotADirectory(PathBuf),

    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// One entry in the registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub path: PathBuf,
    pub profile: String,
    /// Hash of the block as cpx wrote it, so a hand-edit is detectable
    /// without re-deriving the whole file.
    pub block_sha256: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Bindings {
    #[serde(default, rename = "bindings")]
    pub entries: Vec<Binding>,
}

/// What a registry entry looks like on disk right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingHealth {
    Healthy,
    DirectoryMissing,
    /// The registry names a profile the config no longer defines.
    ProfileMissing,
    /// The `.envrc` exists but no longer carries our block.
    BlockAbsent,
    /// The block is there but no longer matches what cpx wrote.
    BlockEdited,
    /// direnv has not been told to trust this `.envrc`.
    NotAllowed,
}

/// The plan to bind a directory, plus the registry entry to record on success.
#[derive(Debug, Clone)]
pub struct BindPlan {
    pub plan: Plan,
    pub binding: Binding,
}

/// Render the managed block for a profile.
pub fn render_block(
    name: &str,
    profile_dir: &Path,
    profile_bin_dir: &Path,
    env: &BTreeMap<String, String>,
    color: Option<&str>,
) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    writeln!(s, "{BLOCK_BEGIN_PREFIX} {name} >>>").unwrap();
    writeln!(s, "# Managed by cpx. Edit with `cpx bind` / `cpx unbind`;").unwrap();
    writeln!(s, "# anything outside these markers is left alone.").unwrap();
    writeln!(
        s,
        "export CLAUDE_CONFIG_DIR={}",
        sh_quote(&profile_dir.to_string_lossy())
    )
    .unwrap();
    writeln!(s, "export CLAUDE_PROFILE={}", sh_quote(name)).unwrap();
    if let Some(color) = color {
        writeln!(s, "export CPX_PROFILE_COLOR={}", sh_quote(color)).unwrap();
    }
    for (key, value) in env {
        writeln!(s, "export {key}={}", sh_quote(value)).unwrap();
    }
    writeln!(
        s,
        "# Puts this profile's `claude` first, so the plain command is the right account.",
    )
    .unwrap();
    writeln!(
        s,
        "PATH_add {}",
        sh_quote(&profile_bin_dir.to_string_lossy())
    )
    .unwrap();
    write!(s, "{BLOCK_END}").unwrap();
    s
}

/// Locate the managed block's line range within `existing`.
fn locate(existing: &str) -> Result<Option<(usize, usize)>, UnterminatedBlock> {
    let lines: Vec<&str> = existing.lines().collect();
    let begin = lines
        .iter()
        .position(|l| l.trim_start().starts_with(BLOCK_BEGIN_PREFIX));
    let Some(begin) = begin else {
        return Ok(None);
    };
    let end = lines[begin..]
        .iter()
        .position(|l| l.trim() == BLOCK_END)
        .map(|offset| begin + offset);
    // Refuse rather than guess: appending a second block to a half-open one
    // would leave a file no future run could parse.
    end.map(|end| Some((begin, end))).ok_or(UnterminatedBlock)
}

/// The block currently present in `existing`, if any.
pub fn extract_block(existing: &str) -> Result<Option<String>, UnterminatedBlock> {
    let Some((begin, end)) = locate(existing)? else {
        return Ok(None);
    };
    let lines: Vec<&str> = existing.lines().collect();
    Ok(Some(lines[begin..=end].join("\n")))
}

/// Insert or replace the managed block, preserving everything else.
pub fn upsert_block(existing: &str, block: &str) -> Result<String, UnterminatedBlock> {
    let lines: Vec<&str> = existing.lines().collect();
    let block_lines: Vec<&str> = block.lines().collect();

    let out: Vec<&str> = match locate(existing)? {
        Some((begin, end)) => {
            let mut out = lines[..begin].to_vec();
            out.extend(block_lines);
            out.extend(&lines[end + 1..]);
            out
        }
        None => {
            let mut out = lines.clone();
            if !out.is_empty() && !out.last().is_some_and(|l| l.trim().is_empty()) {
                out.push("");
            }
            out.extend(block_lines);
            out
        }
    };

    let mut text = out.join("\n");
    text.push('\n');
    Ok(text)
}

/// Remove the managed block. `None` when there was none to remove.
pub fn remove_block(existing: &str) -> Result<Option<String>, UnterminatedBlock> {
    let Some((begin, end)) = locate(existing)? else {
        return Ok(None);
    };
    let lines: Vec<&str> = existing.lines().collect();
    let mut out = lines[..begin].to_vec();
    let tail = &lines[end + 1..];

    // Drop the blank line `upsert_block` inserts as a separator, so that
    // bind followed by unbind restores the file byte-for-byte.
    if tail.iter().all(|l| l.trim().is_empty()) {
        while out.last().is_some_and(|l| l.trim().is_empty()) {
            out.pop();
        }
    } else {
        out.extend(tail);
    }

    let mut text = out.join("\n");
    if !text.trim().is_empty() {
        text.push('\n');
    }
    Ok(Some(text))
}

/// The `info/exclude` file for the repository containing `dir`, if it is one.
///
/// `.gitignore` is deliberately not used: it is shared with everyone who
/// clones the repository, and a personal tool has no business in it.
pub fn git_info_exclude(dir: &Path) -> Option<PathBuf> {
    let dot_git = dir.join(".git");
    let meta = std::fs::symlink_metadata(&dot_git).ok()?;

    if meta.is_dir() {
        return Some(dot_git.join("info/exclude"));
    }
    // A worktree's `.git` is a file pointing at the real git directory.
    let text = std::fs::read_to_string(&dot_git).ok()?;
    let gitdir = text.lines().find_map(|l| l.strip_prefix("gitdir:"))?;
    Some(PathBuf::from(gitdir.trim()).join("info/exclude"))
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> BindError + '_ {
    move |source| BindError::Io {
        path: path.to_path_buf(),
        source,
    }
}

impl Bindings {
    pub fn load(path: &Path) -> Result<Bindings, BindError> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|e| BindError::Io {
                path: path.to_path_buf(),
                source: std::io::Error::new(std::io::ErrorKind::InvalidData, e),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Bindings::default()),
            Err(e) => Err(io(path)(e)),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), BindError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io(path))?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| BindError::Io {
            path: path.to_path_buf(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, e),
        })?;
        std::fs::write(path, text).map_err(io(path))
    }

    pub fn get(&self, dir: &Path) -> Option<&Binding> {
        self.entries.iter().find(|b| b.path == dir)
    }

    pub fn upsert(&mut self, binding: Binding) {
        self.entries.retain(|b| b.path != binding.path);
        self.entries.push(binding);
        self.entries.sort_by(|a, b| a.path.cmp(&b.path));
    }

    pub fn remove(&mut self, dir: &Path) -> Option<Binding> {
        let index = self.entries.iter().position(|b| b.path == dir)?;
        Some(self.entries.remove(index))
    }
}

/// Plan binding `dir` to `profile_name`.
pub fn plan_bind(
    config: &Config,
    layout: &Layout,
    profile_name: &str,
    dir: &Path,
) -> Result<BindPlan, BindError> {
    let profile = config
        .profiles
        .get(profile_name)
        .ok_or_else(|| BindError::UnknownProfile(profile_name.to_string()))?;
    if !dir.is_dir() {
        return Err(BindError::NotADirectory(dir.to_path_buf()));
    }

    let block = render_block(
        profile_name,
        &config.config_dir(layout, profile_name),
        &layout.profile_bin_dir(profile_name),
        &profile.env,
        profile.color.as_deref(),
    );

    let envrc = dir.join(".envrc");
    let existing = std::fs::read_to_string(&envrc).unwrap_or_default();
    let content = upsert_block(&existing, &block).map_err(|_| BindError::UnterminatedBlock {
        path: envrc.clone(),
    })?;

    let mut plan = Plan::default();
    plan.push(
        Action::WriteEnvrcBlock {
            envrc: envrc.clone(),
            content,
        },
        // Everything outside the markers is preserved, so this never
        // displaces anything the user wrote.
        Risk::Safe,
        format!("bind {} to profile {profile_name}", dir.display()),
        Some(profile_name),
    );

    if let Some(exclude) = git_info_exclude(dir) {
        plan.push(
            Action::GitInfoExclude {
                repo: exclude,
                line: ".envrc".to_string(),
            },
            Risk::Safe,
            "ignore .envrc locally (.git/info/exclude, not .gitignore)".to_string(),
            Some(profile_name),
        );
    }

    plan.push(
        Action::RunDirenvAllow {
            dir: dir.to_path_buf(),
        },
        Risk::Safe,
        format!("direnv allow {}", dir.display()),
        None,
    );

    Ok(BindPlan {
        plan,
        binding: Binding {
            path: dir.to_path_buf(),
            profile: profile_name.to_string(),
            block_sha256: sha256_bytes(block.as_bytes()),
        },
    })
}

/// Plan removing the binding on `dir`.
pub fn plan_unbind(dir: &Path) -> Result<Plan, BindError> {
    let envrc = dir.join(".envrc");
    let existing = std::fs::read_to_string(&envrc).unwrap_or_default();

    let mut plan = Plan::default();
    let removed = remove_block(&existing).map_err(|_| BindError::UnterminatedBlock {
        path: envrc.clone(),
    })?;
    if removed.is_none() {
        plan.note(format!("{} has no cpx block", envrc.display()));
        return Ok(plan);
    }

    plan.push(
        Action::RemoveEnvrcBlock {
            envrc: envrc.clone(),
        },
        Risk::Safe,
        format!("unbind {}", dir.display()),
        None,
    );
    plan.push(
        Action::RunDirenvAllow {
            dir: dir.to_path_buf(),
        },
        Risk::Safe,
        format!("direnv allow {}", dir.display()),
        None,
    );
    Ok(plan)
}

/// Assess a registry entry against what is actually on disk.
pub fn health(binding: &Binding, config: &Config) -> BindingHealth {
    if !binding.path.is_dir() {
        return BindingHealth::DirectoryMissing;
    }
    if !config.profiles.contains_key(&binding.profile) {
        return BindingHealth::ProfileMissing;
    }

    let envrc = binding.path.join(".envrc");
    let Ok(text) = std::fs::read_to_string(&envrc) else {
        return BindingHealth::BlockAbsent;
    };
    let block = match extract_block(&text) {
        Ok(Some(block)) => block,
        _ => return BindingHealth::BlockAbsent,
    };
    if sha256_bytes(block.trim_end().as_bytes()) != binding.block_sha256 {
        return BindingHealth::BlockEdited;
    }
    if direnv_allows(&binding.path) == Some(false) {
        return BindingHealth::NotAllowed;
    }
    BindingHealth::Healthy
}

/// Whether direnv trusts this directory's `.envrc`.
/// `None` when direnv is not installed or did not answer.
pub fn direnv_allows(dir: &Path) -> Option<bool> {
    let out = std::process::Command::new("direnv")
        .arg("status")
        .arg("--json")
        .current_dir(dir)
        .output()
        .ok()?;
    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    // direnv reports 0 for allowed, non-zero for not allowed or denied.
    let allowed = parsed.get("state")?.get("foundRC")?.get("allowed")?.as_i64()?;
    Some(allowed == 0)
}
