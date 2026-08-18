//! Planning the materialization of profiles.

use crate::config::{Config, Profile, ResourceKey, ResourceMode, ResourceSpec};
use crate::layout::Layout;
use crate::merge::deep_merge;
use crate::plan::{Action, Plan, Risk};
use crate::state::{hash_path, sha256_bytes, Ownership, State};
use crate::wrapper::{shim_script, wrapper_script, WrapperContext};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("source directory {0} does not exist")]
    SourceMissing(PathBuf),

    #[error("could not inspect {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone)]
pub struct ApplyOptions {
    /// Refresh `copy` resources from source instead of leaving them alone.
    pub sync: bool,
    /// The real Claude binary, resolved once and baked into every wrapper.
    pub claude_binary: PathBuf,
}

impl Default for ApplyOptions {
    fn default() -> Self {
        ApplyOptions {
            sync: false,
            claude_binary: PathBuf::from("claude"),
        }
    }
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> PlanError + '_ {
    move |source| PlanError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// What replacing whatever is at `path` would cost.
fn risk_of(state: &State, path: &Path) -> Result<Risk, PlanError> {
    Ok(match state.classify(path).map_err(io(path))? {
        Ownership::Absent => Risk::Safe,
        Ownership::Ours => Risk::OverwritesGenerated,
        // A file we wrote and a human then edited is treated exactly like a
        // file we never wrote: their edit is the thing worth protecting.
        Ownership::OursModified | Ownership::Foreign => Risk::OverwritesForeign,
    })
}

/// Read the source JSON for a `merge` resource. A missing file merges as an
/// empty object; an unparseable one is reported and skipped rather than
/// silently discarding whatever the user has in there.
fn read_source_json(path: &Path, plan: &mut Plan) -> Value {
    match std::fs::read_to_string(path) {
        Err(_) => Value::Object(Default::default()),
        Ok(text) => match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(e) => {
                plan.note(format!(
                    "{} is not valid JSON ({e}); merging over an empty object instead",
                    path.display()
                ));
                Value::Object(Default::default())
            }
        },
    }
}

/// One resource of one profile, resolved to concrete paths.
struct ResourceTask<'a> {
    name: &'a str,
    key: ResourceKey,
    spec: &'a ResourceSpec,
    src: PathBuf,
    dst: PathBuf,
}

/// Plan one resource for one profile.
fn plan_resource(
    plan: &mut Plan,
    state: &State,
    options: &ApplyOptions,
    task: &ResourceTask,
) -> Result<(), PlanError> {
    let ResourceTask { name, key, spec, src, dst } = task;
    let (key, src, dst) = (*key, src.as_path(), dst.as_path());
    let label = key.config_name();

    match spec.mode {
        ResourceMode::Link => {
            if !src.exists() {
                plan.note(format!(
                    "{name}: skipping `{label}` — {} does not exist",
                    src.display()
                ));
                return Ok(());
            }
            // An existing symlink already pointing at the right place is
            // correct no matter who made it, so it is never a conflict.
            if let Ok(existing) = std::fs::read_link(dst) {
                if existing == src {
                    return Ok(());
                }
            }
            plan.push(
                Action::Symlink {
                    link: dst.to_path_buf(),
                    target: src.to_path_buf(),
                },
                risk_of(state, dst)?,
                format!("{name}: link {label} -> {}", src.display()),
                Some(name),
            );
        }

        ResourceMode::Copy => {
            if !src.exists() {
                plan.note(format!(
                    "{name}: skipping `{label}` — {} does not exist",
                    src.display()
                ));
                return Ok(());
            }
            let exists = std::fs::symlink_metadata(dst).is_ok();
            // `copy` seeds a resource once. Refreshing it is opt-in, because
            // the whole point of the mode is that the profile may diverge.
            if exists && !options.sync {
                return Ok(());
            }
            let action = if src.is_dir() {
                Action::CopyTree {
                    src: src.to_path_buf(),
                    dst: dst.to_path_buf(),
                }
            } else {
                Action::CopyFile {
                    src: src.to_path_buf(),
                    dst: dst.to_path_buf(),
                }
            };
            plan.push(
                action,
                risk_of(state, dst)?,
                format!("{name}: copy {label} from {}", src.display()),
                Some(name),
            );
        }

        ResourceMode::Own => {
            // Files in `own` mode are Claude's to create. Directories are
            // made up front so it has somewhere to write.
            if key.is_dir() && std::fs::symlink_metadata(dst).is_err() {
                plan.push(
                    Action::CreateDir {
                        path: dst.to_path_buf(),
                    },
                    Risk::Safe,
                    format!("{name}: create {label}/ (profile-private)"),
                    Some(name),
                );
            }
        }

        ResourceMode::Merge => {
            let patch = spec.patch.clone();
            if !src.exists() && patch.is_none() {
                // Nothing to merge and nothing to merge it over: writing an
                // empty object would invent a file the user never had.
                return Ok(());
            }

            let base = read_source_json(src, plan);
            let merged = match &patch {
                Some(patch) => deep_merge(&base, patch),
                None => base,
            };
            let mut content = serde_json::to_string_pretty(&merged)
                .expect("merged JSON is serializable");
            content.push('\n');

            let ownership = state.classify(dst).map_err(io(dst))?;
            let actual = hash_path(dst).map_err(io(dst))?;
            if ownership == Ownership::Ours
                && actual.as_deref() == Some(sha256_bytes(content.as_bytes()).as_str())
            {
                return Ok(());
            }

            plan.push(
                Action::WriteFile {
                    path: dst.to_path_buf(),
                    content,
                    executable: false,
                },
                risk_of(state, dst)?,
                match patch {
                    Some(_) => format!("{name}: write {label} (source merged with patch)"),
                    None => format!("{name}: write {label} (from source)"),
                },
                Some(name),
            );
        }
    }
    Ok(())
}

