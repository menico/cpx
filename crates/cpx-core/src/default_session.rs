//! The sessions a plain `claude` and a plain `codex` use.
//!
//! With no `CLAUDE_CONFIG_DIR` set, Claude Code reads `~/.claude`; with no
//! `CODEX_HOME`, Codex reads `~/.codex`. cpx models those directories as the
//! sources profiles inherit from rather than as profiles, so they are never
//! managed — but each is usually a working account, and leaving them out of
//! the interface means the tool shows fewer accounts than the machine has.
//! They are reported, never written to.

use crate::config::{Config, Provider};
use crate::credentials::{self, CredentialSource};
use crate::layout::Layout;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultSession {
    pub provider: Provider,
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

/// Report on one provider's default directory without touching it.
pub fn default_session_for(layout: &Layout, config: &Config, provider: Provider) -> DefaultSession {
    let dir = layout
        .home
        .join(provider.default_source_dir().trim_start_matches("~/"));
    let status = credentials::status_for(provider, &dir, &layout.home);

    DefaultSession {
        provider,
        is_source: config.source_dir(provider) == dir,
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

/// Report on `~/.claude` without touching it.
pub fn default_session(layout: &Layout, config: &Config) -> DefaultSession {
    default_session_for(layout, config, Provider::Claude)
}

/// Every default directory worth showing: `~/.claude` always, `~/.codex`
/// when it exists or a Codex profile is configured.
pub fn default_sessions(layout: &Layout, config: &Config) -> Vec<DefaultSession> {
    let mut found = vec![default_session_for(layout, config, Provider::Claude)];
    let codex = default_session_for(layout, config, Provider::Codex);
    if codex.dir.is_dir() || config.profiles.values().any(|p| p.provider == Provider::Codex) {
        found.push(codex);
    }
    found
}
