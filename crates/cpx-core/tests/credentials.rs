//! Credential status. Nothing here reads a token.

use cpx_core::credentials::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// The keychain probe is replaced once, process-wide: these tests must never
/// touch the real Keychain, and must never make macOS prompt.
fn stub_keychain() {
    set_keychain_lookup(|service, _account| service.ends_with("-loggedin"));
}

#[test]
fn the_service_name_is_the_base_plus_a_digest_of_the_config_dir() {
    let name = keychain_service(Path::new("/Users/t/.claude-profiles/work"));
    let suffix = name
        .strip_prefix(&format!("{KEYCHAIN_SERVICE_BASE}-"))
        .expect("service should be prefixed by the base");
    assert_eq!(suffix.len(), 8, "expected 8 hex chars, got {suffix:?}");
    assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()), "{suffix}");
}

#[test]
fn different_profiles_get_different_services() {
    assert_ne!(
        keychain_service(Path::new("/Users/t/.claude-profiles/work")),
        keychain_service(Path::new("/Users/t/.claude-profiles/personal")),
        "profiles sharing a service would overwrite each other's logins"
    );
}

#[test]
fn the_same_config_dir_always_yields_the_same_service() {
    let dir = Path::new("/Users/t/.claude-profiles/work");
    assert_eq!(keychain_service(dir), keychain_service(dir));
}

#[test]
fn a_profile_service_never_collides_with_the_default_login() {
    let profile = keychain_service(Path::new("/Users/t/.claude-profiles/work"));
    assert_ne!(
        profile, KEYCHAIN_SERVICE_BASE,
        "a profile must not be able to clobber the bare ~/.claude session"
    );
}

#[test]
fn the_account_name_is_the_user() {
    assert_eq!(keychain_account(Some("meni")), "meni");
    assert_eq!(keychain_account(Some("first.last_1-2")), "first.last_1-2");
}

#[test]
fn an_unusable_user_name_falls_back_the_way_claude_code_does() {
    for bad in ["", "has space", "hös", "semi;colon"] {
        assert_eq!(
            keychain_account(Some(bad)),
            "claude-code-user",
            "input {bad:?}"
        );
    }
}

#[test]
fn an_absent_profile_is_not_authenticated() {
    stub_keychain();
    let d = TempDir::new().unwrap();
    let status = status(&d.path().join("never-created"), d.path());
    assert!(!status.authenticated);
    assert_eq!(status.source, CredentialSource::None);
}

#[test]
fn a_keychain_entry_makes_a_profile_authenticated() {
    set_keychain_lookup(|_, _| true);
    let d = TempDir::new().unwrap();
    let status = status(d.path(), d.path());
    assert!(status.authenticated);
    assert_eq!(status.source, CredentialSource::Keychain);
}

#[test]
fn a_credentials_file_authenticates_when_there_is_no_keychain() {
    set_keychain_lookup(|_, _| false);
    let d = TempDir::new().unwrap();
    fs::write(
        credentials_file(d.path()),
        r#"{"claudeAiOauth":{"expiresAt":99999999999999}}"#,
    )
    .unwrap();

    let status = status(d.path(), d.path());
    assert!(status.authenticated);
    assert_eq!(status.source, CredentialSource::File);
    assert_eq!(status.expired, Some(false));
}

#[test]
fn an_expired_credentials_file_is_reported_as_expired() {
    set_keychain_lookup(|_, _| false);
    let d = TempDir::new().unwrap();
    fs::write(
        credentials_file(d.path()),
        r#"{"claudeAiOauth":{"expiresAt":1000}}"#,
    )
    .unwrap();
    assert_eq!(status(d.path(), d.path()).expired, Some(true));
}

#[test]
fn the_account_email_comes_from_claude_json() {
    let d = TempDir::new().unwrap();
    fs::write(
        d.path().join(".claude.json"),
        r#"{"oauthAccount":{"emailAddress":"me@example.com","organizationUuid":"org-1"}}"#,
    )
    .unwrap();
    let (email, org) = account_from_claude_json(d.path(), d.path());
    assert_eq!(email.as_deref(), Some("me@example.com"));
    assert_eq!(org.as_deref(), Some("org-1"));
}

