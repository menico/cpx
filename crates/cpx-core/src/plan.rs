//! Plans.
//!
//! Nothing in cpx-core mutates the filesystem directly. Every operation
//! computes a `Plan` — an ordered list of typed actions — which a separate
//! `execute` runs. `--dry-run` renders the plan, the Phase 2 UI renders the
//! same plan as a confirmation sheet, and tests assert on plan values instead
//! of inspecting directories.

use std::path::{Path, PathBuf};

/// What an action would displace. Ordered: `Safe` < `OverwritesGenerated` <
/// `OverwritesForeign`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Risk {
    /// The target is absent, or already exactly correct.
    Safe,
    /// The target is ours and untouched; replacing it loses nothing.
    OverwritesGenerated,
    /// The target is something cpx did not write, or wrote and a human has
    /// since edited. Requires `--force`, and is backed up first.
    OverwritesForeign,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    CreateDir {
        path: PathBuf,
    },
    Symlink {
        link: PathBuf,
        target: PathBuf,
    },
    CopyFile {
        src: PathBuf,
        dst: PathBuf,
    },
    CopyTree {
        src: PathBuf,
        dst: PathBuf,
    },
    WriteFile {
        path: PathBuf,
        content: String,
        executable: bool,
    },
    /// Rename rather than delete. cpx never destroys anything.
    Backup {
        path: PathBuf,
        to: PathBuf,
    },
    /// Remove an artifact cpx wrote and no longer wants. Refuses at execution
    /// time unless the path is genuinely ours.
    RemoveGenerated {
        path: PathBuf,
    },
    WriteEnvrcBlock {
        envrc: PathBuf,
        content: String,
    },
    RemoveEnvrcBlock {
        envrc: PathBuf,
    },
    GitInfoExclude {
        repo: PathBuf,
        line: String,
    },
    RunDirenvAllow {
        dir: PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedAction {
    pub action: Action,
    pub risk: Risk,
    pub description: String,
    /// The profile this artifact belongs to, recorded in the state manifest
    /// so a later apply recognises it as ours.
    pub owner: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub actions: Vec<PlannedAction>,
    /// Things the user should know that are not actions: a missing source
    /// directory, a skipped resource, a foreign wrapper left alone.
    pub notes: Vec<String>,
}

impl Action {
    /// The path this action writes to, used for the source-directory guard.
    pub fn target(&self) -> &Path {
        match self {
            Action::CreateDir { path }
            | Action::WriteFile { path, .. }
            | Action::RemoveGenerated { path }
            | Action::Backup { path, .. } => path,
            Action::Symlink { link, .. } => link,
            Action::CopyFile { dst, .. } | Action::CopyTree { dst, .. } => dst,
            Action::WriteEnvrcBlock { envrc, .. } | Action::RemoveEnvrcBlock { envrc } => envrc,
            Action::GitInfoExclude { repo, .. } => repo,
            Action::RunDirenvAllow { dir } => dir,
        }
    }
}

impl Plan {
    pub fn push(&mut self, action: Action, risk: Risk, description: impl Into<String>, owner: Option<&str>) {
        self.actions.push(PlannedAction {
            action,
            risk,
            description: description.into(),
            owner: owner.map(str::to_string),
        });
    }

    pub fn note(&mut self, note: impl Into<String>) {
        self.notes.push(note.into());
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    pub fn max_risk(&self) -> Risk {
        self.actions
            .iter()
            .map(|a| a.risk)
            .max()
            .unwrap_or(Risk::Safe)
    }

    /// A plan touching anything foreign must not run without `--force`.
    pub fn requires_force(&self) -> bool {
        self.max_risk() == Risk::OverwritesForeign
    }

    pub fn extend(&mut self, other: Plan) {
        self.actions.extend(other.actions);
        self.notes.extend(other.notes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_with(risks: &[Risk]) -> Plan {
        let mut plan = Plan::default();
        for (i, risk) in risks.iter().enumerate() {
            plan.push(
                Action::CreateDir {
                    path: PathBuf::from(format!("/tmp/{i}")),
                },
                *risk,
                "create",
                None,
            );
        }
        plan
    }

    #[test]
    fn an_empty_plan_is_safe() {
        assert_eq!(Plan::default().max_risk(), Risk::Safe);
        assert!(Plan::default().is_empty());
        assert!(!Plan::default().requires_force());
    }

    #[test]
    fn plan_risk_is_the_worst_of_its_actions() {
        let plan = plan_with(&[Risk::Safe, Risk::OverwritesForeign, Risk::Safe]);
        assert_eq!(plan.max_risk(), Risk::OverwritesForeign);
        assert!(plan.requires_force());
    }

    #[test]
    fn overwriting_only_our_own_files_does_not_require_force() {
        let plan = plan_with(&[Risk::Safe, Risk::OverwritesGenerated]);
        assert_eq!(plan.max_risk(), Risk::OverwritesGenerated);
        assert!(!plan.requires_force());
    }

    #[test]
    fn every_action_reports_the_path_it_writes_to() {
        let cases = [
            (
                Action::Symlink {
                    link: PathBuf::from("/a/link"),
                    target: PathBuf::from("/b/target"),
                },
                "/a/link",
            ),
            (
                Action::CopyFile {
                    src: PathBuf::from("/b/src"),
                    dst: PathBuf::from("/a/dst"),
                },
                "/a/dst",
            ),
            (
                Action::CopyTree {
                    src: PathBuf::from("/b/src"),
                    dst: PathBuf::from("/a/dst"),
                },
                "/a/dst",
            ),
            (
                Action::WriteFile {
                    path: PathBuf::from("/a/f"),
                    content: String::new(),
                    executable: false,
                },
                "/a/f",
            ),
        ];
        for (action, expected) in cases {
            assert_eq!(action.target(), Path::new(expected), "{action:?}");
        }
    }
}