/// Plan the wrapper and the shim, which are the same script at two paths.
fn plan_scripts(
    plan: &mut Plan,
    state: &State,
    config: &Config,
    layout: &Layout,
    options: &ApplyOptions,
    name: &str,
    profile: &Profile,
) -> Result<(), PlanError> {
    let profile_dir = layout.profile_dir(name);
    let ctx = WrapperContext {
        name,
        profile,
        profile_dir: &profile_dir,
        claude_binary: &options.claude_binary,
    };

    for (path, content, what) in [
        (config.wrapper_path(name), wrapper_script(&ctx), "wrapper"),
        (layout.shim_path(name), shim_script(&ctx), "shim"),
    ] {
        let ownership = state.classify(&path).map_err(io(&path))?;
        let actual = hash_path(&path).map_err(io(&path))?;
        if ownership == Ownership::Ours
            && actual.as_deref() == Some(sha256_bytes(content.as_bytes()).as_str())
        {
            continue;
        }
        plan.push(
            Action::WriteFile {
                path: path.clone(),
                content,
                executable: true,
            },
            risk_of(state, &path)?,
            format!("{name}: write {what} {}", path.display()),
            Some(name),
        );
    }
    Ok(())
}

/// Remove wrappers cpx generated for profiles that no longer exist. Anything
/// cpx did not generate is left strictly alone, however it is named.
fn plan_stale_wrappers(
    plan: &mut Plan,
    state: &State,
    config: &Config,
) -> Result<(), PlanError> {
    let Ok(entries) = std::fs::read_dir(&config.bin_dir) else {
        return Ok(());
    };
    let live: Vec<String> = config
        .profiles
        .keys()
        .map(|n| format!("{}{}", config.wrapper_prefix, n))
        .collect();

    let mut stale: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_string();
        if !file_name.starts_with(&config.wrapper_prefix) || live.contains(&file_name) {
            continue;
        }
        let path = entry.path();
        if state.classify(&path).map_err(io(&path))? == Ownership::Ours {
            stale.push(path);
        } else {
            plan.note(format!(
                "leaving {} alone — cpx did not generate it",
                path.display()
            ));
        }
    }

    stale.sort();
    for path in stale {
        plan.push(
            Action::RemoveGenerated { path: path.clone() },
            Risk::OverwritesGenerated,
            format!("remove stale wrapper {}", path.display()),
            None,
        );
    }
    Ok(())
}

/// Compute everything that must happen for the configured profiles to exist
/// on disk exactly as declared.
pub fn plan_apply(
    config: &Config,
    layout: &Layout,
    state: &State,
    options: &ApplyOptions,
) -> Result<Plan, PlanError> {
    if !config.source_dir.exists() {
        return Err(PlanError::SourceMissing(config.source_dir.clone()));
    }

    let mut plan = Plan::default();

    for (name, profile) in &config.profiles {
        let dir = layout.profile_dir(name);
        for path in [dir.clone(), layout.profile_bin_dir(name)] {
            if std::fs::symlink_metadata(&path).is_err() {
                plan.push(
                    Action::CreateDir { path: path.clone() },
                    Risk::Safe,
                    format!("{name}: create {}", path.display()),
                    Some(name),
                );
            }
        }

        for (key, spec) in &profile.resources {
            let src = config.source_dir.join(key.target_name());
            let dst = dir.join(key.target_name());
            plan_resource(
                &mut plan,
                state,
                options,
                &ResourceTask {
                    name,
                    key: *key,
                    spec,
                    src,
                    dst,
                },
            )?;
        }

        plan_scripts(&mut plan, state, config, layout, options, name, profile)?;
    }

    plan_stale_wrappers(&mut plan, state, config)?;

    Ok(plan)
}
