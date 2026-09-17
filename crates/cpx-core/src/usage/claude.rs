//! Claude Code usage, from Anthropic's OAuth usage endpoint.
//!
//! Claude Code keeps its OAuth token in the Keychain under a service name
//! derived from the config directory (see `credentials`), which is exactly
//! what lets each profile be asked about separately. The token is read for
//! one request and never written back to the Keychain.
//!
//! macOS asks before a program may read another program's Keychain item,
//! and asks again after Claude Code rewrites the item, which it does on
//! every renewal. So, when asked to, cpx keeps an owner-only copy of the
//! login under its own root and renews that copy itself, the way Claude
//! Code renews its own — after which the Keychain is not needed until the
//! copy is rejected outright. Without a copy, an expired login is reported
//! with the fix being to run the profile's own command once.

use super::http::retry_after_seconds;
use super::{format_timestamp, now_epoch, parse_timestamp, ExtraUsage, Fetcher, RateWindow, Target, UsageError, UsageSnapshot};
use crate::config::Provider;
use crate::credentials::{keychain_account, keychain_service_for};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const BETA_HEADER: &str = "oauth-2025-04-20";

/// The version Claude Code reports about itself, for the user agent the
/// endpoint expects.
fn claude_code_version() -> String {
    "2.1.0".to_string()
}