#[test]
fn a_missing_or_unparseable_claude_json_yields_no_account() {
    let d = TempDir::new().unwrap();
    assert_eq!(account_from_claude_json(d.path(), d.path()), (None, None));
    fs::write(d.path().join(".claude.json"), "not json at all").unwrap();
    assert_eq!(account_from_claude_json(d.path(), d.path()), (None, None));
}

#[test]
fn status_reports_the_account_alongside_the_source() {
    set_keychain_lookup(|_, _| true);
    let d = TempDir::new().unwrap();
    fs::write(
        d.path().join(".claude.json"),
        r#"{"oauthAccount":{"emailAddress":"me@example.com"}}"#,
    )
    .unwrap();
    assert_eq!(status(d.path(), d.path()).account.as_deref(), Some("me@example.com"));
}

#[test]
fn service_names_match_claude_codes_own_keychain_entries() {
    // The derivation was verified against live Keychain entries on a machine
    // running several profiles: every `Claude Code-credentials-<suffix>` it
    // held matched the digest of the config directory using it. These pinned
    // values keep that result as a regression test. If Claude Code ever
    // changes how it derives the service name, this is where it shows up —
    // profiles would silently stop finding their own logins.
    for (dir, expected) in [
        ("/Users/example/.claude-work", "dd1118a7"),
        ("/Users/example/.claude-personal", "5034c31c"),
        ("/Users/example/.claude-client", "daabbf05"),
    ] {
        assert_eq!(
            keychain_service(Path::new(dir)),
            format!("{KEYCHAIN_SERVICE_BASE}-{expected}"),
            "config dir {dir}"
        );
    }
}

#[test]
fn the_default_config_directory_uses_the_unsuffixed_service() {
    // Claude Code stores the default session under the bare service name.
    // Verified on a real machine: the bare `Claude Code-credentials` existed
    // while the digest of that machine's ~/.claude did not. Deriving a suffix
    // here would report the default session — usually the busiest account —
    // as signed out.
    let home = Path::new("/Users/example");
    assert_eq!(
        keychain_service_for(&home.join(".claude"), home),
        KEYCHAIN_SERVICE_BASE
    );
}

#[test]
fn any_other_directory_still_gets_a_digest_suffix() {
    let home = Path::new("/Users/example");
    assert_eq!(
        keychain_service_for(&home.join(".claude-work"), home),
        format!("{KEYCHAIN_SERVICE_BASE}-dd1118a7")
    );
}

#[test]
fn a_directory_merely_named_claude_elsewhere_is_not_the_default() {
    let home = Path::new("/Users/example");
    let elsewhere = Path::new("/opt/.claude");
    assert_ne!(
        keychain_service_for(elsewhere, home),
        KEYCHAIN_SERVICE_BASE,
        "only the default directory under this home is the default session"
    );
}

#[test]
fn the_default_session_account_is_read_from_the_sibling_claude_json() {
    // Verified on a real machine: for the default config directory the
    // account lives in ~/.claude.json, while ~/.claude/.claude.json exists
    // but carries no oauthAccount. A custom directory keeps it inside.
    let home = TempDir::new().unwrap();
    let default_dir = home.path().join(".claude");
    fs::create_dir_all(&default_dir).unwrap();
    fs::write(default_dir.join(".claude.json"), r#"{"tips":1}"#).unwrap();
    fs::write(
        home.path().join(".claude.json"),
        r#"{"oauthAccount":{"emailAddress":"default@example.com"}}"#,
    )
    .unwrap();

    let (account, _) = account_from_claude_json(&default_dir, home.path());
    assert_eq!(account.as_deref(), Some("default@example.com"));
}

#[test]
fn a_custom_directory_keeps_its_account_inside_itself() {
    let home = TempDir::new().unwrap();
    let dir = home.path().join(".claude-hd");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(".claude.json"),
        r#"{"oauthAccount":{"emailAddress":"hd@example.com"}}"#,
    )
    .unwrap();
    // A sibling file must not be preferred for a custom directory.
    fs::write(
        home.path().join(".claude.json"),
        r#"{"oauthAccount":{"emailAddress":"default@example.com"}}"#,
    )
    .unwrap();

    let (account, _) = account_from_claude_json(&dir, home.path());
    assert_eq!(account.as_deref(), Some("hd@example.com"));
}

