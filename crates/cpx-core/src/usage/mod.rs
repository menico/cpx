//! Live usage: how much of each account's rate limits are used, and when
//! they reset.
//!
//! This is the one place cpx reads a token. Everything else in the tool
//! works from the *existence* of a login; a usage figure needs the login
//! itself, because it is the account's own API that reports it. The token is
//! read for the duration of one request and never written anywhere.
//!
//! The data comes from the same endpoints the tools use themselves: Claude
//! Code's OAuth usage endpoint, and the ChatGPT backend Codex talks to. Both
//! are undocumented and can change; every parser here is defensive, and a
//! failure is reported as such rather than guessed around.

pub mod claude;
pub mod codex;
pub mod http;

use crate::config::{Config, Provider};
use crate::layout::Layout;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// One rate-limit window: a fraction used and a time it resets.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RateWindow {
    /// A stable identifier: `session`, `weekly`, `weekly-opus`, ...
    pub id: String,
    /// What to call it: `Session`, `Weekly`, `Opus weekly`, ...
    pub title: String,
    /// Percent used, 0–100. May exceed 100 when the provider says so.
    pub used_percent: f64,
    /// How long the window is, when known.
    pub window_minutes: Option<u32>,
    /// When the window resets, as RFC 3339, when known.
    pub resets_at: Option<String>,
    /// The same instant as seconds since the epoch, for arithmetic.
    pub resets_at_epoch: Option<i64>,
}

impl RateWindow {
    pub fn remaining_percent(&self) -> f64 {
        (100.0 - self.used_percent).max(0.0)
    }
}

/// Claude's pay-as-you-go allowance beyond the subscription.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtraUsage {
    pub enabled: bool,
    pub used_usd: Option<f64>,
    pub limit_usd: Option<f64>,
    pub utilization: Option<f64>,
}

/// Codex credits, when the plan has them.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Credits {
    pub has_credits: bool,
    pub unlimited: bool,
    pub balance: Option<f64>,
}

/// What one account's usage looks like right now.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    pub provider: Provider,
    pub account: Option<String>,
    /// The plan as a label: `Max 5x`, `Pro`, `Plus`, ...
    pub plan: Option<String>,
    /// Session first, then weekly, then anything model-specific.
    pub windows: Vec<RateWindow>,
    pub extra_usage: Option<ExtraUsage>,
    pub credits: Option<Credits>,
    /// Where the numbers came from: `oauth` or `cli`.
    pub source: String,
    /// Seconds since the epoch.
    pub fetched_at: i64,
}

impl UsageSnapshot {
    pub fn window(&self, id: &str) -> Option<&RateWindow> {
        self.windows.iter().find(|w| w.id == id)
    }
}

/// Why usage could not be fetched. Each carries what the user can do about
/// it, because a bare failure in a menu bar is a dead end.
#[derive(Debug, Clone, PartialEq, thiserror::Error, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UsageError {
    #[error("not signed in")]
    NotSignedIn,

    #[error("the login has expired; run `{command}` once and it will refresh itself")]
    TokenExpired { command: String },

    #[error("Keychain access was denied; allow cpx to read the login, or run `{command}` to check it")]
    KeychainDenied { command: String },

    #[error("cpx has not been allowed to read this login yet; refresh by hand to be asked, and choose Always Allow")]
    KeychainNeedsPermission { command: String },

    #[error("the login is not usable ({detail}); run `{command}` to sign in again")]
    Unauthorized { command: String, detail: String },

    #[error("rate limited by the provider; try again in a few minutes")]
    RateLimited { retry_after_seconds: Option<u64> },

    #[error("the provider answered HTTP {status}: {body}")]
    Http { status: u16, body: String },

    #[error("network error: {0}")]
    Network(String),

    #[error("could not read the response: {0}")]
    Parse(String),

    #[error("{0}")]
    Cli(String),
}

/// Whose usage to fetch: a profile, or a tool's default session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// `None` for the unmanaged default session.
    pub profile: Option<String>,
    pub provider: Provider,
    pub dir: PathBuf,
    /// What to run to sign in or refresh: `claude-work`, `codex`, ...
    pub command: String,
}

impl Target {
    pub fn for_profile(config: &Config, layout: &Layout, name: &str) -> Option<Target> {
        let profile = config.profiles.get(name)?;
        Some(Target {
            profile: Some(name.to_string()),
            provider: profile.provider,
            dir: config.config_dir(layout, name),
            command: config.command_name(name),
        })
    }

    pub fn default_session(layout: &Layout, provider: Provider) -> Target {
        Target {
            profile: None,
            provider,
            dir: layout
                .home
                .join(provider.default_source_dir().trim_start_matches("~/")),
            command: provider.binary_name().to_string(),
        }
    }

