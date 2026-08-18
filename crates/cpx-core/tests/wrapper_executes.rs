//! The wrapper is generated shell, so asserting on its text proves little.
//! These tests run it for real against a fake `claude` that reports the
//! arguments and environment it received.

use cpx_core::config::Config;
use cpx_core::wrapper::{wrapper_script, WrapperContext};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Harness {
    _dir: tempfile::TempDir,
    wrapper: PathBuf,
    bin_dir: PathBuf,
}

/// Build a runnable wrapper for `toml`'s single profile, plus a fake claude
/// that prints `argv` and the CLAUDE_* environment it was handed.
fn harness(toml: &str, name: &str) -> Harness {
    let dir = tempfile::TempDir::new().unwrap();
    let home = dir.path();
    let cfg = Config::parse(toml, home).unwrap();

    let bin_dir = home.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let fake_claude = bin_dir.join("claude");
    fs::write(
        &fake_claude,
        "#!/usr/bin/env bash\nfor a in \"$@\"; do echo \"arg:$a\"; done\n\
         echo \"env:CLAUDE_CONFIG_DIR=${CLAUDE_CONFIG_DIR:-}\"\n\
         echo \"env:CLAUDE_PROFILE=${CLAUDE_PROFILE:-}\"\n\
         echo \"env:LEAKED=${CLAUDE_LEAKED:-none}\"\n\
         echo \"env:CUSTOM=${MY_CUSTOM:-none}\"\n",
    )
    .unwrap();
    fs::set_permissions(&fake_claude, fs::Permissions::from_mode(0o755)).unwrap();

    let profile_dir = home.join(".claude-profiles").join(name);
    fs::create_dir_all(&profile_dir).unwrap();

    let script = wrapper_script(&WrapperContext {
        name,
        profile: &cfg.profiles[name],
        profile_dir: &profile_dir,
        claude_binary: &fake_claude,
    });
    let wrapper = bin_dir.join(format!("claude-{name}"));
    fs::write(&wrapper, script).unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();

    Harness {
        _dir: dir,
        wrapper,
        bin_dir,
    }
}

