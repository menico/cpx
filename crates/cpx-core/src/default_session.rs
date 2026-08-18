//! The session a plain `claude` uses.
//!
//! With no `CLAUDE_CONFIG_DIR` set, Claude Code reads `~/.claude`. cpx models
//! that directory as the source profiles inherit from rather than as a
//! profile, so it is never managed — but it is usually a working account, and
//! leaving it out of the interface means the tool shows fewer accounts than
//! the machine has. It is reported, never written to.

use crate::config::Config;
use crate::credentials::{self, CredentialSource};
use crate::layout::Layout;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultSession {
    pub dir: PathBuf,
    pub account: Option<String>,
    pub signed_in: bool,
    pub credential_source: CredentialSource,
    /// True when this is also the directory profiles inherit from.
    pub is_source: bool,
    /// The profile that has adopted this directory, if any. When set, the
    /// directory is already managed and needs no separate mention.
    pub claimed_by: Option<String>,
}

/// Report on `~/.claude` without touching it.
pub fn default_session(layout: &Layout, config: &Config) -> DefaultSession {
    let dir = layout.home.join(".claude");
    let status = credentials::status(&dir, &layout.home);

    DefaultSession {
        is_source: config.source_dir == dir,
        claimed_by: config
            .profiles
            .iter()
            .find(|(_, profile)| profile.dir.as_deref() == Some(dir.as_path()))
            .map(|(name, _)| name.clone()),
        account: status.account,
        signed_in: status.authenticated,
        credential_source: status.source,
        dir,
    }
}
