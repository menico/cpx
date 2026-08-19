//! The generated wrapper is a shell script whose whole job is to behave
//! correctly when Claude runs it, so these tests run it.

use cpx_core::statusline::*;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::TempDir;

const SESSION: &str = r#"{"session_id":"abc","context_window":{"used":1234}}"#;

fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// Run a generated wrapper the way Claude does: session JSON on stdin.
fn run(script: &Path, stdin: &str) -> String {
    let mut child = Command::new("bash")
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("wrapper should run");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "wrapper failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn a_hex_colour_becomes_a_truecolor_escape() {
    assert_eq!(ansi_color("#5c8dff").as_deref(), Some("\x1b[38;2;92;141;255m"));
    assert_eq!(ansi_color("#000000").as_deref(), Some("\x1b[38;2;0;0;0m"));
}

#[test]
fn a_malformed_colour_is_refused_rather_than_rendered_wrong() {
    for bad in ["5c8dff", "#abc", "#gggggg", "", "#5c8dff "] {
        assert_eq!(ansi_color(bad), None, "accepted {bad:?}");
    }
}

#[test]
fn the_generated_script_is_valid_bash() {
    let d = TempDir::new().unwrap();
    let script = write_script(
        d.path(),
        "sl.sh",
        &render_wrapper(&Badge::new("work", Some("#5c8dff".into())), None),
    );
    assert!(Command::new("bash")
        .arg("-n")
        .arg(&script)
        .status()
        .unwrap()
        .success());
}

#[test]
fn without_a_delegate_it_prints_just_the_badge() {
    let d = TempDir::new().unwrap();
    let script = write_script(
        d.path(),
        "sl.sh",
        &render_wrapper(&Badge::new("work", None), None),
    );
    let out = run(&script, SESSION);
    assert!(out.contains("work"), "{out:?}");
    // A trailing newline is normal for a command; what matters is that the
    // line is not split into several.
    assert!(
        !out.trim_end().contains('\n'),
        "a statusline is one line: {out:?}"
    );
}

#[test]
fn the_badge_carries_the_profile_colour() {
    let d = TempDir::new().unwrap();
    let script = write_script(
        d.path(),
        "sl.sh",
        &render_wrapper(&Badge::new("work", Some("#5c8dff".into())), None),
    );
    let out = run(&script, SESSION);
    assert!(out.contains("\x1b[38;2;92;141;255m"), "{out:?}");
    assert!(out.contains("\x1b[0m"), "colour must be reset: {out:?}");
}

#[test]
fn the_delegated_statusline_output_follows_the_badge() {
    let d = TempDir::new().unwrap();
    let inner = write_script(d.path(), "inner.sh", "#!/bin/sh\nprintf 'INNER-OUTPUT'\n");
    let script = write_script(
        d.path(),
        "sl.sh",
        &render_wrapper(
            &Badge::new("work", None),
            Some(&Delegate {
                command: format!("bash {}", inner.display()),
            }),
        ),
    );
    let out = run(&script, SESSION);
    assert!(out.contains("work"), "{out:?}");
    assert!(out.contains("INNER-OUTPUT"), "{out:?}");
    assert!(
        out.find("work").unwrap() < out.find("INNER-OUTPUT").unwrap(),
        "badge should come first: {out:?}"
    );
}

#[test]
fn the_session_json_reaches_the_delegate_unchanged() {
    // The delegate reads the session from stdin; a wrapper that swallowed it
    // would silently break every statusline that shows context or cost.
    let d = TempDir::new().unwrap();
    let inner = write_script(d.path(), "inner.sh", "#!/bin/sh\nprintf 'GOT:'; cat\n");
    let script = write_script(
        d.path(),
        "sl.sh",
        &render_wrapper(
            &Badge::new("work", None),
            Some(&Delegate {
                command: format!("bash {}", inner.display()),
            }),
        ),
    );
    let out = run(&script, SESSION);
    assert!(out.contains(&format!("GOT:{SESSION}")), "{out:?}");
}

