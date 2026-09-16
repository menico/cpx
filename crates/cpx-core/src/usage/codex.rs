//! Codex usage, from the ChatGPT backend Codex itself reports to.
//!
//! Codex keeps its login in `auth.json` inside its home, so a profile's
//! usage is a matter of reading that file and asking. The access token is
//! short-lived and the Codex CLI owns its renewal: when it is about to
//! expire, the question goes through `codex app-server` instead, which
//! refreshes the file as a side effect. Nothing here ever writes `auth.json`.

use super::http::retry_after_seconds;
use super::{format_timestamp, now_epoch, Credits, Fetcher, RateWindow, Target, UsageError, UsageSnapshot};
use crate::config::Provider;
use crate::credentials::{codex_auth_file, jwt_claims};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};

const DEFAULT_BASE_URL: &str = "https://chatgpt.com/backend-api";

/// How close to expiry a token is treated as stale, so a request does not
/// race the tool's own refresh.
const REFRESH_WINDOW_SECONDS: i64 = 5 * 60;

/// What `auth.json` holds that a usage request needs.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthFile {
    pub access_token: String,
    pub account_id: Option<String>,
    pub email: Option<String>,
    pub plan: Option<String>,
    /// From the access token's `exp` claim, seconds since the epoch.
    pub expires_at: Option<i64>,
    pub is_api_key: bool,
}

impl AuthFile {
    pub fn needs_refresh(&self, now: i64) -> bool {
        !self.is_api_key && self.expires_at.is_some_and(|at| at - now <= REFRESH_WINDOW_SECONDS)
    }
}

