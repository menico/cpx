//! Diagnostics: each check is a failure someone will actually hit.

use cpx_core::binding::Bindings;
use cpx_core::config::Config;
use cpx_core::doctor::*;
use cpx_core::execute::{execute, ExecuteOptions};
use cpx_core::layout::Layout;
use cpx_core::materialize::{plan_apply, ApplyOptions};
use cpx_core::state::{State, MARKER};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

struct Env {
    dir: TempDir,
    layout: Layout,
    state: State,
}

fn env() -> Env {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join(".claude");
    for sub in ["commands", "skills", "agents", "plugins", "hooks"] {
        fs::create_dir_all(source.join(sub)).unwrap();
    }
    fs::write(source.join("settings.json"), "{}").unwrap();
    fs::write(source.join("CLAUDE.md"), "# base\n").unwrap();
    let layout = Layout::new(dir.path());
    Env {
        layout,
        state: State::default(),
        dir,
    }
}

impl Env {
    fn config(&self) -> Config {
        Config::parse("version = 1\n[profiles.work]\n", self.dir.path()).unwrap()
    }

    fn applied(&mut self) {
        let config = self.config();
        let plan = plan_apply(&config, &self.layout, &self.state, &ApplyOptions::default()).unwrap();
        let source = self.dir.path().join(".claude");
        execute(&plan, &mut self.state, &source, &ExecuteOptions::default()).unwrap();
    }

    fn ambient(&self) -> Ambient {
        Ambient {
            path: self.dir.path().join(".local/bin").display().to_string(),
            claude_config_dir: None,
            direnv_present: true,
            claude_binary: Some(PathBuf::from("/usr/local/bin/claude")),
        }
    }

    fn run(&self, ambient: &Ambient) -> Vec<Check> {
        diagnose(
            &self.config(),
            &self.layout,
            &self.state,
            &Bindings::default(),
            ambient,
        )
    }
}

fn find<'a>(checks: &'a [Check], needle: &str) -> Option<&'a Check> {
    checks.iter().find(|c| c.name.contains(needle))
}

fn severity(checks: &[Check], needle: &str) -> Severity {
    find(checks, needle)
        .unwrap_or_else(|| panic!("no check named {needle} in {checks:#?}"))
        .severity
}