#[test]
fn a_delegate_that_fails_still_leaves_the_badge_visible() {
    // A broken statusline must not blank the line: the profile is the part
    // worth keeping when something else goes wrong.
    let d = TempDir::new().unwrap();
    let script = write_script(
        d.path(),
        "sl.sh",
        &render_wrapper(
            &Badge::new("work", None),
            Some(&Delegate {
                command: "exit 3".to_string(),
            }),
        ),
    );
    let out = run(&script, SESSION);
    assert!(out.contains("work"), "{out:?}");
}

#[test]
fn a_delegate_that_does_not_exist_does_not_break_the_line() {
    let d = TempDir::new().unwrap();
    let script = write_script(
        d.path(),
        "sl.sh",
        &render_wrapper(
            &Badge::new("work", None),
            Some(&Delegate {
                command: "/nonexistent/statusline".to_string(),
            }),
        ),
    );
    let out = run(&script, SESSION);
    assert!(out.contains("work"), "{out:?}");
}

#[test]
fn multi_line_delegate_output_is_flattened_to_one_line() {
    let d = TempDir::new().unwrap();
    let inner = write_script(d.path(), "inner.sh", "#!/bin/sh\nprintf 'one\\ntwo'\n");
    let script = write_script(
        d.path(),
        "sl.sh",
        &render_wrapper(
            &Badge::new("work", None),
            Some(&Delegate {
                command: format!("bash {}", inner.display()),
            }),
        ),
    );
    let out = run(&script, SESSION);
    assert!(
        !out.trim_end().contains('\n'),
        "a delegate printing two lines must not break the statusline: {out:?}"
    );
    assert!(out.contains("one") && out.contains("two"), "{out:?}");
}

#[test]
fn the_running_profile_overrides_the_name_baked_in() {
    // One wrapper serves every profile that inherits it, so the badge must
    // follow CLAUDE_PROFILE rather than the name it was generated for.
    let d = TempDir::new().unwrap();
    let script = write_script(
        d.path(),
        "sl.sh",
        &render_wrapper(&Badge::new("work", Some("#5c8dff".into())), None),
    );
    let out = Command::new("bash")
        .arg(&script)
        .env("CLAUDE_PROFILE", "personal")
        .env("CPX_PROFILE_COLOR", "#5dc794")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(text.contains("personal"), "{text:?}");
    assert!(!text.contains("work"), "{text:?}");
    assert!(text.contains("38;2;93;199;148"), "colour should follow too: {text:?}");
}

#[test]
fn a_hostile_profile_name_is_not_executed() {
    // The witness lives in this test's own directory: a shared path in /tmp
    // would survive a failing run and fail every run after it.
    let d = TempDir::new().unwrap();
    let witness = d.path().join("pwned");
    let label = format!("$(touch {})", witness.display());
    let script = write_script(d.path(), "sl.sh", &render_wrapper(&Badge::new(&label, None), None));

    let out = run(&script, SESSION);
    assert!(out.contains(&label), "the name should print literally: {out:?}");
    assert!(!witness.exists(), "command substitution ran");
}

#[test]
fn a_hostile_profile_name_from_the_environment_is_not_executed_either() {
    let d = TempDir::new().unwrap();
    let witness = d.path().join("pwned-env");
    let script = write_script(d.path(), "sl.sh", &render_wrapper(&Badge::new("work", None), None));

    let out = Command::new("bash")
        .arg(&script)
        .env("CLAUDE_PROFILE", format!("$(touch {})", witness.display()))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!witness.exists(), "an environment value was executed");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("$(touch"),
        "{:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn the_script_says_it_is_generated_so_nobody_edits_it_by_hand() {
    let script = render_wrapper(&Badge::new("work", None), None);
    assert!(script.contains(cpx_core::state::MARKER), "{script}");
}

// --- planning an install ---

use cpx_core::config::Config;
use cpx_core::layout::Layout;

struct Env {
    dir: TempDir,
}