    /// Every account worth asking about: each profile, then each tool's
    /// default session unless a profile has adopted it.
    pub fn all(config: &Config, layout: &Layout) -> Vec<Target> {
        let mut targets: Vec<Target> = config
            .profiles
            .keys()
            .filter_map(|name| Target::for_profile(config, layout, name))
            .collect();
        for provider in Provider::ALL {
            let default = Target::default_session(layout, provider);
            let claimed = config.profiles.values().any(|p| p.dir.as_deref() == Some(default.dir.as_path()));
            if !claimed && default.dir.is_dir() {
                targets.push(default);
            }
        }
        targets
    }
}

/// How Claude's stored credentials are read. The Keychain on macOS, a file
/// elsewhere; swapped out in tests.
pub trait ClaudeCredentialReader {
    /// The raw JSON Claude Code stored, if any. With `prompt` false the
    /// read must never put a dialog on the screen.
    fn read(&self, dir: &Path, home: &Path, prompt: bool) -> Result<Option<String>, UsageError>;
}

/// The reader for a real machine: the Keychain, then the credentials file.
pub struct SystemCredentialReader;

impl ClaudeCredentialReader for SystemCredentialReader {
    fn read(&self, dir: &Path, home: &Path, prompt: bool) -> Result<Option<String>, UsageError> {
        if let Some(json) = claude::keychain_read(dir, home, prompt)? {
            return Ok(Some(json));
        }
        let file = crate::credentials::credentials_file(dir);
        match std::fs::read_to_string(&file) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(UsageError::Parse(format!("{}: {e}", file.display()))),
        }
    }
}

/// Everything a fetch needs from the outside world, so tests can supply
/// fakes for all of it.
pub struct Fetcher<'a> {
    pub http: &'a dyn http::Http,
    pub claude_credentials: &'a dyn ClaudeCredentialReader,
    /// The real Codex binary, for the app-server fallback.
    pub codex_binary: Option<PathBuf>,
    pub home: PathBuf,
    /// Whether a Claude Keychain read may ask the user. False for anything
    /// that runs in the background.
    pub keychain_prompt: bool,
}

impl Fetcher<'_> {
    pub fn fetch(&self, target: &Target) -> Result<UsageSnapshot, UsageError> {
        match target.provider {
            Provider::Claude => claude::fetch(self, target),
            Provider::Codex => codex::fetch(self, target),
        }
    }
}

pub(crate) fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Parse an RFC 3339 / ISO 8601 timestamp into seconds since the epoch.
pub(crate) fn parse_timestamp(text: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(text.trim())
        .ok()
        .map(|d| d.timestamp())
}

pub(crate) fn format_timestamp(epoch: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(epoch, 0)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_round_trip() {
        let epoch = parse_timestamp("2026-09-16T10:00:00Z").unwrap();
        assert_eq!(format_timestamp(epoch), "2026-09-16T10:00:00Z");
        assert_eq!(parse_timestamp("2026-09-16T12:00:00.000000+02:00"), Some(epoch));
        assert_eq!(parse_timestamp("nope"), None);
    }

    #[test]
    fn remaining_percent_never_goes_negative() {
        let w = RateWindow {
            id: "session".into(),
            title: "Session".into(),
            used_percent: 130.0,
            window_minutes: None,
            resets_at: None,
            resets_at_epoch: None,
        };
        assert_eq!(w.remaining_percent(), 0.0);
    }

    #[test]
    fn targets_cover_profiles_and_unclaimed_default_sessions() {
        let d = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(d.path().join(".claude")).unwrap();
        std::fs::create_dir_all(d.path().join(".codex")).unwrap();
        let layout = Layout::new(d.path());
        let config = Config::parse(
            "version = 1\n[profiles.work]\n[profiles.gpt]\nprovider = \"codex\"\n",
            d.path(),
        )
        .unwrap();
        let targets = Target::all(&config, &layout);
        let names: Vec<(Option<&str>, Provider)> = targets
            .iter()
            .map(|t| (t.profile.as_deref(), t.provider))
            .collect();
        assert_eq!(
            names,
            vec![
                (Some("gpt"), Provider::Codex),
                (Some("work"), Provider::Claude),
                (None, Provider::Claude),
                (None, Provider::Codex),
            ]
        );
        assert_eq!(targets[0].command, "codex-gpt");
        assert_eq!(targets[2].command, "claude");
    }

    #[test]
    fn a_default_session_adopted_by_a_profile_is_not_listed_twice() {
        let d = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(d.path().join(".claude")).unwrap();
        let layout = Layout::new(d.path());
        let config = Config::parse("version = 1\n[profiles.base]\ndir = \"~/.claude\"\n", d.path()).unwrap();
        let targets = Target::all(&config, &layout);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].profile.as_deref(), Some("base"));
    }
}
