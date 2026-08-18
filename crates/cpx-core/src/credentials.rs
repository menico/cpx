//! Where a profile's login lives, and which account it belongs to.
//!
//! Status only: no token value is ever read. On macOS the Keychain lookup
//! deliberately omits `-w`, so nothing is returned and no access prompt
//! appears.

use serde::Serialize;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

/// The Keychain service Claude Code writes its OAuth token under. A custom
/// `CLAUDE_CONFIG_DIR` gets a suffix derived from that path, which is exactly
/// what keeps profiles isolated from each other and from the default login.
pub const KEYCHAIN_SERVICE_BASE: &str = "Claude Code-credentials";

/// Claude Code's own validation of `$USER` before using it as an account name.
fn account_is_valid(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum CredentialSource {
    Keychain,
    File,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CredentialStatus {
    pub authenticated: bool,
    pub source: CredentialSource,
    /// The signed-in email, when Claude Code has recorded one.
    pub account: Option<String>,
    pub organization: Option<String>,
    /// Only knowable from a credentials file; the Keychain is not read.
    pub expired: Option<bool>,
}

/// Swapped out in tests so the suite never touches the real Keychain.
pub type Lookup = fn(&str, &str) -> bool;

thread_local! {
    /// Thread-local rather than global: the test harness runs each test on
    /// its own thread, so one test's stub cannot leak into another's.
    static LOOKUP: RefCell<Option<Lookup>> = const { RefCell::new(None) };
}

/// Override the Keychain probe for the current thread. Intended for tests.
pub fn set_keychain_lookup(lookup: Lookup) {
    LOOKUP.with(|l| *l.borrow_mut() = Some(lookup));
}

fn lookup() -> Lookup {
    LOOKUP.with(|l| *l.borrow()).unwrap_or(real_keychain_lookup)
}

/// The Keychain service name for a given `CLAUDE_CONFIG_DIR`: the base
/// service plus the first eight hex characters of the SHA-256 of the path.
pub fn keychain_service(config_dir: &Path) -> String {
    let digest = crate::state::sha256_bytes(config_dir.as_os_str().as_encoded_bytes());
    format!("{KEYCHAIN_SERVICE_BASE}-{}", &digest[..8])
}

/// The Keychain account name Claude Code uses.
pub fn keychain_account(user: Option<&str>) -> String {
    let name = match user {
        Some(name) => name.to_string(),
        None => std::env::var("USER").unwrap_or_default(),
    };
    if account_is_valid(&name) {
        name
    } else {
        "claude-code-user".to_string()
    }
}

/// The real probe: existence only. `-w` is deliberately omitted, so the
/// token is never returned and macOS shows no access prompt.
fn real_keychain_lookup(service: &str, account: &str) -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    std::process::Command::new("security")
        .args(["find-generic-password", "-a", account, "-s", service])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Whether a Keychain entry exists, without reading it.
pub fn keychain_has_entry(service: &str, account: &str) -> bool {
    lookup()(service, account)
}

/// The account recorded in a profile's `.claude.json`, which Claude Code
/// writes regardless of where the token itself is stored.
pub fn account_from_claude_json(profile_dir: &Path) -> (Option<String>, Option<String>) {
    let Ok(text) = std::fs::read_to_string(profile_dir.join(".claude.json")) else {
        return (None, None);
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return (None, None);
    };
    let account = value.pointer("/oauthAccount/emailAddress");
    let org = value.pointer("/oauthAccount/organizationUuid");
    (
        account.and_then(|v| v.as_str()).map(str::to_string),
        org.and_then(|v| v.as_str()).map(str::to_string),
    )
}

/// Whether the credentials file records an expiry in the past.
fn file_expired(path: &Path) -> Option<bool> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let expires_ms = value.pointer("/claudeAiOauth/expiresAt")?.as_i64()?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as i64;
    Some(expires_ms <= now_ms)
}

/// Full credential status for one profile directory.
pub fn status(profile_dir: &Path) -> CredentialStatus {
    let (account, organization) = account_from_claude_json(profile_dir);

    if keychain_has_entry(
        &keychain_service(profile_dir),
        &keychain_account(None),
    ) {
        return CredentialStatus {
            authenticated: true,
            source: CredentialSource::Keychain,
            account,
            organization,
            // The Keychain entry is never read, so its expiry is unknowable
            // without prompting the user — which is not worth it for a status.
            expired: None,
        };
    }

    let file = credentials_file(profile_dir);
    if file.exists() {
        return CredentialStatus {
            authenticated: true,
            source: CredentialSource::File,
            account,
            organization,
            expired: file_expired(&file),
        };
    }

    CredentialStatus {
        authenticated: false,
        source: CredentialSource::None,
        account,
        organization,
        expired: None,
    }
}

/// The credentials file used on Linux and in CI.
pub fn credentials_file(profile_dir: &Path) -> PathBuf {
    profile_dir.join(".credentials.json")
}