#[test]
fn a_healthy_installation_reports_no_problems() {
    let mut e = env();
    e.applied();
    cpx_core::credentials::set_keychain_lookup(|_, _| true);
    let checks = e.run(&e.ambient());
    assert_eq!(
        worst(&checks),
        Severity::Ok,
        "unexpected findings: {:#?}",
        checks
            .iter()
            .filter(|c| c.severity != Severity::Ok)
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_missing_source_directory_is_an_error() {
    let mut e = env();
    e.applied();
    fs::remove_dir_all(e.dir.path().join(".claude")).unwrap();
    assert_eq!(severity(&e.run(&e.ambient()), "source"), Severity::Error);
}

#[test]
fn an_unapplied_profile_is_reported() {
    let e = env();
    assert_eq!(severity(&e.run(&e.ambient()), "work"), Severity::Error);
}

#[test]
fn a_broken_symlink_is_reported() {
    let mut e = env();
    e.applied();
    fs::remove_dir_all(e.dir.path().join(".claude/commands")).unwrap();
    let checks = e.run(&e.ambient());
    assert_eq!(worst(&checks), Severity::Error, "{checks:#?}");
    assert!(
        checks
            .iter()
            .any(|c| c.detail.contains("commands") && c.severity == Severity::Error),
        "{checks:#?}"
    );
}

#[test]
fn a_bin_dir_missing_from_path_is_a_warning() {
    let mut e = env();
    e.applied();
    let ambient = Ambient {
        path: "/usr/bin:/bin".to_string(),
        ..e.ambient()
    };
    assert_eq!(severity(&e.run(&ambient), "PATH"), Severity::Warning);
}

#[test]
fn a_config_dir_leaking_from_the_environment_is_a_warning() {
    let mut e = env();
    e.applied();
    let ambient = Ambient {
        claude_config_dir: Some("/somewhere/else".to_string()),
        ..e.ambient()
    };
    let checks = e.run(&ambient);
    let check = find(&checks, "CLAUDE_CONFIG_DIR").expect("check should exist");
    assert_eq!(check.severity, Severity::Warning);
    assert!(
        check.detail.contains("/somewhere/else"),
        "the user needs to see which directory: {check:?}"
    );
}

#[test]
fn a_missing_direnv_is_a_warning_because_bindings_need_it() {
    let mut e = env();
    e.applied();
    let ambient = Ambient {
        direnv_present: false,
        ..e.ambient()
    };
    assert_eq!(severity(&e.run(&ambient), "direnv"), Severity::Warning);
}

#[test]
fn a_missing_claude_binary_is_an_error() {
    let mut e = env();
    e.applied();
    let ambient = Ambient {
        claude_binary: None,
        ..e.ambient()
    };
    assert_eq!(severity(&e.run(&ambient), "Claude binary"), Severity::Error);
}

#[test]
fn a_foreign_claude_wrapper_is_reported_without_alarm() {
    let mut e = env();
    e.applied();
    let bin = e.dir.path().join(".local/bin");
    fs::write(bin.join("claude-company"), "#!/bin/sh\nexec other\n").unwrap();

    let checks = e.run(&e.ambient());
    let check = find(&checks, "claude-company").expect("should be reported");
    assert_eq!(
        check.severity,
        Severity::Warning,
        "someone else's tool is worth mentioning, not an error"
    );
}

#[test]
fn a_wrapper_that_was_deleted_is_an_error() {
    let mut e = env();
    e.applied();
    fs::remove_file(e.dir.path().join(".local/bin/claude-work")).unwrap();
    let checks = e.run(&e.ambient());
    assert!(
        checks
            .iter()
            .any(|c| c.severity == Severity::Error && c.detail.contains("claude-work")),
        "{checks:#?}"
    );
}

#[test]
fn an_unauthenticated_profile_is_a_warning_not_an_error() {
    let mut e = env();
    e.applied();
    cpx_core::credentials::set_keychain_lookup(|_, _| false);
    let checks = e.run(&e.ambient());
    let auth = find(&checks, "login").expect("an auth check should exist");
    assert_eq!(
        auth.severity,
        Severity::Warning,
        "not being logged in yet is normal on a fresh profile"
    );
    assert!(auth.remedy.is_some(), "tell the user how to log in");
}

#[test]
fn a_binding_pointing_at_a_deleted_directory_is_reported() {
    let mut e = env();
    e.applied();
    let mut bindings = Bindings::default();
    bindings.upsert(cpx_core::binding::Binding {
        path: PathBuf::from("/definitely/not/here"),
        profile: "work".into(),
        block_sha256: "x".into(),
    });

    let checks = diagnose(
        &e.config(),
        &e.layout,
        &e.state,
        &bindings,
        &e.ambient(),
    );
    assert!(
        checks
            .iter()
            .any(|c| c.detail.contains("/definitely/not/here") && c.severity != Severity::Ok),
        "{checks:#?}"
    );
}

#[test]
fn every_check_that_fails_says_what_to_do_about_it() {
    let mut e = env();
    e.applied();
    cpx_core::credentials::set_keychain_lookup(|_, _| false);
    fs::remove_file(e.dir.path().join(".local/bin/claude-work")).unwrap();
    fs::remove_dir_all(e.dir.path().join(".claude/commands")).unwrap();

    let ambient = Ambient {
        path: "/usr/bin".to_string(),
        claude_config_dir: Some("/elsewhere".to_string()),
        direnv_present: false,
        claude_binary: None,
    };
    for check in e.run(&ambient).iter().filter(|c| c.severity != Severity::Ok) {
        assert!(
            check.remedy.is_some(),
            "a finding with no remedy leaves the user stuck: {check:?}"
        );
    }
}

#[test]
fn a_stale_wrapper_marker_is_recognised_as_ours() {
    let mut e = env();
    e.applied();
    let bin = e.dir.path().join(".local/bin");
    fs::write(
        bin.join("claude-old"),
        format!("#!/usr/bin/env bash\n{MARKER}\n"),
    )
    .unwrap();

    let checks = e.run(&e.ambient());
    let check = find(&checks, "claude-old").expect("should be reported");
    assert!(
        check.remedy.as_deref().unwrap_or_default().contains("apply"),
        "cpx apply cleans these up: {check:?}"
    );
}

#[test]
fn checks_are_ordered_worst_first_so_the_important_thing_is_visible() {
    let mut e = env();
    e.applied();
    fs::remove_file(e.dir.path().join(".local/bin/claude-work")).unwrap();
    let ambient = Ambient {
        direnv_present: false,
        ..e.ambient()
    };
    let checks = e.run(&ambient);
    let severities: Vec<_> = checks.iter().map(|c| c.severity).collect();
    let mut sorted = severities.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(severities, sorted, "{checks:#?}");
}

#[test]
fn the_login_check_goes_through_the_injectable_keychain_lookup() {
    // If a check shelled out to `security` directly it would ignore this
    // stub, and the profile would still read as logged out.
    let mut e = env();
    e.applied();
    cpx_core::credentials::set_keychain_lookup(|_, _| true);
    let checks = e.run(&e.ambient());
    assert_eq!(severity(&checks, "login"), Severity::Ok, "{checks:#?}");
}