impl Env {
    /// A home whose base settings already have a statusline, as a real one does.
    fn new() -> Env {
        let dir = TempDir::new().unwrap();
        let source = dir.path().join(".claude");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("settings.json"),
            r#"{"statusLine":{"type":"command","command":"node ~/.claude/statusline.mjs","refreshInterval":30}}"#,
        )
        .unwrap();
        Env { dir }
    }

    fn home(&self) -> &Path {
        self.dir.path()
    }

    fn layout(&self) -> Layout {
        Layout::new(self.home())
    }

    fn config(&self, toml: &str) -> Config {
        Config::parse(toml, self.home()).unwrap()
    }
}

const MERGED: &str = "version = 1\n[profiles.work]\ncolor = \"#5c8dff\"\n";
const OWNED: &str = "version = 1\n[profiles.work]\ncolor = \"#5c8dff\"\n[profiles.work.resources]\nsettings = \"own\"\n";

#[test]
fn a_merged_profile_records_the_statusline_as_a_config_patch() {
    // A direct edit would be reverted by the next apply, which regenerates
    // settings.json from source plus patch.
    let env = Env::new();
    let plan = plan_install(&env.config(MERGED), &env.layout(), &Target::Profile("work".into()), None).unwrap();
    assert_eq!(
        plan.write,
        SettingsWrite::ConfigPatch { profile: "work".into() }
    );
}

#[test]
fn a_profile_owning_its_settings_gets_the_file_edited() {
    let env = Env::new();
    let plan = plan_install(&env.config(OWNED), &env.layout(), &Target::Profile("work".into()), None).unwrap();
    match plan.write {
        SettingsWrite::File { path } => {
            assert!(path.ends_with("work/settings.json"), "{}", path.display())
        }
        other => panic!("expected a file edit, got {other:?}"),
    }
}

#[test]
fn the_wrapper_lives_under_the_cpx_root_not_in_an_adopted_directory() {
    let env = Env::new();
    let adopted = env.home().join(".claude-work");
    fs::create_dir_all(&adopted).unwrap();
    let toml = format!(
        "version = 1\n[profiles.work]\ndir = \"{}\"\n[profiles.work.resources]\nsettings = \"own\"\n",
        adopted.display()
    );
    let plan = plan_install(&env.config(&toml), &env.layout(), &Target::Profile("work".into()), None).unwrap();
    assert!(
        plan.script_path.starts_with(env.home().join(".claude-profiles")),
        "{}",
        plan.script_path.display()
    );
    assert!(!plan.script_path.starts_with(&adopted));
}

#[test]
fn a_first_install_delegates_to_the_statusline_already_configured() {
    let env = Env::new();
    let plan = plan_install(&env.config(MERGED), &env.layout(), &Target::Profile("work".into()), None).unwrap();
    assert_eq!(
        plan.delegate.as_ref().map(|d| d.command.as_str()),
        Some("node ~/.claude/statusline.mjs"),
        "it should wrap what was there, not discard it"
    );
    assert!(!plan.replacing);
}

#[test]
fn the_badge_uses_the_profile_name_and_colour() {
    let env = Env::new();
    let plan = plan_install(&env.config(MERGED), &env.layout(), &Target::Profile("work".into()), None).unwrap();
    assert!(plan.script.contains("work"), "{}", plan.script);
    // The escape is computed at run time from the hex, so the hex is what is
    // baked in.
    assert!(plan.script.contains("#5c8dff"), "{}", plan.script);
}

#[test]
fn a_label_can_be_given_instead_of_the_profile_name() {
    let env = Env::new();
    let plan = plan_install(&env.config(MERGED), &env.layout(), &Target::Profile("work".into()), Some("HD")).unwrap();
    assert!(plan.script.contains("HD"), "{}", plan.script);
}

#[test]
fn installing_over_our_own_wrapper_does_not_nest_it() {
    let env = Env::new();
    let config = env.config(OWNED);
    let layout = env.layout();

    let first = plan_install(&config, &layout, &Target::Profile("work".into()), None).unwrap();
    fs::create_dir_all(first.script_path.parent().unwrap()).unwrap();
    fs::write(&first.script_path, &first.script).unwrap();
    // Point the profile's settings at it, as installing would.
    let settings = layout.profile_dir("work").join("settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(
        &settings,
        format!(
            r#"{{"statusLine":{{"type":"command","command":"{}"}}}}"#,
            command_for(&first.script_path)
        ),
    )
    .unwrap();

    let second = plan_install(&config, &layout, &Target::Profile("work".into()), None).unwrap();
    assert!(second.replacing, "it should notice its own wrapper");
    assert_eq!(
        second.delegate.as_ref().map(|d| d.command.as_str()),
        Some("node ~/.claude/statusline.mjs"),
        "the original statusline must survive a reinstall"
    );
    assert!(
        !second.script.contains("statusline.sh"),
        "the wrapper must not delegate to itself: {}",
        second.script
    );
}