/// Read the stored credentials JSON from the Keychain.
///
/// macOS asks the user the first time cpx touches a login; "Always Allow"
/// makes every later read silent — until Claude Code rewrites the item,
/// which it does when it refreshes the token, after which the question
/// comes back. With `prompt` false the read never asks: it succeeds when
/// access was already granted and otherwise reports that permission is
/// needed, so a background refresh cannot put a dialog on the screen.
#[cfg(target_os = "macos")]
pub fn keychain_read(dir: &Path, home: &Path, prompt: bool) -> Result<Option<String>, UsageError> {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::boolean::CFBoolean;
    use core_foundation::data::CFData;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::string::CFString;
    use security_framework_sys::base::errSecSuccess;
    use security_framework_sys::item::*;
    use security_framework_sys::keychain_item::SecItemCopyMatching;

    #[link(name = "Security", kind = "framework")]
    extern "C" {
        static kSecUseAuthenticationUIFail: core_foundation::string::CFStringRef;
        static kSecMatchLimitOne: core_foundation::string::CFStringRef;
    }

    // A way to run without ever touching the Keychain: for tests, and for
    // anyone who would rather cpx did not ask.
    if std::env::var_os("CPX_NO_KEYCHAIN").is_some() {
        return Err(UsageError::KeychainNeedsPermission {
            command: String::new(),
        });
    }

    let service = keychain_service_for(dir, home);
    let account = keychain_account(None);

    let (status, data) = unsafe {
        let mut pairs: Vec<(CFString, CFType)> = vec![
            (CFString::wrap_under_get_rule(kSecClass), CFString::wrap_under_get_rule(kSecClassGenericPassword).as_CFType()),
            (CFString::wrap_under_get_rule(kSecAttrService), CFString::new(&service).as_CFType()),
            (CFString::wrap_under_get_rule(kSecAttrAccount), CFString::new(&account).as_CFType()),
            (CFString::wrap_under_get_rule(kSecMatchLimit), CFString::wrap_under_get_rule(kSecMatchLimitOne).as_CFType()),
            (CFString::wrap_under_get_rule(kSecReturnData), CFBoolean::true_value().as_CFType()),
        ];
        if !prompt {
            pairs.push((
                CFString::wrap_under_get_rule(kSecUseAuthenticationUI),
                CFString::wrap_under_get_rule(kSecUseAuthenticationUIFail).as_CFType(),
            ));
        }
        let query = CFDictionary::from_CFType_pairs(&pairs);
        let mut result: core_foundation::base::CFTypeRef = std::ptr::null();
        let status = SecItemCopyMatching(query.as_concrete_TypeRef(), &mut result);
        let data = if status == errSecSuccess && !result.is_null() {
            let data: CFData = CFData::wrap_under_create_rule(result as _);
            Some(String::from_utf8_lossy(data.bytes()).to_string())
        } else {
            None
        };
        (status, data)
    };

    match status {
        0 => Ok(data),
        // errSecItemNotFound: no login.
        -25300 => Ok(None),
        // errSecInteractionNotAllowed: access would need a prompt, and none
        // was allowed this time.
        -25308 => Err(UsageError::KeychainNeedsPermission {
            command: String::new(),
        }),
        // errSecUserCanceled, errSecAuthFailed, errSecNoAccessForItem: no.
        -128 | -25293 | -25243 => Err(UsageError::KeychainDenied {
            command: String::new(),
        }),
        other => Err(UsageError::Parse(format!("Keychain error {other}"))),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn keychain_read(_dir: &Path, _home: &Path, _prompt: bool) -> Result<Option<String>, UsageError> {
    Ok(None)
}

/// What the stored JSON holds, minus the secrets that are not needed.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredCredentials {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Milliseconds since the epoch, as Claude Code writes it.
    pub expires_at_ms: Option<i64>,
    pub scopes: Vec<String>,
    pub subscription_type: Option<String>,
    pub rate_limit_tier: Option<String>,
}

impl StoredCredentials {
    pub fn is_expired(&self, now_ms: i64) -> bool {
        self.expires_at_ms.is_some_and(|at| at <= now_ms)
    }

    /// The same shape Claude Code writes, so a copy reads back the same way.
    pub fn to_json(&self) -> String {
        let mut oauth = serde_json::Map::new();
        oauth.insert("accessToken".into(), Value::String(self.access_token.clone()));
        if let Some(refresh) = &self.refresh_token {
            oauth.insert("refreshToken".into(), Value::String(refresh.clone()));
        }
        if let Some(at) = self.expires_at_ms {
            oauth.insert("expiresAt".into(), Value::from(at));
        }
        oauth.insert("scopes".into(), Value::Array(self.scopes.iter().map(|s| Value::String(s.clone())).collect()));
        if let Some(kind) = &self.subscription_type {
            oauth.insert("subscriptionType".into(), Value::String(kind.clone()));
        }
        if let Some(tier) = &self.rate_limit_tier {
            oauth.insert("rateLimitTier".into(), Value::String(tier.clone()));
        }
        serde_json::json!({ "claudeAiOauth": Value::Object(oauth) }).to_string()
    }
}

/// Where Claude Code's own OAuth client renews a token.
pub const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
/// Claude Code's OAuth client id, which the refresh endpoint expects.
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
/// A copy is refreshed this long before it would expire.
const REFRESH_MARGIN_MS: i64 = 5 * 60 * 1000;

/// Why a refresh did not produce a token.
enum RefreshOutcome {
    /// The provider rejected the refresh token: the copy is dead.
    Rejected,
    /// Something transient; the copy may still be fine later.
    Failed(UsageError),
}

/// Renew a copied login with its refresh token.
///
/// This is what Claude Code itself does when its token expires, with the
/// same client id. cpx's copy is renewed independently and never written
/// back: Claude Code keeps renewing its own.
fn refresh(http: &dyn super::http::Http, credentials: &StoredCredentials) -> Result<StoredCredentials, RefreshOutcome> {
    let Some(refresh_token) = credentials.refresh_token.as_deref() else {
        return Err(RefreshOutcome::Rejected);
    };
    let response = http
        .post_form(
            TOKEN_URL,
            &[("Accept", "application/json")],
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", CLIENT_ID),
            ],
        )
        .map_err(RefreshOutcome::Failed)?;
    if response.status == 400 || response.status == 401 {
        return Err(RefreshOutcome::Rejected);
    }
    if response.status != 200 {
        return Err(RefreshOutcome::Failed(UsageError::Http {
            status: response.status,
            body: response.body.chars().take(400).collect(),
        }));
    }
    let value: Value = serde_json::from_str(&response.body)
        .map_err(|e| RefreshOutcome::Failed(UsageError::Parse(format!("token response: {e}"))))?;
    let access_token = value
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| RefreshOutcome::Failed(UsageError::Parse("token response had no access_token".into())))?;
    let expires_in = value.get("expires_in").and_then(|v| v.as_i64()).unwrap_or(3600);
    Ok(StoredCredentials {
        access_token: access_token.to_string(),
        refresh_token: value
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| credentials.refresh_token.clone()),
        expires_at_ms: Some(now_epoch() * 1000 + expires_in * 1000),
        scopes: credentials.scopes.clone(),
        subscription_type: credentials.subscription_type.clone(),
        rate_limit_tier: credentials.rate_limit_tier.clone(),
    })
}

/// The copy of a login for one config directory.
pub fn cache_file(cache_dir: &Path, config_dir: &Path) -> PathBuf {
    let digest = crate::state::sha256_bytes(config_dir.as_os_str().as_encoded_bytes());
    cache_dir.join(format!("{}.json", &digest[..16]))
}

/// Write a copy that only this user can read.
fn write_cache(path: &Path, credentials: &StoredCredentials) -> Result<(), UsageError> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let io = |e: std::io::Error| UsageError::Parse(format!("{}: {e}", path.display()));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).map_err(io)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(io)?;
    file.write_all(credentials.to_json().as_bytes()).map_err(io)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(io)
}

