//! Running a plan.

use crate::plan::{Action, Plan, Risk};
use crate::state::{hash_path, sha256_bytes, Ownership, State};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("refusing to run: {count} action(s) would write inside the source directory {dir}")]
    WritesIntoSource { dir: PathBuf, count: usize },

    #[error("refusing to overwrite {count} file(s) cpx did not write; re-run with --force to back them up and continue")]
    ForceRequired { count: usize },

    #[error("{path} is not a file cpx generated, so it will not be removed")]
    NotOurs { path: PathBuf },

    #[error("{action} failed on {path}: {source}")]
    Io {
        action: String,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, Default)]
pub struct ExecuteOptions {
    /// Permit overwriting files cpx did not write. Each is backed up first.
    pub force: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ExecuteReport {
    /// One line per action performed, in order.
    pub performed: Vec<String>,
    /// Paths moved aside, as (original, backup).
    pub backups: Vec<(PathBuf, PathBuf)>,
}

/// Run every action in `plan`, updating `state` to record what was written.
///
/// The plan is validated in full before any action runs, so a rejected plan
/// leaves the filesystem exactly as it was.
fn io<'a>(action: &'a str, path: &'a Path) -> impl FnOnce(std::io::Error) -> ExecError + 'a {
    move |source| ExecError::Io {
        action: action.to_string(),
        path: path.to_path_buf(),
        source,
    }
}

/// Reject the plan as a whole before touching anything.
fn validate(plan: &Plan, source_dir: &Path, options: &ExecuteOptions) -> Result<(), ExecError> {
    let intruding = plan
        .actions
        .iter()
        .filter(|a| a.action.target().starts_with(source_dir))
        .count();
    if intruding > 0 {
        return Err(ExecError::WritesIntoSource {
            dir: source_dir.to_path_buf(),
            count: intruding,
        });
    }

    if !options.force {
        let foreign = plan
            .actions
            .iter()
            .filter(|a| a.risk == Risk::OverwritesForeign)
            .count();
        if foreign > 0 {
            return Err(ExecError::ForceRequired { count: foreign });
        }
    }
    Ok(())
}

/// The first free `<path>.cpx.bak`, `<path>.cpx.bak.2`, ... so no rescue ever
/// overwrites an earlier one.
fn backup_path(path: &Path) -> PathBuf {
    let base = format!("{}.cpx.bak", path.display());
    let first = PathBuf::from(&base);
    if fs::symlink_metadata(&first).is_err() {
        return first;
    }
    (2..)
        .map(|n| PathBuf::from(format!("{base}.{n}")))
        .find(|p| fs::symlink_metadata(p).is_err())
        .expect("an unused backup name exists")
}

/// Move whatever is at `path` aside. cpx never destroys anything it did not
/// write, so every overwrite of foreign content goes through here first.
fn rescue(path: &Path, report: &mut ExecuteReport) -> Result<(), ExecError> {
    if fs::symlink_metadata(path).is_err() {
        return Ok(());
    }
    let to = backup_path(path);
    fs::rename(path, &to).map_err(io("backup", path))?;
    report
        .performed
        .push(format!("backed up {} -> {}", path.display(), to.display()));
    report.backups.push((path.to_path_buf(), to));
    Ok(())
}

/// Clear the way for a new artifact at `path`, backing up anything foreign.
fn clear(path: &Path, risk: Risk, report: &mut ExecuteReport) -> Result<(), ExecError> {
    match fs::symlink_metadata(path) {
        Err(_) => Ok(()),
        Ok(meta) => {
            if risk == Risk::OverwritesForeign {
                return rescue(path, report);
            }
            // Ours and superseded: replacing a symlink or directory in place
            // is not possible, so it has to come out first.
            if meta.is_symlink() {
                fs::remove_file(path).map_err(io("replace", path))
            } else if meta.is_dir() {
                fs::remove_dir_all(path).map_err(io("replace", path))
            } else {
                Ok(())
            }
        }
    }
}

fn ensure_parent(path: &Path) -> Result<(), ExecError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io("create parent of", path))?;
    }
    Ok(())
}

fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let meta = fs::symlink_metadata(&from)?;
        if meta.is_dir() {
            copy_tree(&from, &to)?;
        } else if meta.is_symlink() {
            let target = fs::read_link(&from)?;
            let _ = fs::remove_file(&to);
            std::os::unix::fs::symlink(target, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Run every action in `plan`, updating `state` to record what was written.
///
/// The plan is validated in full before any action runs, so a rejected plan
/// leaves the filesystem exactly as it was.
pub fn execute(
    plan: &Plan,
    state: &mut State,
    source_dir: &Path,
    options: &ExecuteOptions,
) -> Result<ExecuteReport, ExecError> {
    validate(plan, source_dir, options)?;

    let mut report = ExecuteReport::default();

    for planned in &plan.actions {
        let risk = planned.risk;
        match &planned.action {
            Action::CreateDir { path } => {
                fs::create_dir_all(path).map_err(io("create", path))?;
                // Profile directories sit next to credentials, so they are
                // private by construction rather than by umask.
                fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                    .map_err(io("chmod", path))?;
            }

            Action::Symlink { link, target } => {
                clear(link, risk, &mut report)?;
                ensure_parent(link)?;
                std::os::unix::fs::symlink(target, link).map_err(io("symlink", link))?;
                state.record(
                    link,
                    sha256_bytes(target.as_os_str().as_encoded_bytes()),
                    planned.owner.as_deref().unwrap_or(""),
                );
            }

            Action::CopyFile { src, dst } => {
                clear(dst, risk, &mut report)?;
                ensure_parent(dst)?;
                fs::copy(src, dst).map_err(io("copy", dst))?;
                let hash = hash_path(dst).map_err(io("hash", dst))?;
                if let Some(hash) = hash {
                    state.record(dst, hash, planned.owner.as_deref().unwrap_or(""));
                }
            }

            Action::CopyTree { src, dst } => {
                clear(dst, risk, &mut report)?;
                ensure_parent(dst)?;
                copy_tree(src, dst).map_err(io("copy tree", dst))?;
                state.record(dst, String::from("dir"), planned.owner.as_deref().unwrap_or(""));
            }

            Action::WriteFile {
                path,
                content,
                executable,
            } => {
                clear(path, risk, &mut report)?;
                ensure_parent(path)?;
                fs::write(path, content).map_err(io("write", path))?;
                let mode = if *executable { 0o755 } else { 0o644 };
                fs::set_permissions(path, fs::Permissions::from_mode(mode))
                    .map_err(io("chmod", path))?;
                state.record(
                    path,
                    sha256_bytes(content.as_bytes()),
                    planned.owner.as_deref().unwrap_or(""),
                );
            }

            Action::Backup { path, to } => {
                fs::rename(path, to).map_err(io("backup", path))?;
                report.backups.push((path.clone(), to.clone()));
            }

            Action::RemoveGenerated { path } => {
                // Re-checked here rather than trusted from the plan: the plan
                // may have been computed some time ago, or by the UI.
                if state.classify(path).map_err(io("inspect", path))? != Ownership::Ours {
                    return Err(ExecError::NotOurs { path: path.clone() });
                }
                let meta = fs::symlink_metadata(path).map_err(io("inspect", path))?;
                if meta.is_dir() && !meta.is_symlink() {
                    fs::remove_dir_all(path).map_err(io("remove", path))?;
                } else {
                    fs::remove_file(path).map_err(io("remove", path))?;
                }
                state.forget(path);
            }

            Action::WriteEnvrcBlock { .. }
            | Action::RemoveEnvrcBlock { .. }
            | Action::GitInfoExclude { .. }
            | Action::RunDirenvAllow { .. } => {
                unimplemented!("directory binding actions land with the binding module")
            }
        }

        report.performed.push(planned.description.clone());
    }

    Ok(report)
}