#[test]
fn the_delegate_can_be_read_back_out_of_a_generated_wrapper() {
    let d = TempDir::new().unwrap();
    let script = d.path().join("sl.sh");
    fs::write(
        &script,
        render_wrapper_with_delegate_record(
            &Badge::new("work", None),
            Some(&Delegate { command: "node x.mjs".into() }),
        ),
    )
    .unwrap();
    assert_eq!(
        delegate_of_wrapper(&script).map(|d| d.command),
        Some("node x.mjs".to_string())
    );
}

#[test]
fn a_wrapper_with_no_delegate_reads_back_as_none() {
    let d = TempDir::new().unwrap();
    let script = d.path().join("sl.sh");
    fs::write(&script, render_wrapper_with_delegate_record(&Badge::new("work", None), None)).unwrap();
    assert_eq!(delegate_of_wrapper(&script), None);
}

#[test]
fn the_base_statusline_targets_the_source_settings_file() {
    let env = Env::new();
    let plan = plan_install(&env.config(MERGED), &env.layout(), &Target::Base, None).unwrap();
    match plan.write {
        SettingsWrite::File { path } => {
            assert_eq!(path, env.home().join(".claude/settings.json"))
        }
        other => panic!("expected a file edit, got {other:?}"),
    }
    assert_eq!(
        plan.delegate.as_ref().map(|d| d.command.as_str()),
        Some("node ~/.claude/statusline.mjs")
    );
}

#[test]
fn the_base_wrapper_is_kept_out_of_the_source_directory() {
    let env = Env::new();
    let plan = plan_install(&env.config(MERGED), &env.layout(), &Target::Base, None).unwrap();
    assert!(
        !plan.script_path.starts_with(env.home().join(".claude/")),
        "cpx must not put scripts inside the source directory: {}",
        plan.script_path.display()
    );
}

#[test]
fn an_unknown_profile_is_refused() {
    let env = Env::new();
    assert!(matches!(
        plan_install(&env.config(MERGED), &env.layout(), &Target::Profile("nope".into()), None),
        Err(StatusLineError::UnknownProfile(_))
    ));
}