/// Where a login came from, which decides what a rejection means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Copy,
    Original,
}

/// A usable login for `target`: the copy if there is a live one, renewed
/// if it is about to expire, otherwise Claude Code's own — which is copied
/// when copies are wanted.
fn load_credentials(fetcher: &Fetcher, target: &Target) -> Result<(StoredCredentials, Source), UsageError> {
    let with_command = |e: UsageError| match e {
        UsageError::KeychainDenied { .. } => UsageError::KeychainDenied {
            command: format!("{} auth status", target.command),
        },
        UsageError::KeychainNeedsPermission { .. } => UsageError::KeychainNeedsPermission {
            command: target.command.clone(),
        },
        other => other,
    };
    let copy = fetcher.credential_cache.as_ref().map(|dir| cache_file(dir, &target.dir));

    if let Some(path) = &copy {
        if let Some(cached) = std::fs::read_to_string(path).ok().and_then(|text| parse_stored(&text).ok().flatten()) {
            if !cached.is_expired(now_epoch() * 1000 + REFRESH_MARGIN_MS) {
                return Ok((cached, Source::Copy));
            }
            match refresh(fetcher.http, &cached) {
                Ok(renewed) => {
                    write_cache(path, &renewed)?;
                    return Ok((renewed, Source::Copy));
                }
                // A dead copy is dropped; Claude Code's own login is next.
                Err(RefreshOutcome::Rejected) => {
                    let _ = std::fs::remove_file(path);
                }
                Err(RefreshOutcome::Failed(e)) => return Err(e),
            }
        }
    }

    let stored = fetcher
        .claude_credentials
        .read(&target.dir, &fetcher.home, fetcher.keychain_prompt)
        .map_err(with_command)?;
    let Some(stored) = stored else {
        return Err(UsageError::NotSignedIn);
    };
    let Some(credentials) = parse_stored(&stored)? else {
        return Err(UsageError::NotSignedIn);
    };
    if credentials.is_expired(now_epoch() * 1000) {
        return Err(UsageError::TokenExpired {
            command: target.command.clone(),
        });
    }
    if let Some(path) = &copy {
        write_cache(path, &credentials)?;
    }
    Ok((credentials, Source::Original))
}

/// Parse what Claude Code stores. The Keychain value and the credentials
/// file have the same shape.
pub fn parse_stored(json: &str) -> Result<Option<StoredCredentials>, UsageError> {
    let value: Value =
        serde_json::from_str(json).map_err(|e| UsageError::Parse(format!("stored credentials: {e}")))?;
    let Some(oauth) = value.get("claudeAiOauth") else {
        // A Keychain item holding only MCP server logins is a known state on
        // some Claude Code versions: signed in to servers, not to Claude.
        return Ok(None);
    };
    let Some(access_token) = oauth.get("accessToken").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    Ok(Some(StoredCredentials {
        access_token: access_token.to_string(),
        refresh_token: oauth
            .get("refreshToken")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        expires_at_ms: oauth.get("expiresAt").and_then(|v| v.as_f64()).map(|f| f as i64),
        scopes: oauth
            .get("scopes")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|s| s.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        subscription_type: oauth
            .get("subscriptionType")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        rate_limit_tier: oauth
            .get("rateLimitTier")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    }))
}