fn run(h: &Harness, args: &[&str], extra_env: &[(&str, &str)]) -> String {
    let mut cmd = Command::new(&h.wrapper);
    cmd.args(args);
    // Put bin_dir first on PATH: a wrapper that exec'd a bare `claude` would
    // find itself here and recurse until the process table gives out.
    cmd.env(
        "PATH",
        format!(
            "{}:{}",
            h.bin_dir.display(),
            std::env::var("PATH").unwrap_or_default()
        ),
    );
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("wrapper should execute");
    assert!(
        out.status.success(),
        "wrapper failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn is_valid_bash(path: &Path) -> bool {
    Command::new("bash")
        .arg("-n")
        .arg(path)
        .status()
        .expect("bash should be available")
        .success()
}

const PLAIN: &str = "version = 1\n[profiles.work]\n";

#[test]
fn generated_wrapper_is_syntactically_valid_bash() {
    assert!(is_valid_bash(&harness(PLAIN, "work").wrapper));
}

#[test]
fn generated_wrapper_with_every_feature_is_valid_bash() {
    let h = harness(
        "version = 1\n[profiles.work]\nmodel = \"sonnet\"\nadd_dirs = [\"/srv/a\", \"/srv/b\"]\nenv = { A = \"1\", B = \"two words\" }\n",
        "work",
    );
    assert!(is_valid_bash(&h.wrapper));
}

#[test]
fn wrapper_hands_the_profile_config_dir_to_claude() {
    let out = run(&harness(PLAIN, "work"), &[], &[]);
    assert!(
        out.contains(".claude-profiles/work"),
        "config dir not exported: {out}"
    );
    assert!(out.contains("env:CLAUDE_PROFILE=work"), "{out}");
}

#[test]
fn wrapper_does_not_recurse_when_bin_dir_shadows_the_real_claude() {
    // Regression guard for cpm's `exec claude "$@"`: with bin_dir ahead of
    // the real binary on PATH, that resolves back to the wrapper itself.
    let out = run(&harness(PLAIN, "work"), &["hello"], &[]);
    assert_eq!(out.matches("arg:hello").count(), 1, "{out}");
}

#[test]
fn wrapper_drops_an_inherited_claude_variable() {
    let out = run(&harness(PLAIN, "work"), &[], &[("CLAUDE_LEAKED", "yes")]);
    assert!(out.contains("env:LEAKED=none"), "inherited var survived: {out}");
}

#[test]
fn wrapper_exports_profile_env_vars() {
    let h = harness(
        "version = 1\n[profiles.work]\nenv = { MY_CUSTOM = \"set\" }\n",
        "work",
    );
    assert!(run(&h, &[], &[]).contains("env:CUSTOM=set"));
}

#[test]
fn wrapper_forwards_user_arguments_after_its_own_flags() {
    let h = harness(
        "version = 1\n[profiles.work]\nadd_dirs = [\"/srv/a\"]\n",
        "work",
    );
    let out = run(&h, &["-p", "explain"], &[]);
    assert!(out.contains("arg:--add-dir"), "{out}");
    assert!(out.contains("arg:/srv/a"), "{out}");
    assert!(out.contains("arg:-p"), "{out}");
    assert!(out.contains("arg:explain"), "{out}");
}

#[test]
fn configured_model_is_applied_when_the_user_passes_none() {
    let h = harness("version = 1\n[profiles.work]\nmodel = \"sonnet\"\n", "work");
    let out = run(&h, &[], &[]);
    assert!(out.contains("arg:--model"), "{out}");
    assert!(out.contains("arg:sonnet"), "{out}");
}

#[test]
fn a_command_line_model_suppresses_the_profile_default() {
    let h = harness("version = 1\n[profiles.work]\nmodel = \"sonnet\"\n", "work");
    let out = run(&h, &["--model", "opus"], &[]);
    assert!(out.contains("arg:opus"), "{out}");
    assert!(!out.contains("arg:sonnet"), "profile default leaked in: {out}");
    assert_eq!(out.matches("arg:--model").count(), 1, "{out}");
}

#[test]
fn a_joined_model_flag_also_suppresses_the_default() {
    let h = harness("version = 1\n[profiles.work]\nmodel = \"sonnet\"\n", "work");
    let out = run(&h, &["--model=opus"], &[]);
    assert!(!out.contains("arg:sonnet"), "{out}");
}

#[test]
fn installation_subcommands_bypass_session_flags() {
    let h = harness(
        "version = 1\n[profiles.work]\nmodel = \"sonnet\"\nadd_dirs = [\"/srv/a\"]\n",
        "work",
    );
    let out = run(&h, &["mcp", "list"], &[]);
    assert!(out.contains("arg:mcp"), "{out}");
    assert!(!out.contains("arg:--add-dir"), "add-dir leaked into mcp: {out}");
    assert!(!out.contains("arg:--model"), "model leaked into mcp: {out}");
}

#[test]
fn a_hostile_env_value_is_not_executed() {
    let h = harness(
        "version = 1\n[profiles.work]\nenv = { MY_CUSTOM = \"$(echo pwned)\" }\n",
        "work",
    );
    let out = run(&h, &[], &[]);
    assert!(
        out.contains("env:CUSTOM=$(echo pwned)"),
        "command substitution was evaluated: {out}"
    );
}

#[test]
fn a_hostile_add_dir_is_not_executed() {
    let h = harness(
        "version = 1\n[profiles.work]\nadd_dirs = [\"/srv/$(echo pwned)\"]\n",
        "work",
    );
    let out = run(&h, &[], &[]);
    assert!(out.contains("arg:/srv/$(echo pwned)"), "{out}");
}
