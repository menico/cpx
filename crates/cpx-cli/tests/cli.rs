//! End-to-end tests driving the real `cpx` binary against a throwaway home.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

struct Cpx {
    dir: TempDir,
}

impl Cpx {
    fn new() -> Cpx {
        let dir = TempDir::new().unwrap();
        let source = dir.path().join(".claude");
        for sub in ["commands", "skills", "agents", "plugins", "hooks"] {
            fs::create_dir_all(source.join(sub)).unwrap();
        }
        fs::write(source.join("settings.json"), r#"{"model":"opus"}"#).unwrap();
        fs::write(source.join("CLAUDE.md"), "# base\n").unwrap();
        Cpx { dir }
    }

    fn home(&self) -> &Path {
        self.dir.path()
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_cpx"))
            .args(args)
            .env("CPX_HOME", self.home())
            .env("CPX_ROOT", self.home().join(".claude-profiles"))
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CLAUDE_PROFILE")
            .output()
            .expect("cpx should run")
    }

    /// Run and require success, returning stdout.
    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "cpx {args:?} failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    fn fails(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(!out.status.success(), "cpx {args:?} unexpectedly succeeded");
        String::from_utf8_lossy(&out.stderr).to_string()
    }

    fn initialised(&self) -> &Cpx {
        self.ok(&["init", "--profile", "work"]);
        self
    }

    fn profile_dir(&self, name: &str) -> PathBuf {
        self.home().join(".claude-profiles").join(name)
    }
}

#[test]
fn init_writes_a_config_that_the_tool_can_read_back() {
    let cpx = Cpx::new();
    cpx.ok(&["init", "--profile", "work"]);
    assert!(cpx.home().join(".claude-profiles/config.toml").is_file());
    assert!(cpx.ok(&["list"]).contains("work"));
}

#[test]
fn init_refuses_to_clobber_an_existing_config() {
    let cpx = Cpx::new();
    cpx.initialised();
    assert!(cpx.fails(&["init"]).contains("already exists"));
}

#[test]
fn commands_before_init_say_what_to_do() {
    let cpx = Cpx::new();
    let err = cpx.fails(&["list"]);
    assert!(err.contains("cpx init"), "{err}");
}

#[test]
fn dry_run_changes_nothing() {
    let cpx = Cpx::new();
    cpx.initialised();
    let out = cpx.ok(&["apply", "--dry-run"]);
    assert!(out.contains("mkdir") || out.contains("write"), "{out}");
    assert!(
        !cpx.profile_dir("work").exists(),
        "--dry-run must not touch the disk"
    );
}

#[test]
fn apply_builds_the_profile_and_then_has_nothing_left_to_do() {
    let cpx = Cpx::new();
    cpx.initialised();
    cpx.ok(&["apply"]);
    assert!(cpx.profile_dir("work").is_dir());
    assert!(cpx.home().join(".local/bin/claude-work").is_file());
    assert!(cpx.ok(&["apply"]).contains("Nothing to do"));
}

#[test]
fn status_reports_pending_work_before_apply_and_none_after() {
    let cpx = Cpx::new();
    cpx.initialised();
    assert!(!cpx.ok(&["status"]).contains("Nothing to do"));
    cpx.ok(&["apply"]);
    assert!(cpx.ok(&["status"]).contains("Nothing to do"));
}

#[test]
fn apply_refuses_to_overwrite_a_foreign_wrapper_but_force_rescues_it() {
    let cpx = Cpx::new();
    cpx.initialised();
    let bin = cpx.home().join(".local/bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("claude-work"), "#!/bin/sh\necho mine\n").unwrap();

    assert!(cpx.fails(&["apply"]).contains("--force"));
    cpx.ok(&["apply", "--force"]);

    let rescued = fs::read_dir(&bin)
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().contains(".cpx.bak"));
    assert!(rescued, "the foreign wrapper should have been backed up");
}

#[test]
fn list_shows_the_command_to_run_for_each_profile() {
    let cpx = Cpx::new();
    cpx.initialised();
    let out = cpx.ok(&["list"]);
    assert!(out.contains("claude-work"), "{out}");
}

#[test]
fn show_reports_the_resolved_resource_modes() {
    let cpx = Cpx::new();
    cpx.initialised();
    let out = cpx.ok(&["show", "work"]);
    assert!(out.contains("commands"), "{out}");
    assert!(out.contains("link"), "{out}");
    assert!(out.contains("merge"), "{out}");
}

#[test]
fn show_of_an_unknown_profile_fails_clearly() {
    let cpx = Cpx::new();
    cpx.initialised();
    assert!(cpx.fails(&["show", "nope"]).contains("nope"));
}