#[test]
fn the_generated_wrapper_from_a_plan_actually_runs() {
    let env = Env::new();
    let d = TempDir::new().unwrap();
    let inner = write_script(d.path(), "inner.sh", "#!/bin/sh\nprintf 'REAL'\n");
    let toml = format!(
        "version = 1\n[profiles.work]\ncolor = \"#5dc794\"\n[profiles.work.resources]\nsettings = \"own\"\n"
    );
    let config = env.config(&toml);
    let layout = env.layout();

    // Give the profile an existing statusline to wrap.
    let settings = layout.profile_dir("work").join("settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(
        &settings,
        format!(r#"{{"statusLine":{{"command":"bash {}"}}}}"#, inner.display()),
    )
    .unwrap();

    let plan = plan_install(&config, &layout, &Target::Profile("work".into()), None).unwrap();
    let script = write_script(d.path(), "generated.sh", &plan.script);
    let out = run(&script, SESSION);
    assert!(out.contains("work"), "{out:?}");
    assert!(out.contains("REAL"), "{out:?}");
}

// --- installing and removing for real ---

fn settings_of(path: &Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn installing_on_an_owned_profile_writes_the_script_and_points_settings_at_it() {
    let env = Env::new();
    let config = env.config(OWNED);
    let layout = env.layout();
    let settings = layout.profile_dir("work").join("settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(&settings, r#"{"theme":"dark","statusLine":{"command":"old"}}"#).unwrap();

    let plan = plan_install(&config, &layout, &Target::Profile("work".into()), None).unwrap();
    let applied = install(&plan, OWNED, Some(30)).unwrap();

    assert!(plan.script_path.is_file(), "the wrapper should exist");
    let written = settings_of(&settings);
    assert!(
        written["statusLine"]["command"].as_str().unwrap().contains("statusline.sh"),
        "{written}"
    );
    assert_eq!(written["statusLine"]["refreshInterval"], 30);
    assert_eq!(written["theme"], "dark", "other settings must survive");
    assert!(applied.backup.is_some(), "the previous file should be kept");
}

#[test]
fn installing_is_executable_and_runs() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new();
    let config = env.config(OWNED);
    let layout = env.layout();
    let plan = plan_install(&config, &layout, &Target::Profile("work".into()), None).unwrap();
    install(&plan, OWNED, None).unwrap();

    let mode = fs::metadata(&plan.script_path).unwrap().permissions().mode() & 0o111;
    assert_ne!(mode, 0, "the wrapper must be executable");
    assert!(run(&plan.script_path, SESSION).contains("work"));
}

#[test]
fn installing_on_a_merged_profile_edits_the_config_not_the_file() {
    let env = Env::new();
    let config = env.config(MERGED);
    let layout = env.layout();
    let plan = plan_install(&config, &layout, &Target::Profile("work".into()), None).unwrap();
    let applied = install(&plan, MERGED, None).unwrap();

    assert!(applied.settings_path.is_none(), "no file should be touched");
    let text = applied.config_text.expect("config should be rewritten");
    assert!(text.contains("statusLine"), "{text}");
    // And the rewritten config still parses, with the patch in place.
    let reparsed = Config::parse(&text, env.home()).unwrap();
    let patch = reparsed.profiles["work"].resources[&cpx_core::config::ResourceKey::Settings]
        .patch
        .clone()
        .unwrap();
    assert!(patch["statusLine"]["command"].as_str().unwrap().contains("statusline.sh"));
}

#[test]
fn removing_restores_the_statusline_that_was_there_before() {
    let env = Env::new();
    let config = env.config(OWNED);
    let layout = env.layout();
    let settings = layout.profile_dir("work").join("settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(
        &settings,
        r#"{"statusLine":{"type":"command","command":"node original.mjs"}}"#,
    )
    .unwrap();

    let plan = plan_install(&config, &layout, &Target::Profile("work".into()), None).unwrap();
    install(&plan, OWNED, None).unwrap();
    assert!(settings_of(&settings)["statusLine"]["command"]
        .as_str()
        .unwrap()
        .contains("statusline.sh"));

    // Re-plan so the removal sees the installed state, then undo.
    let plan = plan_install(&config, &layout, &Target::Profile("work".into()), None).unwrap();
    remove(&plan, OWNED).unwrap();

    assert_eq!(
        settings_of(&settings)["statusLine"]["command"], "node original.mjs",
        "removing must put back exactly what was there"
    );
    assert!(!plan.script_path.exists(), "the wrapper should be gone");
}

#[test]
fn removing_when_there_was_nothing_before_leaves_no_statusline() {
    let env = Env::new();
    let toml = "version = 1\n[profiles.solo.resources]\nsettings = \"own\"\n";
    let config = env.config(toml);
    let layout = env.layout();
    let settings = layout.profile_dir("solo").join("settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(&settings, r#"{"theme":"dark"}"#).unwrap();

    // No statusline anywhere: the base has one, so blank it first.
    fs::write(env.home().join(".claude/settings.json"), "{}").unwrap();

    let plan = plan_install(&config, &layout, &Target::Profile("solo".into()), None).unwrap();
    install(&plan, toml, None).unwrap();
    let plan = plan_install(&config, &layout, &Target::Profile("solo".into()), None).unwrap();
    remove(&plan, toml).unwrap();

    let written = settings_of(&settings);
    assert!(written.get("statusLine").is_none(), "{written}");
    assert_eq!(written["theme"], "dark");
}

#[test]
fn installing_on_the_base_touches_only_the_status_line_key() {
    let env = Env::new();
    let base = env.home().join(".claude/settings.json");
    fs::write(
        &base,
        r#"{"permissions":{"allow":["a"]},"statusLine":{"command":"node ~/.claude/statusline.mjs"},"theme":"dark"}"#,
    )
    .unwrap();

    let config = env.config(MERGED);
    let plan = plan_install(&config, &env.layout(), &Target::Base, None).unwrap();
    install(&plan, MERGED, None).unwrap();

    let written = settings_of(&base);
    assert_eq!(written["theme"], "dark");
    assert_eq!(written["permissions"]["allow"][0], "a");
    assert!(written["statusLine"]["command"]
        .as_str()
        .unwrap()
        .contains("statusline-base.sh"));
}

#[test]
fn installing_on_the_base_keeps_a_backup_of_the_original() {
    let env = Env::new();
    let base = env.home().join(".claude/settings.json");
    let original = fs::read_to_string(&base).unwrap();

    let config = env.config(MERGED);
    let plan = plan_install(&config, &env.layout(), &Target::Base, None).unwrap();
    let applied = install(&plan, MERGED, None).unwrap();

    let backup = applied.backup.expect("a backup is required for the base");
    assert_eq!(fs::read_to_string(&backup).unwrap(), original);
}

// --- finding the script behind a command ---

#[test]
fn a_node_script_is_found_and_its_tilde_expanded() {
    let home = Path::new("/Users/tester");
    assert_eq!(
        script_path_of("node ~/.claude/statusline.mjs", home),
        Some(PathBuf::from("/Users/tester/.claude/statusline.mjs"))
    );
}

#[test]
fn a_quoted_path_with_spaces_is_found() {
    let home = Path::new("/Users/tester");
    assert_eq!(
        script_path_of("bash '/Users/tester/my scripts/line.sh'", home),
        Some(PathBuf::from("/Users/tester/my scripts/line.sh"))
    );
}

#[test]
fn interpreter_flags_are_skipped() {
    let home = Path::new("/Users/tester");
    assert_eq!(
        script_path_of("node --no-warnings /opt/line.mjs", home),
        Some(PathBuf::from("/opt/line.mjs"))
    );
}

#[test]
fn a_direct_executable_path_is_the_script() {
    let home = Path::new("/Users/tester");
    assert_eq!(
        script_path_of("/usr/local/bin/mystatus --compact", home),
        Some(PathBuf::from("/usr/local/bin/mystatus"))
    );
}

#[test]
fn commands_with_no_file_to_edit_report_none() {
    let home = Path::new("/Users/tester");
    for command in [
        "npx fold-statusline",
        "npm exec statusline",
        "mystatus",
        "",
    ] {
        assert_eq!(script_path_of(command, home), None, "for {command:?}");
    }
}

#[test]
fn arguments_after_the_script_do_not_confuse_it() {
    let home = Path::new("/Users/tester");
    assert_eq!(
        script_path_of("node /opt/line.mjs --theme dark", home),
        Some(PathBuf::from("/opt/line.mjs"))
    );
}

// --- editing the script ---

/// A home whose base statusline runs a script we can open.
fn env_with_script(body: &str) -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join(".claude");
    fs::create_dir_all(&source).unwrap();
    let script = source.join("statusline.mjs");
    fs::write(&script, body).unwrap();
    fs::write(
        source.join("settings.json"),
        format!(
            r#"{{"statusLine":{{"type":"command","command":"node {}"}}}}"#,
            script.display()
        ),
    )
    .unwrap();
    (dir, script)
}

#[test]
fn the_script_behind_the_base_statusline_can_be_read() {
    let (dir, script) = env_with_script("console.log('hi')\n");
    let layout = Layout::new(dir.path());
    let config = Config::parse("version = 1\n", dir.path()).unwrap();

    let found = script_of(&config, &layout, &Target::Base).unwrap().unwrap();
    assert_eq!(found.path, script);
    assert_eq!(found.contents, "console.log('hi')\n");
    assert!(!found.owned, "a script under ~/.claude is not ours");
}

#[test]
fn a_script_installed_by_a_package_manager_is_flagged() {
    let (dir, _) = env_with_script("// install with `npx github:someone/fold-statusline`\nconsole.log(1)\n");
    let layout = Layout::new(dir.path());
    let config = Config::parse("version = 1\n", dir.path()).unwrap();

    let found = script_of(&config, &layout, &Target::Base).unwrap().unwrap();
    assert!(
        found.managed_by.is_some(),
        "editing this in place would be lost on its next update"
    );
    assert!(found.managed_by.unwrap().contains("npx"));
}

#[test]
fn a_plain_script_is_not_flagged() {
    let (dir, _) = env_with_script("console.log('mine')\n");
    let layout = Layout::new(dir.path());
    let config = Config::parse("version = 1\n", dir.path()).unwrap();
    assert_eq!(
        script_of(&config, &layout, &Target::Base).unwrap().unwrap().managed_by,
        None
    );
}

#[test]
fn a_statusline_with_no_file_behind_it_reports_nothing_to_edit() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join(".claude");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("settings.json"),
        r#"{"statusLine":{"command":"npx fold-statusline"}}"#,
    )
    .unwrap();
    let layout = Layout::new(dir.path());
    let config = Config::parse("version = 1\n", dir.path()).unwrap();
    assert_eq!(script_of(&config, &layout, &Target::Base).unwrap(), None);
}

