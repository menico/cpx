//! Where a profile's login lives, and which account it belongs to.
//!
//! Status only: no token value is ever read by these functions. On macOS the
//! Claude Keychain lookup deliberately omits `-w`, so nothing is returned and
//! no access prompt appears. Codex keeps its login in a plain `auth.json`,
//! whose identity token names the account; the file is parsed for that name
//! and nothing else.

use crate::config::Provider;
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
    /// Codex signed in with an API key rather than an account.
    ApiKey,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CredentialStatus {
    pub authenticated: bool,
    pub source: CredentialSource,
    /// The signed-in email, when the tool has recorded one.
    pub account: Option<String>,
    pub organization: Option<String>,
    /// The subscription, when the credentials say (`plus`, `pro`, `team`…).
    /// Codex writes it into its identity token; Claude does not record it.
    pub plan: Option<String>,
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

/// The Keychain service for a config directory, given the home it belongs to.
///
/// The default directory (`~/.claude`) stores its token under the bare
/// service name; only a custom `CLAUDE_CONFIG_DIR` gets a digest suffix.
/// Deriving a suffix for the default would report it as signed out.
pub fn keychain_service_for(config_dir: &Path, home: &Path) -> String {
    if config_dir == home.join(".claude") {
        return KEYCHAIN_SERVICE_BASE.to_string();
    }
    keychain_service(config_dir)
}

/// The Keychain service name for a custom `CLAUDE_CONFIG_DIR`: the base
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

/// The account recorded in `.claude.json`, which Claude Code writes
/// regardless of where the token itself is stored.
///
/// For a custom `CLAUDE_CONFIG_DIR` that file sits inside the directory. For
/// the default directory it sits beside it, at `~/.claude.json`, and the file
/// inside carries no account — so the default session would otherwise report
/// as signed in to nobody.
pub fn account_from_claude_json(profile_dir: &Path, home: &Path) -> (Option<String>, Option<String>) {
    let mut candidates = vec![profile_dir.join(".claude.json")];
    if profile_dir == home.join(".claude") {
        candidates.push(home.join(".claude.json"));
    }

    for path in candidates {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let account = value
            .pointer("/oauthAccount/emailAddress")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        if account.is_some() {
            let org = value
                .pointer("/oauthAccount/organizationUuid")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            return (account, org);
        }
    }
    (None, None)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Whether the credentials file records an expiry in the past.
fn file_expired(path: &Path) -> Option<bool> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let expires_ms = value.pointer("/claudeAiOauth/expiresAt")?.as_i64()?;
    Some(expires_ms <= now_ms())
}

/// Full credential status for one profile directory of the given provider.
pub fn status_for(provider: Provider, profile_dir: &Path, home: &Path) -> CredentialStatus {
    match provider {
        Provider::Claude => status(profile_dir, home),
        Provider::Codex => codex_status(profile_dir),
    }
}

/// Full credential status for one Claude profile directory.
///
/// `home` is needed to recognise the default config directory, whose token
/// lives under the unsuffixed Keychain service.
pub fn status(profile_dir: &Path, home: &Path) -> CredentialStatus {
    let (account, organization) = account_from_claude_json(profile_dir, home);

    if keychain_has_entry(
        &keychain_service_for(profile_dir, home),
        &keychain_account(None),
    ) {
        return CredentialStatus {
            authenticated: true,
            source: CredentialSource::Keychain,
            account,
            organization,
            plan: None,
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
            plan: None,
            expired: file_expired(&file),
        };
    }

    CredentialStatus {
        authenticated: false,
        source: CredentialSource::None,
        account,
        organization,
        plan: None,
        expired: None,
    }
}

/// The credentials file used on Linux and in CI.
pub fn credentials_file(profile_dir: &Path) -> PathBuf {
    profile_dir.join(".credentials.json")
}

/// Where Codex keeps its login.
pub fn codex_auth_file(home_dir: &Path) -> PathBuf {
    home_dir.join("auth.json")
}

/// The claims of a JWT, without verifying it. Enough to read who a token
/// was issued to; never used to decide whether to trust it.
pub fn jwt_claims(token: &str) -> Option<serde_json::Value> {
    use base64::Engine;
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// What Codex's `auth.json` says about the account.
///
/// Codex signs in either through ChatGPT — leaving OAuth tokens whose
/// identity token names the email and plan — or with an API key, which
/// names nobody.
pub fn codex_status(home_dir: &Path) -> CredentialStatus {
    let none = CredentialStatus {
        authenticated: false,
        source: CredentialSource::None,
        account: None,
        organization: None,
        plan: None,
        expired: None,
    };

    let Ok(text) = std::fs::read_to_string(codex_auth_file(home_dir)) else {
        return none;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return none;
    };

    let tokens = value.get("tokens").filter(|t| t.is_object());
    let access = tokens
        .and_then(|t| t.get("access_token"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());

    if let Some(access) = access {
        let id_claims = tokens
            .and_then(|t| t.get("id_token"))
            .and_then(|v| v.as_str())
            .and_then(jwt_claims);
        let auth = id_claims
            .as_ref()
            .and_then(|c| c.get("https://api.openai.com/auth"));
        let organization = auth
            .and_then(|a| a.get("organizations"))
            .and_then(|o| o.as_array())
            .and_then(|orgs| {
                orgs.iter()
                    .find(|o| o.get("is_default").and_then(|d| d.as_bool()) == Some(true))
                    .or(orgs.first())
            })
            .and_then(|o| o.get("title"))
            .and_then(|t| t.as_str())
            .map(str::to_string);
        let expired = jwt_claims(access)
            .and_then(|c| c.get("exp")?.as_i64())
            .map(|exp| exp * 1000 <= now_ms());

        return CredentialStatus {
            authenticated: true,
            source: CredentialSource::File,
            account: id_claims
                .as_ref()
                .and_then(|c| c.get("email"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
            organization,
            plan: auth
                .and_then(|a| a.get("chatgpt_plan_type"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
            expired,
        };
    }

    let api_key = value
        .get("OPENAI_API_KEY")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    if api_key.is_some() {
        return CredentialStatus {
            authenticated: true,
            source: CredentialSource::ApiKey,
            ..none
        };
    }

    none
}

/// Where a profile's login is kept, for showing rather than reading.
pub fn location(provider: Provider, profile_dir: &Path, home: &Path) -> String {
    match provider {
        Provider::Claude => format!("Keychain: {}", keychain_service_for(profile_dir, home)),
        Provider::Codex => codex_auth_file(profile_dir).display().to_string(),
    }
}