#[test]
fn json_output_is_actually_json() {
    let cpx = Cpx::new();
    cpx.initialised();
    cpx.ok(&["apply"]);
    for args in [
        vec!["--json", "list"],
        vec!["--json", "show", "work"],
        vec!["--json", "status"],
        vec!["--json", "bindings"],
        vec!["--json", "which"],
    ] {
        let out = cpx.ok(&args);
        serde_json::from_str::<serde_json::Value>(&out)
            .unwrap_or_else(|e| panic!("cpx {args:?} emitted invalid JSON: {e}\n{out}"));
    }
}

#[test]
fn doctor_fails_when_something_is_actually_broken() {
    let cpx = Cpx::new();
    cpx.initialised();
    // Never applied, so the profile does not exist.
    let out = cpx.run(&["doctor"]);
    assert!(!out.status.success(), "doctor should exit non-zero on errors");
    assert!(String::from_utf8_lossy(&out.stdout).contains("cpx apply"));
}

#[test]
fn doctor_succeeds_on_a_healthy_installation() {
    let cpx = Cpx::new();
    cpx.initialised();
    cpx.ok(&["apply"]);
    let out = cpx.run(&["doctor"]);
    assert!(
        out.status.success(),
        "doctor failed:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn binding_a_directory_writes_an_envrc_and_records_it() {
    let cpx = Cpx::new();
    cpx.initialised();
    cpx.ok(&["apply"]);
    let project = cpx.home().join("project");
    fs::create_dir_all(&project).unwrap();

    cpx.ok(&["bind", "work", project.to_str().unwrap()]);

    let envrc = fs::read_to_string(project.join(".envrc")).unwrap();
    assert!(envrc.contains("cpx: work"), "{envrc}");
    assert!(envrc.contains("CLAUDE_CONFIG_DIR"), "{envrc}");
    assert!(cpx.ok(&["bindings"]).contains("project"));
}

#[test]
fn unbinding_removes_both_the_block_and_the_registry_entry() {
    let cpx = Cpx::new();
    cpx.initialised();
    cpx.ok(&["apply"]);
    let project = cpx.home().join("project");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join(".envrc"), "use flake\n").unwrap();

    cpx.ok(&["bind", "work", project.to_str().unwrap()]);
    cpx.ok(&["unbind", project.to_str().unwrap()]);

    assert_eq!(
        fs::read_to_string(project.join(".envrc")).unwrap(),
        "use flake\n"
    );
    assert!(cpx.ok(&["bindings"]).contains("No directories bound"));
}

#[test]
fn binding_an_unknown_profile_fails() {
    let cpx = Cpx::new();
    cpx.initialised();
    let project = cpx.home().join("project");
    fs::create_dir_all(&project).unwrap();
    assert!(cpx
        .fails(&["bind", "nope", project.to_str().unwrap()])
        .contains("nope"));
}

#[test]
fn which_reports_nothing_outside_a_bound_directory() {
    let cpx = Cpx::new();
    cpx.initialised();
    assert!(cpx.ok(&["which"]).contains("no profile"));
}

#[test]
fn clone_copies_the_configuration_but_says_credentials_are_not_copied() {
    let cpx = Cpx::new();
    cpx.initialised();
    let out = cpx.ok(&["clone", "work", "work2"]);
    assert!(out.contains("not copied"), "{out}");
    assert!(cpx.ok(&["list"]).contains("work2"));
}

#[test]
fn profile_add_then_remove_round_trips() {
    let cpx = Cpx::new();
    cpx.initialised();
    cpx.ok(&["profile", "add", "personal", "--description", "Mine"]);
    assert!(cpx.ok(&["list"]).contains("personal"));
    cpx.ok(&["profile", "rm", "personal"]);
    assert!(!cpx.ok(&["list"]).contains("personal"));
}

#[test]
fn removing_a_profile_makes_the_next_apply_clean_up_its_wrapper() {
    let cpx = Cpx::new();
    cpx.initialised();
    cpx.ok(&["profile", "add", "temp"]);
    cpx.ok(&["apply"]);
    assert!(cpx.home().join(".local/bin/claude-temp").exists());

    cpx.ok(&["profile", "rm", "temp"]);
    cpx.ok(&["apply"]);
    assert!(!cpx.home().join(".local/bin/claude-temp").exists());
}

#[test]
fn an_invalid_profile_name_is_rejected_before_it_reaches_the_config() {
    let cpx = Cpx::new();
    cpx.initialised();
    cpx.fails(&["profile", "add", "a/b"]);
    // The config must still be loadable afterwards.
    cpx.ok(&["list"]);
}