#[test]
fn saving_keeps_the_previous_contents() {
    let (dir, script) = env_with_script("original\n");
    let backup = save_script(&script, "edited\n").unwrap().unwrap();
    assert_eq!(fs::read_to_string(&script).unwrap(), "edited\n");
    assert_eq!(fs::read_to_string(&backup).unwrap(), "original\n");
    drop(dir);
}

#[test]
fn saving_preserves_the_executable_bit() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, script) = env_with_script("#!/bin/sh\necho hi\n");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    save_script(&script, "#!/bin/sh\necho bye\n").unwrap();
    let mode = fs::metadata(&script).unwrap().permissions().mode() & 0o111;
    assert_ne!(mode, 0, "an executable statusline must stay executable");
    drop(dir);
}

#[test]
fn forking_copies_the_script_somewhere_edits_are_safe() {
    let (dir, original) = env_with_script("// install with `npx thing`\nconsole.log(1)\n");
    let layout = Layout::new(dir.path());
    let config = Config::parse("version = 1\n[profiles.work]\n", dir.path()).unwrap();

    let fork = fork_script(&config, &layout, &Target::Profile("work".into()))
        .unwrap()
        .expect("a shared script should be forkable");

    assert!(fork.path.starts_with(&layout.root), "{}", fork.path.display());
    assert_eq!(
        fs::read_to_string(&fork.path).unwrap(),
        fs::read_to_string(&original).unwrap(),
        "the copy should start identical"
    );
    assert!(fork.command.contains("node"), "the interpreter should carry over: {}", fork.command);
    assert!(fork.command.contains(fork.path.to_str().unwrap()));
    assert_eq!(
        fs::read_to_string(&original).unwrap(),
        "// install with `npx thing`\nconsole.log(1)\n",
        "the original must not be touched"
    );
}

#[test]
fn a_script_already_ours_is_not_forked_again() {
    let dir = TempDir::new().unwrap();
    let layout = Layout::new(dir.path());
    let source = dir.path().join(".claude");
    fs::create_dir_all(&source).unwrap();

    let owned = layout.root.join("work").join("custom-line.mjs");
    fs::create_dir_all(owned.parent().unwrap()).unwrap();
    fs::write(&owned, "mine\n").unwrap();
    fs::write(
        source.join("settings.json"),
        format!(r#"{{"statusLine":{{"command":"node {}"}}}}"#, owned.display()),
    )
    .unwrap();

    let config = Config::parse("version = 1\n[profiles.work]\n", dir.path()).unwrap();
    assert!(script_of(&config, &layout, &Target::Profile("work".into()))
        .unwrap()
        .unwrap()
        .owned);
    assert!(fork_script(&config, &layout, &Target::Profile("work".into()))
        .unwrap()
        .is_none());
}