pub fn parse_auth(json: &str) -> Result<Option<AuthFile>, UsageError> {
    let value: Value = serde_json::from_str(json).map_err(|e| UsageError::Parse(format!("auth.json: {e}")))?;

    if let Some(tokens) = value.get("tokens").filter(|t| t.is_object()) {
        let access = tokens
            .get("access_token")
            .or_else(|| tokens.get("accessToken"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());
        if let Some(access) = access {
            let id_token = tokens
                .get("id_token")
                .or_else(|| tokens.get("idToken"))
                .and_then(|v| v.as_str());
            let claims = id_token.and_then(jwt_claims);
            let auth = claims.as_ref().and_then(|c| c.get("https://api.openai.com/auth"));
            let account_id = tokens
                .get("account_id")
                .or_else(|| tokens.get("accountId"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    auth.and_then(|a| a.get("chatgpt_account_id"))
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                });
            return Ok(Some(AuthFile {
                access_token: access.to_string(),
                account_id,
                email: claims
                    .as_ref()
                    .and_then(|c| c.get("email"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                plan: auth
                    .and_then(|a| a.get("chatgpt_plan_type"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                expires_at: jwt_claims(access).and_then(|c| c.get("exp")?.as_i64()),
                is_api_key: false,
            }));
        }
    }

    if let Some(key) = value.get("OPENAI_API_KEY").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()) {
        return Ok(Some(AuthFile {
            access_token: key.trim().to_string(),
            account_id: None,
            email: None,
            plan: None,
            expires_at: None,
            is_api_key: true,
        }));
    }

    Ok(None)
}

/// The backend base URL, honouring a `chatgpt_base_url` override in the
/// home's `config.toml` the way Codex does.
pub fn base_url(home_dir: &Path) -> String {
    let configured = std::fs::read_to_string(home_dir.join("config.toml"))
        .ok()
        .and_then(|text| {
            text.lines()
                .map(|line| line.split('#').next().unwrap_or("").trim())
                .find_map(|line| {
                    let (key, value) = line.split_once('=')?;
                    (key.trim() == "chatgpt_base_url").then(|| value.trim().trim_matches(['"', '\'']).to_string())
                })
        })
        .filter(|s| !s.is_empty());
    let mut base = configured.unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
    while base.ends_with('/') {
        base.pop();
    }
    let chatgpt_host = base.starts_with("https://chatgpt.com") || base.starts_with("https://chat.openai.com");
    if chatgpt_host && !base.contains("/backend-api") {
        base.push_str("/backend-api");
    }
    base
}

pub fn usage_url(base: &str) -> String {
    if base.contains("/backend-api") {
        format!("{base}/wham/usage")
    } else {
        format!("{base}/api/codex/usage")
    }
}

/// A plan label from Codex's `plan_type`.
pub fn plan_label(plan_type: Option<&str>) -> Option<String> {
    let raw = plan_type?.trim();
    if raw.is_empty() || raw == "unknown" {
        return None;
    }
    Some(match raw {
        "plus" => "Plus".to_string(),
        "pro" => "Pro".to_string(),
        "team" => "Team".to_string(),
        "business" => "Business".to_string(),
        "enterprise" => "Enterprise".to_string(),
        "free" => "Free".to_string(),
        "go" => "Go".to_string(),
        "edu" | "education" => "Education".to_string(),
        other => {
            let mut chars = other.chars();
            let first = chars.next()?;
            first.to_uppercase().collect::<String>() + chars.as_str()
        }
    })
}

fn window_from_oauth(value: Option<&Value>, id: &str, title: &str) -> Option<RateWindow> {
    let value = value?;
    let used = value.get("used_percent")?.as_f64()?;
    let reset_at = value.get("reset_at").and_then(|v| v.as_i64());
    let minutes = value
        .get("limit_window_seconds")
        .and_then(|v| v.as_u64())
        .map(|s| (s / 60) as u32);
    Some(RateWindow {
        id: id.to_string(),
        title: title.to_string(),
        used_percent: used,
        window_minutes: minutes,
        resets_at: reset_at.map(format_timestamp),
        resets_at_epoch: reset_at,
    })
}

fn credits_from(value: Option<&Value>) -> Option<Credits> {
    let value = value?;
    Some(Credits {
        has_credits: value.get("has_credits").and_then(|v| v.as_bool()).unwrap_or(false),
        unlimited: value.get("unlimited").and_then(|v| v.as_bool()).unwrap_or(false),
        balance: value.get("balance").and_then(|v| match v {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.parse().ok(),
            _ => None,
        }),
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
pub fn parse_usage(body: &str, auth: &AuthFile, fetched_at: i64) -> Result<UsageSnapshot, UsageError> {
    let value: Value = serde_json::from_str(body).map_err(|e| UsageError::Parse(e.to_string()))?;
    let rate_limit = value.get("rate_limit");

    let mut windows = Vec::new();
    windows.extend(window_from_oauth(rate_limit.and_then(|r| r.get("primary_window")), "session", "Session"));
    windows.extend(window_from_oauth(rate_limit.and_then(|r| r.get("secondary_window")), "weekly", "Weekly"));

    if let Some(extra) = value.get("additional_rate_limits").and_then(|v| v.as_array()) {
        for limit in extra {
            let name = limit
                .get("limit_name")
                .and_then(|v| v.as_str())
                .or_else(|| limit.get("metered_feature").and_then(|v| v.as_str()))
                .unwrap_or("Extra");
            let inner = limit.get("rate_limit");
            let base = slug(name);
            windows.extend(window_from_oauth(
                inner.and_then(|r| r.get("primary_window")),
                &format!("{base}-session"),
                &format!("{name} session"),
            ));
            windows.extend(window_from_oauth(
                inner.and_then(|r| r.get("secondary_window")),
                &format!("{base}-weekly"),
                &format!("{name} weekly"),
            ));
        }
    }

    if windows.is_empty() {
        return Err(UsageError::Parse("the response carried no rate-limit windows".to_string()));
    }

    let plan = value
        .get("plan_type")
        .and_then(|v| v.as_str())
        .and_then(|p| plan_label(Some(p)))
        .or_else(|| plan_label(auth.plan.as_deref()));

    Ok(UsageSnapshot {
        provider: Provider::Codex,
        account: auth.email.clone(),
        plan,
        windows,
        extra_usage: None,
        credits: credits_from(value.get("credits")),
        source: "oauth".to_string(),
        fetched_at,
    })
}

/// Ask a running `codex app-server` for the rate limits.
///
/// The protocol is newline-delimited JSON-RPC without the `jsonrpc` field:
/// `initialize`, an `initialized` notification, then
/// `account/rateLimits/read`. The CLI refreshes `auth.json` itself on the
/// way, which is the whole reason to go through it.
pub fn rpc_rate_limits(binary: &Path, home_dir: &Path, timeout: std::time::Duration) -> Result<Value, UsageError> {
    let mut child = Command::new(binary)
        .args(["-s", "read-only", "-a", "never", "app-server"])
        .env("CODEX_HOME", home_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| UsageError::Cli(format!("could not start `codex app-server`: {e}")))?;

    let result = (|| {
        let mut stdin = child.stdin.take().ok_or_else(|| UsageError::Cli("no stdin".into()))?;
        let stdout = child.stdout.take().ok_or_else(|| UsageError::Cli("no stdout".into()))?;
        let mut reader = BufReader::new(stdout);

        let write = |stdin: &mut std::process::ChildStdin, payload: Value| -> Result<(), UsageError> {
            let mut line = payload.to_string();
            line.push('\n');
            stdin
                .write_all(line.as_bytes())
                .map_err(|e| UsageError::Cli(format!("codex app-server closed its input: {e}")))
        };

        // Wait for the reply to request `id`, skipping notifications. The
        // deadline bounds a CLI that never answers.
        let deadline = std::time::Instant::now() + timeout;
        let await_reply = |reader: &mut BufReader<std::process::ChildStdout>, id: u64| -> Result<Value, UsageError> {
            loop {
                if std::time::Instant::now() > deadline {
                    return Err(UsageError::Cli("codex app-server did not answer in time".into()));
                }
                let mut line = String::new();
                let read = reader
                    .read_line(&mut line)
                    .map_err(|e| UsageError::Cli(format!("reading from codex app-server: {e}")))?;
                if read == 0 {
                    return Err(UsageError::Cli("codex app-server exited before answering".into()));
                }
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if message.get("id").and_then(|v| v.as_u64()) != Some(id) {
                    continue;
                }
                if let Some(error) = message.get("error") {
                    let text = error
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown error");
                    return Err(UsageError::Cli(format!("codex app-server: {text}")));
                }
                return Ok(message.get("result").cloned().unwrap_or(Value::Null));
            }
        };

        write(
            &mut stdin,
            serde_json::json!({
                "id": 1,
                "method": "initialize",
                "params": {"clientInfo": {"name": "cpx", "version": env!("CARGO_PKG_VERSION")}}
            }),
        )?;
        await_reply(&mut reader, 1)?;
        write(&mut stdin, serde_json::json!({"method": "initialized", "params": {}}))?;
        write(
            &mut stdin,
            serde_json::json!({"id": 2, "method": "account/rateLimits/read", "params": {}}),
        )?;
        await_reply(&mut reader, 2)
    })();

    let _ = child.kill();
    let _ = child.wait();
    result
}

/// Turn an app-server `account/rateLimits/read` result into a snapshot.
pub fn parse_rpc(result: &Value, auth: &AuthFile, fetched_at: i64) -> Result<UsageSnapshot, UsageError> {
    let limits = result.get("rateLimits").unwrap_or(result);

    let window = |value: Option<&Value>, id: &str, title: &str| -> Option<RateWindow> {
        let value = value?;
        let used = value.get("usedPercent")?.as_f64()?;
        let reset = value.get("resetsAt").and_then(|v| v.as_i64());
        Some(RateWindow {
            id: id.to_string(),
            title: title.to_string(),
            used_percent: used,
            window_minutes: value.get("windowDurationMins").and_then(|v| v.as_u64()).map(|m| m as u32),
            resets_at: reset.map(format_timestamp),
            resets_at_epoch: reset,
        })
    };

    let mut windows = Vec::new();
    windows.extend(window(limits.get("primary"), "session", "Session"));
    windows.extend(window(limits.get("secondary"), "weekly", "Weekly"));
    if windows.is_empty() {
        return Err(UsageError::Parse("codex app-server reported no rate-limit windows".into()));
    }

    let credits = limits.get("credits").map(|c| Credits {
        has_credits: c.get("hasCredits").and_then(|v| v.as_bool()).unwrap_or(false),
        unlimited: c.get("unlimited").and_then(|v| v.as_bool()).unwrap_or(false),
        balance: c.get("balance").and_then(|v| match v {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.parse().ok(),
            _ => None,
        }),
    });

    Ok(UsageSnapshot {
        provider: Provider::Codex,
        account: auth.email.clone(),
        plan: limits
            .get("planType")
            .and_then(|v| v.as_str())
            .and_then(|p| plan_label(Some(p)))
            .or_else(|| plan_label(auth.plan.as_deref())),
        windows,
        extra_usage: None,
        credits,
        source: "cli".to_string(),
        fetched_at,
    })
}

/// Fetch one Codex account's usage.
pub fn fetch(fetcher: &Fetcher, target: &Target) -> Result<UsageSnapshot, UsageError> {
    let path = codex_auth_file(&target.dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(UsageError::NotSignedIn),
        Err(e) => return Err(UsageError::Parse(format!("{}: {e}", path.display()))),
    };
    let Some(auth) = parse_auth(&text)? else {
        return Err(UsageError::NotSignedIn);
    };
    if auth.is_api_key {
        return Err(UsageError::Unauthorized {
            command: format!("{} login", target.command),
            detail: "signed in with an API key, which has no rate-limit windows to show".to_string(),
        });
    }

    let via_cli = |reason: UsageError| -> Result<UsageSnapshot, UsageError> {
        let Some(binary) = &fetcher.codex_binary else {
            return Err(match reason {
                UsageError::Unauthorized { .. } | UsageError::TokenExpired { .. } => UsageError::TokenExpired {
                    command: target.command.clone(),
                },
                other => other,
            });
        };
        let result = rpc_rate_limits(binary, &target.dir, std::time::Duration::from_secs(20))?;
        parse_rpc(&result, &auth, now_epoch())
    };

    if auth.needs_refresh(now_epoch()) {
        return via_cli(UsageError::TokenExpired {
            command: target.command.clone(),
        });
    }

    let base = base_url(&target.dir);
    let bearer = format!("Bearer {}", auth.access_token);
    let mut headers: Vec<(&str, &str)> = vec![
        ("Authorization", &bearer),
        ("User-Agent", "cpx"),
        ("Accept", "application/json"),
    ];
    if let Some(account_id) = &auth.account_id {
        headers.push(("ChatGPT-Account-Id", account_id));
    }
    let response = fetcher.http.get(&usage_url(&base), &headers)?;

    match response.status {
        200 => parse_usage(&response.body, &auth, now_epoch()),
        401 => via_cli(UsageError::Unauthorized {
            command: format!("{} login", target.command),
            detail: "the token was rejected".to_string(),
        }),
        429 => Err(UsageError::RateLimited {
            retry_after_seconds: retry_after_seconds(response.retry_after.as_deref()),
        }),
        status => Err(UsageError::Http {
            status,
            body: response.body.chars().take(400).collect(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(claims: &str) -> String {
        use base64::Engine;
        let b64 = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
        format!("{}.{}.{}", b64("{}"), b64(claims), b64("s"))
    }

    fn auth() -> AuthFile {
        AuthFile {
            access_token: "tok".into(),
            account_id: Some("acc".into()),
            email: Some("me@x.io".into()),
            plan: Some("plus".into()),
            expires_at: Some(2_000_000_000),
            is_api_key: false,
        }
    }

    #[test]
    fn auth_json_yields_the_token_identity_and_expiry() {
        let access = jwt(r#"{"exp": 1900000000}"#);
        let id = jwt(r#"{"email":"me@x.io","https://api.openai.com/auth":{"chatgpt_plan_type":"pro","chatgpt_account_id":"acc-1"}}"#);
        let text = format!(r#"{{"tokens":{{"access_token":"{access}","id_token":"{id}","refresh_token":"r"}},"last_refresh":"2026-01-01T00:00:00Z"}}"#);
        let auth = parse_auth(&text).unwrap().unwrap();
        assert_eq!(auth.email.as_deref(), Some("me@x.io"));
        assert_eq!(auth.plan.as_deref(), Some("pro"));
        assert_eq!(auth.account_id.as_deref(), Some("acc-1"));
        assert_eq!(auth.expires_at, Some(1_900_000_000));
        assert!(!auth.needs_refresh(1_800_000_000));
        assert!(auth.needs_refresh(1_900_000_000 - 100));
        assert!(!auth.is_api_key);
    }

    #[test]
    fn an_api_key_login_is_recognised() {
        let auth = parse_auth(r#"{"OPENAI_API_KEY":" sk-1 "}"#).unwrap().unwrap();
        assert!(auth.is_api_key);
        assert_eq!(auth.access_token, "sk-1");
        assert!(!auth.needs_refresh(0));
        assert!(parse_auth("{}").unwrap().is_none());
    }

    #[test]
    fn the_base_url_defaults_and_honours_config_toml() {
        let d = tempfile::TempDir::new().unwrap();
        assert_eq!(base_url(d.path()), "https://chatgpt.com/backend-api");
        std::fs::write(d.path().join("config.toml"), "chatgpt_base_url = \"https://chatgpt.com/\" # note\n").unwrap();
        assert_eq!(base_url(d.path()), "https://chatgpt.com/backend-api");
        std::fs::write(d.path().join("config.toml"), "chatgpt_base_url = 'https://proxy.example/v1/'\n").unwrap();
        assert_eq!(base_url(d.path()), "https://proxy.example/v1");
        assert_eq!(usage_url("https://proxy.example/v1"), "https://proxy.example/v1/api/codex/usage");
        assert_eq!(usage_url("https://chatgpt.com/backend-api"), "https://chatgpt.com/backend-api/wham/usage");
    }

    const USAGE: &str = r#"{
        "plan_type": "pro",
        "rate_limit": {
            "primary_window": {"used_percent": 33, "reset_at": 1789000000, "limit_window_seconds": 18000},
            "secondary_window": {"used_percent": 7, "reset_at": 1789500000, "limit_window_seconds": 604800}
        },
        "credits": {"has_credits": true, "unlimited": false, "balance": "12.5"},
        "additional_rate_limits": [
            {"limit_name": "GPT-5.3-Codex-Spark", "rate_limit": {"primary_window": {"used_percent": 1, "reset_at": 1789000000, "limit_window_seconds": 18000}}}
        ]
    }"#;

    #[test]
    fn usage_windows_come_from_the_rate_limit_lanes() {
        let snap = parse_usage(USAGE, &auth(), 5).unwrap();
        let ids: Vec<&str> = snap.windows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, vec!["session", "weekly", "gpt-5-3-codex-spark-session"]);
        let session = snap.window("session").unwrap();
        assert_eq!(session.used_percent, 33.0);
        assert_eq!(session.window_minutes, Some(300));
        assert_eq!(session.resets_at_epoch, Some(1_789_000_000));
        assert_eq!(session.resets_at.as_deref(), Some("2026-09-10T00:26:40Z"));
        assert_eq!(snap.plan.as_deref(), Some("Pro"));
        assert_eq!(snap.account.as_deref(), Some("me@x.io"));
        assert_eq!(snap.credits.as_ref().unwrap().balance, Some(12.5));
    }

    #[test]
    fn the_plan_falls_back_to_the_identity_token() {
        let snap = parse_usage(r#"{"rate_limit":{"primary_window":{"used_percent":0}}}"#, &auth(), 5).unwrap();
        assert_eq!(snap.plan.as_deref(), Some("Plus"));
        assert_eq!(snap.windows[0].resets_at, None);
    }

    #[test]
    fn an_empty_rate_limit_is_an_error() {
        assert!(matches!(parse_usage("{}", &auth(), 5), Err(UsageError::Parse(_))));
    }

    #[test]
    fn app_server_results_map_to_the_same_windows() {
        let result = serde_json::json!({
            "rateLimits": {
                "primary": {"usedPercent": 12.5, "windowDurationMins": 300, "resetsAt": 1789000000},
                "secondary": {"usedPercent": 40, "windowDurationMins": 10080, "resetsAt": 1789500000},
                "credits": {"hasCredits": true, "unlimited": true, "balance": null},
                "planType": "team"
            }
        });
        let snap = parse_rpc(&result, &auth(), 5).unwrap();
        assert_eq!(snap.source, "cli");
        assert_eq!(snap.window("session").unwrap().used_percent, 12.5);
        assert_eq!(snap.window("weekly").unwrap().window_minutes, Some(10080));
        assert_eq!(snap.plan.as_deref(), Some("Team"));
        assert!(snap.credits.unwrap().unlimited);
    }

    #[test]
    fn plan_labels_are_readable() {
        assert_eq!(plan_label(Some("plus")).as_deref(), Some("Plus"));
        assert_eq!(plan_label(Some("free_workspace")).as_deref(), Some("Free_workspace"));
        assert_eq!(plan_label(Some("unknown")), None);
        assert_eq!(plan_label(None), None);
    }
}