#[test]
fn run_before_apply_points_at_apply() {
    let cpx = Cpx::new();
    cpx.initialised();
    assert!(cpx.fails(&["run", "work"]).contains("cpx apply"));
}

// --- adoption ---

impl Cpx {
    /// A hand-rolled config directory outside the cpx root, shaped like a
    /// real one: its own settings, plugins and projects.
    fn hand_rolled(&self, name: &str) -> PathBuf {
        let dir = self.home().join(format!(".claude-{name}"));
        fs::create_dir_all(dir.join("plugins")).unwrap();
        fs::create_dir_all(dir.join("projects")).unwrap();
        fs::write(dir.join("settings.json"), r#"{"model":"sonnet"}"#).unwrap();
        fs::write(dir.join("plugins/keep.json"), "precious").unwrap();
        dir
    }
}

#[test]
fn adopt_with_no_arguments_lists_what_could_be_adopted() {
    let cpx = Cpx::new();
    cpx.initialised();
    cpx.hand_rolled("hd");
    let out = cpx.ok(&["adopt"]);
    assert!(out.contains("hd"), "{out}");
    assert!(out.contains(".claude-hd"), "{out}");
}

#[test]
fn adopt_lists_nothing_when_there_is_nothing_to_adopt() {
    let cpx = Cpx::new();
    cpx.initialised();
    assert!(cpx.ok(&["adopt"]).contains("Nothing to adopt"));
}

#[test]
fn adopting_registers_the_profile_at_its_existing_location() {
    let cpx = Cpx::new();
    cpx.initialised();
    let dir = cpx.hand_rolled("hd");

    let out = cpx.ok(&["adopt", dir.to_str().unwrap()]);
    assert!(out.contains("Adopted"), "{out}");
    assert!(cpx.ok(&["list"]).contains("hd"));
    assert!(cpx.ok(&["show", "hd"]).contains(".claude-hd"), "should point at the adopted dir");
}

#[test]
fn applying_an_adopted_profile_changes_nothing_inside_it() {
    let cpx = Cpx::new();
    cpx.initialised();
    let dir = cpx.hand_rolled("hd");
    cpx.ok(&["adopt", dir.to_str().unwrap()]);

    let before: Vec<_> = fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
    cpx.ok(&["apply"]);
    let after: Vec<_> = fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();

    assert_eq!(before.len(), after.len(), "adoption added or removed entries");
    assert_eq!(fs::read_to_string(dir.join("plugins/keep.json")).unwrap(), "precious");
    assert_eq!(fs::read_to_string(dir.join("settings.json")).unwrap(), r#"{"model":"sonnet"}"#);
    assert!(!dir.join("bin").exists(), "the shim must not land in the adopted directory");
    assert!(!dir.join("commands").exists(), "ignored resources must not be created");
}

#[test]
fn an_adopted_profile_still_gets_its_command() {
    let cpx = Cpx::new();
    cpx.initialised();
    let dir = cpx.hand_rolled("hd");
    cpx.ok(&["adopt", dir.to_str().unwrap()]);
    cpx.ok(&["apply"]);

    let wrapper = cpx.home().join(".local/bin/claude-hd");
    assert!(wrapper.is_file());
    assert!(
        fs::read_to_string(&wrapper).unwrap().contains(".claude-hd"),
        "the command must point at the adopted directory"
    );
}

#[test]
fn adopting_the_same_directory_twice_is_refused() {
    let cpx = Cpx::new();
    cpx.initialised();
    let dir = cpx.hand_rolled("hd");
    cpx.ok(&["adopt", dir.to_str().unwrap()]);
    assert!(cpx.fails(&["adopt", dir.to_str().unwrap()]).contains("hd"));
}

#[test]
fn adopting_a_directory_that_is_not_a_config_dir_is_refused() {
    let cpx = Cpx::new();
    cpx.initialised();
    let plain = cpx.home().join("Documents");
    fs::create_dir_all(&plain).unwrap();
    assert!(cpx
        .fails(&["adopt", plain.to_str().unwrap()])
        .contains("does not look like"));
}

#[test]
fn an_adopted_profile_can_be_given_a_different_name() {
    let cpx = Cpx::new();
    cpx.initialised();
    let dir = cpx.hand_rolled("hd");
    cpx.ok(&["adopt", dir.to_str().unwrap(), "--name", "company"]);
    assert!(cpx.ok(&["list"]).contains("claude-company"));
}
