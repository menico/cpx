//! Reporting on the directory a plain `claude` uses.

use cpx_core::config::Config;
use cpx_core::credentials::{set_keychain_lookup, CredentialSource};
use cpx_core::default_session::default_session;
use cpx_core::layout::Layout;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn env() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".claude")).unwrap();
    fs::write(
        dir.path().join(".claude/.claude.json"),
        r#"{"oauthAccount":{"emailAddress":"default@example.com"}}"#,
    )
    .unwrap();
    dir
}

fn config(text: &str, home: &Path) -> Config {
    Config::parse(text, home).unwrap()
}

#[test]
fn it_reports_the_home_claude_directory() {
    let d = env();
    set_keychain_lookup(|_, _| false);
    let session = default_session(&Layout::new(d.path()), &config("version = 1\n", d.path()));
    assert_eq!(session.dir, d.path().join(".claude"));
}

#[test]
fn it_reads_the_account_that_directory_is_signed_into() {
    let d = env();
    set_keychain_lookup(|_, _| false);
    let session = default_session(&Layout::new(d.path()), &config("version = 1\n", d.path()));
    assert_eq!(session.account.as_deref(), Some("default@example.com"));
}

#[test]
fn it_uses_the_unsuffixed_keychain_service() {
    let d = env();
    // Only the bare service exists for the default directory; a lookup for a
    // digest-suffixed one would report this account as signed out.
    set_keychain_lookup(|service, _| service == "Claude Code-credentials");
    let session = default_session(&Layout::new(d.path()), &config("version = 1\n", d.path()));
    assert!(session.signed_in, "the default session should read as signed in");
    assert_eq!(session.credential_source, CredentialSource::Keychain);
}

#[test]
fn it_knows_when_it_is_also_the_source_directory() {
    let d = env();
    set_keychain_lookup(|_, _| false);
    let session = default_session(&Layout::new(d.path()), &config("version = 1\n", d.path()));
    assert!(session.is_source, "source_dir defaults to ~/.claude");
}

#[test]
fn a_source_directory_pointed_elsewhere_is_not_this_one() {
    let d = env();
    set_keychain_lookup(|_, _| false);
    let cfg = config("version = 1\nsource_dir = \"~/dotfiles/claude\"\n", d.path());
    assert!(!default_session(&Layout::new(d.path()), &cfg).is_source);
}

#[test]
fn it_reports_which_profile_has_claimed_it() {
    let d = env();
    set_keychain_lookup(|_, _| false);
    let cfg = config(
        "version = 1\n[profiles.default]\ndir = \"~/.claude\"\n",
        d.path(),
    );
    assert_eq!(
        default_session(&Layout::new(d.path()), &cfg).claimed_by.as_deref(),
        Some("default")
    );
}

#[test]
fn it_is_unclaimed_when_no_profile_points_at_it() {
    let d = env();
    set_keychain_lookup(|_, _| false);
    let cfg = config("version = 1\n[profiles.work]\n", d.path());
    assert_eq!(default_session(&Layout::new(d.path()), &cfg).claimed_by, None);
}

#[test]
fn a_missing_default_directory_reports_no_account() {
    let d = TempDir::new().unwrap();
    set_keychain_lookup(|_, _| false);
    let session = default_session(&Layout::new(d.path()), &config("version = 1\n", d.path()));
    assert!(!session.signed_in);
    assert_eq!(session.account, None);
}

// --- both tools ---

use cpx_core::config::Provider;
use cpx_core::default_session::default_sessions;

#[test]
fn only_claude_is_reported_when_there_is_no_codex_at_all() {
    let d = env();
    set_keychain_lookup(|_, _| false);
    let sessions = default_sessions(&Layout::new(d.path()), &config("version = 1\n", d.path()));
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].provider, Provider::Claude);
}

#[test]
fn an_existing_codex_home_is_reported_with_its_account() {
    let d = env();
    set_keychain_lookup(|_, _| false);
    fs::create_dir_all(d.path().join(".codex")).unwrap();
    fs::write(d.path().join(".codex/auth.json"), r#"{"OPENAI_API_KEY":"sk"}"#).unwrap();
    let sessions = default_sessions(&Layout::new(d.path()), &config("version = 1\n", d.path()));
    assert_eq!(sessions.len(), 2);
    let codex = &sessions[1];
    assert_eq!(codex.provider, Provider::Codex);
    assert_eq!(codex.dir, d.path().join(".codex"));
    assert!(codex.signed_in);
    assert!(codex.is_source);
}

#[test]
fn a_codex_profile_makes_the_codex_home_worth_reporting_even_before_it_exists() {
    let d = env();
    set_keychain_lookup(|_, _| false);
    let cfg = config("version = 1\n[profiles.gpt]\nprovider = \"codex\"\n", d.path());
    let sessions = default_sessions(&Layout::new(d.path()), &cfg);
    assert_eq!(sessions.len(), 2);
    assert!(!sessions[1].signed_in);
}

#[test]
fn a_profile_adopting_the_codex_home_claims_it() {
    let d = env();
    set_keychain_lookup(|_, _| false);
    fs::create_dir_all(d.path().join(".codex")).unwrap();
    let cfg = config(
        "version = 1\n[profiles.base]\nprovider = \"codex\"\ndir = \"~/.codex\"\n",
        d.path(),
    );
    let sessions = default_sessions(&Layout::new(d.path()), &cfg);
    assert_eq!(sessions[1].claimed_by.as_deref(), Some("base"));
}