/// A plan label from what the credentials (or `.claude.json`) say.
///
/// `subscription_type` is what Claude Code itself trusts; the rate-limit
/// tier is the fallback, and is where a Max multiplier lives
/// (`default_claude_max_5x`).
pub fn plan_label(subscription_type: Option<&str>, rate_limit_tier: Option<&str>) -> Option<String> {
    let tiers = ["max", "pro", "team", "enterprise", "ultra"];
    let from_subscription = subscription_type.and_then(|raw| {
        let words: Vec<String> = raw
            .split(|c: char| !c.is_ascii_alphanumeric())
            .map(|w| w.to_ascii_lowercase())
            .collect();
        tiers
            .iter()
            .find(|tier| words.iter().any(|w| w == *tier))
            .map(|tier| tier.to_string())
    });
    let tier_words: Vec<String> = rate_limit_tier
        .unwrap_or("")
        .split(|c: char| !c.is_ascii_alphanumeric())
        .map(|w| w.to_ascii_lowercase())
        .collect();
    let from_tier = rate_limit_tier.and_then(|raw| {
        let lower = raw.to_ascii_lowercase();
        tiers[..4]
            .iter()
            .find(|tier| lower.contains(*tier))
            .map(|tier| tier.to_string())
    });

    let tier = from_subscription.or(from_tier)?;
    let mut label = capitalize(&tier);
    if tier == "max" {
        // The word after `max` names the multiplier: `max_5x`.
        if let Some(index) = tier_words.iter().position(|w| w == "max") {
            if let Some(next) = tier_words.get(index + 1) {
                if next.ends_with('x') && next[..next.len() - 1].parse::<u32>().is_ok() {
                    label = format!("Max {next}");
                }
            }
        }
    }
    Some(label)
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// The tier fields Claude Code records beside the account, which say what
/// plan an account is on without reading its token.
pub fn plan_from_claude_json(dir: &Path, home: &Path) -> Option<String> {
    let mut candidates = vec![dir.join(".claude.json")];
    if dir == home.join(".claude") {
        candidates.push(home.join(".claude.json"));
    }
    for path in candidates {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let Some(account) = value.get("oauthAccount") else {
            continue;
        };
        let tier = account
            .get("userRateLimitTier")
            .and_then(|v| v.as_str())
            .or_else(|| account.get("organizationRateLimitTier").and_then(|v| v.as_str()));
        let org_type = account.get("organizationType").and_then(|v| v.as_str());
        if let Some(label) = plan_label(org_type, tier) {
            return Some(label);
        }
    }
    None
}

fn window(value: Option<&Value>, id: &str, title: &str, minutes: u32) -> Option<RateWindow> {
    let value = value?;
    let used = value.get("utilization")?.as_f64()?;
    let resets_at = value
        .get("resets_at")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let epoch = resets_at.as_deref().and_then(parse_timestamp);
    Some(RateWindow {
        id: id.to_string(),
        title: title.to_string(),
        used_percent: used,
        window_minutes: Some(minutes),
        resets_at: epoch.map(format_timestamp).or(resets_at),
        resets_at_epoch: epoch,
    })
}

fn slug(text: &str) -> String {
    let mut out = String::new();
    let mut last_dash = true;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Turn the usage endpoint's JSON into a snapshot.
pub fn parse_usage(body: &str, account: Option<String>, plan: Option<String>, fetched_at: i64) -> Result<UsageSnapshot, UsageError> {
    let value: Value = serde_json::from_str(body).map_err(|e| UsageError::Parse(e.to_string()))?;
    const WEEK: u32 = 7 * 24 * 60;

    let mut windows = Vec::new();
    windows.extend(window(value.get("five_hour"), "session", "Session", 5 * 60));
    windows.extend(window(value.get("seven_day"), "weekly", "Weekly", WEEK));
    windows.extend(window(value.get("seven_day_opus"), "weekly-opus", "Opus weekly", WEEK));
    windows.extend(window(value.get("seven_day_sonnet"), "weekly-sonnet", "Sonnet weekly", WEEK));

    // Model-scoped weekly limits: one per model that has its own cap.
    if let Some(limits) = value.get("limits").and_then(|v| v.as_array()) {
        for limit in limits {
            let scoped = limit.get("group").and_then(|v| v.as_str()) == Some("weekly")
                && limit.get("kind").and_then(|v| v.as_str()) == Some("weekly_scoped");
            if !scoped {
                continue;
            }
            let Some(percent) = limit.get("percent").and_then(|v| v.as_f64()) else {
                continue;
            };
            let model = limit.pointer("/scope/model");
            let name = model
                .and_then(|m| m.get("display_name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if name.is_empty() {
                continue;
            }
            let model_id = model.and_then(|m| m.get("id")).and_then(|v| v.as_str()).unwrap_or(name);
            let id_slug = slug(model_id);
            if slug(name) == "all-models" || id_slug == "all-models" || id_slug.ends_with("-all-models") {
                continue;
            }
            let id = format!("weekly-{id_slug}");
            if windows.iter().any(|w| w.id == id) {
                continue;
            }
            let resets_at = limit.get("resets_at").and_then(|v| v.as_str()).map(str::to_string);
            let epoch = resets_at.as_deref().and_then(parse_timestamp);
            windows.push(RateWindow {
                id,
                title: format!("{name} weekly"),
                used_percent: percent,
                window_minutes: Some(WEEK),
                resets_at: epoch.map(format_timestamp).or(resets_at),
                resets_at_epoch: epoch,
            });
        }
    }

    for key in ["seven_day_routines", "seven_day_claude_routines", "claude_routines", "routines", "seven_day_cowork", "cowork"] {
        if let Some(found) = window(value.get(key), "routines", "Routines weekly", WEEK) {
            windows.push(found);
            break;
        }
    }

    // Money arrives in cents.
    let extra_usage = value.get("extra_usage").map(|extra| ExtraUsage {
        enabled: extra.get("is_enabled").and_then(|v| v.as_bool()).unwrap_or(false),
        used_usd: extra.get("used_credits").and_then(|v| v.as_f64()).map(|c| c / 100.0),
        limit_usd: extra.get("monthly_limit").and_then(|v| v.as_f64()).map(|c| c / 100.0),
        utilization: extra.get("utilization").and_then(|v| v.as_f64()),
    });

    if windows.is_empty() && !extra_usage.as_ref().is_some_and(|e| e.enabled) {
        return Err(UsageError::Parse("the response carried no usage windows".to_string()));
    }

    Ok(UsageSnapshot {
        provider: Provider::Claude,
        account,
        plan,
        windows,
        extra_usage,
        credits: None,
        source: "oauth".to_string(),
        fetched_at,
    })
}

/// Fetch one Claude account's usage.
pub fn fetch(fetcher: &Fetcher, target: &Target) -> Result<UsageSnapshot, UsageError> {
    let (credentials, source) = load_credentials(fetcher, target)?;
    match ask(fetcher, target, &credentials) {
        // A copy the provider no longer accepts is dropped, and the original
        // is tried once — which may be a fresh login Claude Code made since.
        Err(UsageError::Unauthorized { .. }) if source == Source::Copy => {
            if let Some(dir) = &fetcher.credential_cache {
                let _ = std::fs::remove_file(cache_file(dir, &target.dir));
            }
            let (credentials, _) = load_credentials(fetcher, target)?;
            ask(fetcher, target, &credentials)
        }
        other => other,
    }
}

/// One usage request with a given login.
fn ask(fetcher: &Fetcher, target: &Target, credentials: &StoredCredentials) -> Result<UsageSnapshot, UsageError> {
    let user_agent = format!("claude-code/{}", claude_code_version());
    let bearer = format!("Bearer {}", credentials.access_token);
    let response = fetcher.http.get(
        USAGE_URL,
        &[
            ("Authorization", &bearer),
            ("Accept", "application/json"),
            ("Content-Type", "application/json"),
            ("anthropic-beta", BETA_HEADER),
            ("User-Agent", &user_agent),
        ],
    )?;

    match response.status {
        200 => {}
        401 => {
            return Err(UsageError::Unauthorized {
                command: target.command.clone(),
                detail: "the token was rejected".to_string(),
            })
        }
        403 if !credentials.scopes.iter().any(|s| s == "user:profile") => {
            return Err(UsageError::Unauthorized {
                command: target.command.clone(),
                detail: "this login cannot read usage; sign in again to get a token that can".to_string(),
            })
        }
        429 => {
            return Err(UsageError::RateLimited {
                retry_after_seconds: retry_after_seconds(response.retry_after.as_deref()),
            })
        }
        status => {
            return Err(UsageError::Http {
                status,
                body: response.body.chars().take(400).collect(),
            })
        }
    }

    let (account, _) = crate::credentials::account_from_claude_json(&target.dir, &fetcher.home);
    let plan = plan_label(
        credentials.subscription_type.as_deref(),
        credentials.rate_limit_tier.as_deref(),
    )
    .or_else(|| plan_from_claude_json(&target.dir, &fetcher.home));

    parse_usage(&response.body, account, plan, now_epoch())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_credentials_parse_the_camel_case_shape_claude_writes() {
        let json = r#"{"claudeAiOauth":{"accessToken":"tok","refreshToken":"r","expiresAt":1800000000000,"scopes":["user:inference","user:profile"],"subscriptionType":"max","rateLimitTier":"default_claude_max_5x"}}"#;
        let creds = parse_stored(json).unwrap().unwrap();
        assert_eq!(creds.access_token, "tok");
        assert_eq!(creds.refresh_token.as_deref(), Some("r"));
        assert_eq!(parse_stored(&creds.to_json()).unwrap().unwrap(), creds, "a copy reads back identically");
        assert_eq!(creds.expires_at_ms, Some(1_800_000_000_000));
        assert_eq!(creds.scopes, vec!["user:inference", "user:profile"]);
        assert!(!creds.is_expired(1_700_000_000_000));
        assert!(creds.is_expired(1_900_000_000_000));
    }

    #[test]
    fn an_mcp_only_keychain_item_is_not_a_login() {
        assert_eq!(parse_stored(r#"{"mcpOAuth":{"x":{}}}"#).unwrap(), None);
        assert!(parse_stored("nope").is_err());
    }

    #[test]
    fn plan_labels_prefer_the_subscription_and_carry_the_max_multiplier() {
        assert_eq!(plan_label(Some("max"), Some("default_claude_max_5x")).as_deref(), Some("Max 5x"));
        assert_eq!(plan_label(None, Some("default_claude_max_20x")).as_deref(), Some("Max 20x"));
        assert_eq!(plan_label(Some("pro"), Some("default_claude_max_5x")).as_deref(), Some("Pro"));
        assert_eq!(plan_label(Some("claude_max"), None).as_deref(), Some("Max"));
        assert_eq!(plan_label(None, Some("team_tier_1")).as_deref(), Some("Team"));
        assert_eq!(plan_label(None, None), None);
        assert_eq!(plan_label(Some("free"), Some("default")), None);
    }

    const USAGE: &str = r#"{
        "five_hour": {"utilization": 12.5, "resets_at": "2026-09-16T12:00:00+00:00"},
        "seven_day": {"utilization": 40, "resets_at": "2026-09-20T00:00:00Z"},
        "seven_day_opus": {"utilization": 3.25, "resets_at": "2026-09-20T00:00:00Z"},
        "limits": [
            {"kind": "weekly_scoped", "group": "weekly", "percent": 9, "resets_at": "2026-09-20T00:00:00Z",
             "scope": {"model": {"id": "claude-fable-5-1", "display_name": "Fable"}}, "is_active": true},
            {"kind": "weekly_scoped", "group": "weekly", "percent": 40,
             "scope": {"model": {"id": "all-models", "display_name": "All models"}}},
            {"kind": "other", "group": "weekly", "percent": 1}
        ],
        "extra_usage": {"is_enabled": true, "monthly_limit": 5000, "used_credits": 1234, "utilization": 24.68, "currency": "USD"}
    }"#;

    #[test]
    fn usage_windows_are_mapped_in_a_stable_order() {
        let snap = parse_usage(USAGE, Some("me@x.io".into()), Some("Max 5x".into()), 1).unwrap();
        let ids: Vec<&str> = snap.windows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, vec!["session", "weekly", "weekly-opus", "weekly-claude-fable-5-1"]);
        let session = snap.window("session").unwrap();
        assert_eq!(session.used_percent, 12.5);
        assert_eq!(session.window_minutes, Some(300));
        assert_eq!(session.resets_at.as_deref(), Some("2026-09-16T12:00:00Z"));
        assert_eq!(session.resets_at_epoch, parse_timestamp("2026-09-16T12:00:00Z"));
        assert_eq!(snap.window("weekly-claude-fable-5-1").unwrap().title, "Fable weekly");
        assert_eq!(snap.plan.as_deref(), Some("Max 5x"));
        assert_eq!(snap.source, "oauth");
    }

    #[test]
    fn extra_usage_money_is_converted_from_cents() {
        let snap = parse_usage(USAGE, None, None, 1).unwrap();
        let extra = snap.extra_usage.unwrap();
        assert!(extra.enabled);
        assert_eq!(extra.used_usd, Some(12.34));
        assert_eq!(extra.limit_usd, Some(50.0));
    }

    #[test]
    fn a_response_without_windows_is_an_error_not_an_empty_snapshot() {
        assert!(matches!(parse_usage("{}", None, None, 1), Err(UsageError::Parse(_))));
        assert!(matches!(parse_usage("[", None, None, 1), Err(UsageError::Parse(_))));
    }

    #[test]
    fn a_weekly_only_response_still_reads() {
        let snap = parse_usage(r#"{"seven_day": {"utilization": 5}}"#, None, None, 1).unwrap();
        assert_eq!(snap.windows.len(), 1);
        assert_eq!(snap.windows[0].resets_at, None);
    }

    #[test]
    fn slugs_are_lowercase_and_dashed() {
        assert_eq!(slug("Claude Fable 5.1"), "claude-fable-5-1");
        assert_eq!(slug("--x--"), "x");
    }

    #[test]
    fn plan_is_read_from_claude_json_without_a_token() {
        let d = tempfile::TempDir::new().unwrap();
        std::fs::write(
            d.path().join(".claude.json"),
            r#"{"oauthAccount":{"organizationType":"claude_max","organizationRateLimitTier":"default_claude_max_5x"}}"#,
        )
        .unwrap();
        assert_eq!(plan_from_claude_json(d.path(), d.path()).as_deref(), Some("Max 5x"));
    }
}

#[cfg(test)]
mod copy_tests {
    use super::*;
    use crate::usage::http::FakeHttp;
    use crate::usage::{ClaudeCredentialReader, Fetcher, Target};
    use std::cell::Cell;

    /// A reader that hands out one login and counts how often it is asked.
    struct Original {
        json: String,
        reads: Cell<usize>,
    }

    impl ClaudeCredentialReader for Original {
        fn read(&self, _dir: &Path, _home: &Path, _prompt: bool) -> Result<Option<String>, UsageError> {
            self.reads.set(self.reads.get() + 1);
            Ok(Some(self.json.clone()))
        }
    }

    const USAGE: &str = r#"{"five_hour":{"utilization":10,"resets_at":"2026-09-16T12:00:00Z"}}"#;

    fn login(expires_at_ms: i64) -> String {
        StoredCredentials {
            access_token: "orig".into(),
            refresh_token: Some("refresh-1".into()),
            expires_at_ms: Some(expires_at_ms),
            scopes: vec!["user:profile".into()],
            subscription_type: Some("max".into()),
            rate_limit_tier: None,
        }
        .to_json()
    }

    fn far() -> i64 {
        now_epoch() * 1000 + 24 * 3600 * 1000
    }

    fn target(dir: &tempfile::TempDir) -> Target {
        Target {
            profile: Some("work".into()),
            provider: Provider::Claude,
            dir: dir.path().join("work"),
            command: "claude-work".into(),
        }
    }

    fn bearer(http: &FakeHttp, nth: usize) -> String {
        http.requests.borrow()[nth].1.iter().find(|(k, _)| k == "Authorization").unwrap().1.clone()
    }

    #[test]
    fn the_original_is_read_once_and_the_copy_serves_after_that() {
        let d = tempfile::TempDir::new().unwrap();
        let http = FakeHttp::new();
        http.respond("/api/oauth/usage", 200, USAGE);
        let original = Original { json: login(far()), reads: Cell::new(0) };
        let cache = d.path().join("cache");
        let fetcher = Fetcher {
            http: &http,
            claude_credentials: &original,
            codex_binary: None,
            home: d.path().to_path_buf(),
            keychain_prompt: false,
            credential_cache: Some(cache.clone()),
        };
        let target = target(&d);

        fetcher.fetch(&target).unwrap();
        fetcher.fetch(&target).unwrap();
        assert_eq!(original.reads.get(), 1, "the second fetch must not touch the Keychain");

        let file = cache_file(&cache, &target.dir);
        assert!(file.is_file());
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(&cache).unwrap().permissions().mode() & 0o777, 0o700);
    }

    #[test]
    fn without_a_cache_directory_nothing_is_copied() {
        let d = tempfile::TempDir::new().unwrap();
        let http = FakeHttp::new();
        http.respond("/api/oauth/usage", 200, USAGE);
        let original = Original { json: login(far()), reads: Cell::new(0) };
        let fetcher = Fetcher {
            http: &http,
            claude_credentials: &original,
            codex_binary: None,
            home: d.path().to_path_buf(),
            keychain_prompt: false,
            credential_cache: None,
        };
        let target = target(&d);
        fetcher.fetch(&target).unwrap();
        fetcher.fetch(&target).unwrap();
        assert_eq!(original.reads.get(), 2);
        assert!(!d.path().join("cache").exists());
    }

    #[test]
    fn an_expiring_copy_is_renewed_with_its_refresh_token_not_the_keychain() {
        let d = tempfile::TempDir::new().unwrap();
        let http = FakeHttp::new();
        http.respond("/api/oauth/usage", 200, USAGE);
        http.respond("/v1/oauth/token", 200, r#"{"access_token":"renewed","refresh_token":"refresh-2","expires_in":28800}"#);
        let cache = d.path().join("cache");
        let target = target(&d);
        // A copy that expires in a minute: inside the renewal margin.
        write_cache(
            &cache_file(&cache, &target.dir),
            &parse_stored(&login(now_epoch() * 1000 + 60_000)).unwrap().unwrap(),
        )
        .unwrap();
        let original = Original { json: login(far()), reads: Cell::new(0) };
        let fetcher = Fetcher {
            http: &http,
            claude_credentials: &original,
            codex_binary: None,
            home: d.path().to_path_buf(),
            keychain_prompt: false,
            credential_cache: Some(cache.clone()),
        };

        fetcher.fetch(&target).unwrap();
        assert_eq!(original.reads.get(), 0);
        assert_eq!(http.calls("/v1/oauth/token"), 1);
        let form = http.headers_sent("/v1/oauth/token");
        assert!(form.contains(&("form:grant_type".to_string(), "refresh_token".to_string())), "{form:?}");
        assert!(form.contains(&("form:refresh_token".to_string(), "refresh-1".to_string())), "{form:?}");
        assert!(form.contains(&("form:client_id".to_string(), CLIENT_ID.to_string())), "{form:?}");
        assert_eq!(bearer(&http, 1), "Bearer renewed");

        let saved = parse_stored(&std::fs::read_to_string(cache_file(&cache, &target.dir)).unwrap()).unwrap().unwrap();
        assert_eq!(saved.access_token, "renewed");
        assert_eq!(saved.refresh_token.as_deref(), Some("refresh-2"));
        assert!(saved.expires_at_ms.unwrap() > now_epoch() * 1000 + 7 * 3600 * 1000);
        assert_eq!(saved.subscription_type.as_deref(), Some("max"), "what the copy knew about the plan is kept");
    }

    #[test]
    fn a_rejected_refresh_drops_the_copy_and_goes_back_to_the_original() {
        let d = tempfile::TempDir::new().unwrap();
        let http = FakeHttp::new();
        http.respond("/api/oauth/usage", 200, USAGE);
        http.respond("/v1/oauth/token", 400, r#"{"error":"invalid_grant"}"#);
        let cache = d.path().join("cache");
        let target = target(&d);
        write_cache(&cache_file(&cache, &target.dir), &parse_stored(&login(1000)).unwrap().unwrap()).unwrap();
        let original = Original { json: login(far()), reads: Cell::new(0) };
        let fetcher = Fetcher {
            http: &http,
            claude_credentials: &original,
            codex_binary: None,
            home: d.path().to_path_buf(),
            keychain_prompt: false,
            credential_cache: Some(cache.clone()),
        };

        fetcher.fetch(&target).unwrap();
        assert_eq!(original.reads.get(), 1);
        assert_eq!(bearer(&http, 1), "Bearer orig");
        // And the fresh original was copied for next time.
        let saved = parse_stored(&std::fs::read_to_string(cache_file(&cache, &target.dir)).unwrap()).unwrap().unwrap();
        assert_eq!(saved.access_token, "orig");
    }

    #[test]
    fn a_transient_refresh_failure_is_reported_and_keeps_the_copy() {
        let d = tempfile::TempDir::new().unwrap();
        let http = FakeHttp::new();
        http.respond("/v1/oauth/token", 503, "later");
        let cache = d.path().join("cache");
        let target = target(&d);
        write_cache(&cache_file(&cache, &target.dir), &parse_stored(&login(1000)).unwrap().unwrap()).unwrap();
        let original = Original { json: login(far()), reads: Cell::new(0) };
        let fetcher = Fetcher {
            http: &http,
            claude_credentials: &original,
            codex_binary: None,
            home: d.path().to_path_buf(),
            keychain_prompt: false,
            credential_cache: Some(cache.clone()),
        };
        assert!(matches!(fetcher.fetch(&target), Err(UsageError::Http { status: 503, .. })));
        assert_eq!(original.reads.get(), 0);
        assert!(cache_file(&cache, &target.dir).is_file());
    }

    #[test]
    fn a_copy_the_provider_rejects_is_dropped_and_the_original_tried_once() {
        let d = tempfile::TempDir::new().unwrap();
        let http = FakeHttp::new();
        // The first usage call answers 401, the retry 200: order matters,
        // so respond by the bearer the fake cannot see — use two fetchers.
        http.respond("/api/oauth/usage", 401, "no");
        let cache = d.path().join("cache");
        let target = target(&d);
        let stale = StoredCredentials {
            access_token: "stale".into(),
            refresh_token: None,
            expires_at_ms: Some(far()),
            scopes: vec![],
            subscription_type: None,
            rate_limit_tier: None,
        };
        write_cache(&cache_file(&cache, &target.dir), &stale).unwrap();
        let original = Original { json: login(far()), reads: Cell::new(0) };
        let fetcher = Fetcher {
            http: &http,
            claude_credentials: &original,
            codex_binary: None,
            home: d.path().to_path_buf(),
            keychain_prompt: false,
            credential_cache: Some(cache.clone()),
        };
        let result = fetcher.fetch(&target);
        assert!(matches!(result, Err(UsageError::Unauthorized { .. })), "{result:?}");
        assert_eq!(original.reads.get(), 1, "the original is consulted once after the copy fails");
        assert_eq!(bearer(&http, 0), "Bearer stale");
        assert_eq!(bearer(&http, 1), "Bearer orig");
    }

    #[test]
    fn forgetting_removes_every_copy() {
        let d = tempfile::TempDir::new().unwrap();
        let cache = crate::usage::credential_cache_dir(d.path());
        for name in ["a", "b"] {
            write_cache(&cache_file(&cache, Path::new(name)), &parse_stored(&login(far())).unwrap().unwrap()).unwrap();
        }
        assert_eq!(crate::usage::forget_credentials(d.path()).unwrap(), 2);
        assert_eq!(crate::usage::forget_credentials(d.path()).unwrap(), 0);
        assert_eq!(crate::usage::forget_credentials(&d.path().join("nowhere")).unwrap(), 0);
    }
}
