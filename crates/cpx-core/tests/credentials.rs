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
    // Verified against live Keychain entries: the services
    // `Claude Code-credentials-{37c92b8a,a81467e4,ea75f7c9}` present on a
    // machine using these config directories. If Claude Code ever changes
    // how it derives the service name, this is where it shows up: profiles
    // would silently stop finding their own logins.
    for (dir, expected) in [
        ("/Users/menikoppenhol/.claude-personal", "37c92b8a"),
        ("/Users/menikoppenhol/.claude-hd", "a81467e4"),
        ("/Users/menikoppenhol/.claude-ol", "ea75f7c9"),
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
    // Verified on a real machine: `Claude Code-credentials` exists, while
    // `Claude Code-credentials-010ed29c` — the digest of ~/.claude — does
    // not. Deriving a suffix here would report the default session, which is
    // usually the busiest account, as signed out.
    let home = Path::new("/Users/menikoppenhol");
    assert_eq!(
        keychain_service_for(&home.join(".claude"), home),
        KEYCHAIN_SERVICE_BASE
    );
}

#[test]
fn any_other_directory_still_gets_a_digest_suffix() {
    let home = Path::new("/Users/menikoppenhol");
    assert_eq!(
        keychain_service_for(&home.join(".claude-hd"), home),
        format!("{KEYCHAIN_SERVICE_BASE}-a81467e4")
    );
}

#[test]
fn a_directory_merely_named_claude_elsewhere_is_not_the_default() {
    let home = Path::new("/Users/menikoppenhol");
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