// --- Codex ---

/// A JWT whose payload is `claims`; the signature is irrelevant here.
fn fake_jwt(claims: &str) -> String {
    use base64::Engine;
    let b64 = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
    format!("{}.{}.{}", b64(r#"{"alg":"RS256"}"#), b64(claims), b64("sig"))
}

fn codex_home(auth: &str) -> TempDir {
    let d = TempDir::new().unwrap();
    fs::write(codex_auth_file(d.path()), auth).unwrap();
    d
}

#[test]
fn a_codex_home_without_auth_json_is_not_authenticated() {
    let d = TempDir::new().unwrap();
    let status = codex_status(d.path());
    assert!(!status.authenticated);
    assert_eq!(status.source, CredentialSource::None);
}

#[test]
fn codex_oauth_tokens_name_the_account_plan_and_organisation() {
    let id = fake_jwt(
        r#"{"email":"me@example.com","https://api.openai.com/auth":{"chatgpt_plan_type":"plus","organizations":[{"id":"org-1","is_default":false,"title":"Side"},{"id":"org-2","is_default":true,"title":"Main"}]}}"#,
    );
    let access = fake_jwt(r#"{"exp":99999999999}"#);
    let d = codex_home(&format!(
        r#"{{"auth_mode":"chatgpt","tokens":{{"access_token":"{access}","id_token":"{id}","account_id":"acc"}}}}"#
    ));
    let status = codex_status(d.path());
    assert!(status.authenticated);
    assert_eq!(status.source, CredentialSource::File);
    assert_eq!(status.account.as_deref(), Some("me@example.com"));
    assert_eq!(status.plan.as_deref(), Some("plus"));
    assert_eq!(status.organization.as_deref(), Some("Main"));
    assert_eq!(status.expired, Some(false));
}

#[test]
fn an_expired_codex_access_token_is_reported_but_still_a_login() {
    let access = fake_jwt(r#"{"exp":1000}"#);
    let d = codex_home(&format!(r#"{{"tokens":{{"access_token":"{access}"}}}}"#));
    let status = codex_status(d.path());
    assert!(status.authenticated, "an expired token is still a login the CLI can refresh");
    assert_eq!(status.expired, Some(true));
    assert_eq!(status.account, None);
}

#[test]
fn a_codex_api_key_counts_as_a_login_to_nobody() {
    let d = codex_home(r#"{"OPENAI_API_KEY":"sk-test","tokens":null}"#);
    let status = codex_status(d.path());
    assert!(status.authenticated);
    assert_eq!(status.source, CredentialSource::ApiKey);
    assert_eq!(status.account, None);
}

#[test]
fn an_unparseable_auth_json_is_not_a_login() {
    let d = codex_home("not json");
    assert!(!codex_status(d.path()).authenticated);
}

#[test]
fn jwt_claims_are_decoded_without_verification() {
    let token = fake_jwt(r#"{"email":"a@b.c","exp":42}"#);
    let claims = jwt_claims(&token).unwrap();
    assert_eq!(claims["email"], "a@b.c");
    assert_eq!(claims["exp"], 42);
    assert!(jwt_claims("not.a-jwt").is_none());
    assert!(jwt_claims("").is_none());
}

#[test]
fn status_for_dispatches_on_the_provider() {
    use cpx_core::config::Provider;
    set_keychain_lookup(|_, _| false);
    let d = codex_home(r#"{"OPENAI_API_KEY":"sk-test"}"#);
    assert!(status_for(Provider::Codex, d.path(), d.path()).authenticated);
    assert!(!status_for(Provider::Claude, d.path(), d.path()).authenticated);
}

#[test]
fn the_credential_location_names_where_each_tool_keeps_its_login() {
    use cpx_core::config::Provider;
    let home = Path::new("/Users/example");
    assert!(location(Provider::Claude, &home.join(".claude-work"), home).contains(KEYCHAIN_SERVICE_BASE));
    assert!(location(Provider::Codex, &home.join(".codex-work"), home).ends_with(".codex-work/auth.json"));
}
