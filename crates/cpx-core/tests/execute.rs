//! Executor behaviour, with one test per safety invariant.

use cpx_core::config::Config;
use cpx_core::execute::{execute, ExecError, ExecuteOptions};
use cpx_core::layout::Layout;
use cpx_core::materialize::{plan_apply, ApplyOptions};
use cpx_core::plan::{Action, Plan, Risk};
use cpx_core::state::{State, MARKER};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

struct Env {
    dir: TempDir,
    layout: Layout,
    state: State,
}

impl Env {
    fn new() -> Env {
        let dir = TempDir::new().unwrap();
        let home = dir.path().to_path_buf();
        let source = home.join(".claude");
        for sub in ["commands", "skills", "agents", "plugins", "hooks"] {
            fs::create_dir_all(source.join(sub)).unwrap();
        }
        fs::write(source.join("commands").join("a.md"), "command a").unwrap();
        fs::write(source.join("settings.json"), r#"{"model":"opus"}"#).unwrap();
        fs::write(source.join("CLAUDE.md"), "# base\n").unwrap();
        Env {
            layout: Layout::new(&home),
            state: State::default(),
            dir,
        }
    }

    fn home(&self) -> &Path {
        self.dir.path()
    }

    fn source(&self) -> PathBuf {
        self.home().join(".claude")
    }

    fn config(&self, toml: &str) -> Config {
        Config::parse(toml, self.home()).unwrap()
    }

    fn plan(&self, toml: &str) -> Plan {
        plan_apply(
            &self.config(toml),
            &self.layout,
            &self.state,
            &ApplyOptions::default(),
        )
        .unwrap()
    }

    /// Plan and run in one step, the way `cpx apply` does.
    fn apply(&mut self, toml: &str) -> Result<(), ExecError> {
        self.apply_with(toml, &ExecuteOptions::default())
    }

    fn apply_with(&mut self, toml: &str, opts: &ExecuteOptions) -> Result<(), ExecError> {
        let plan = self.plan(toml);
        let source = self.source();
        execute(&plan, &mut self.state, &source, opts).map(|_| ())
    }

    fn profile(&self, name: &str) -> PathBuf {
        self.layout.profile_dir(name)
    }
}

const WORK: &str = "version = 1\n[profiles.work]\n";

#[test]
fn applying_creates_the_profile_tree() {
    let mut env = Env::new();
    env.apply(WORK).unwrap();
    assert!(env.profile("work").is_dir());
    assert!(env.profile("work").join("bin").is_dir());
    assert!(env.profile("work").join("projects").is_dir());
}

#[test]
fn linked_resources_become_symlinks_to_the_source() {
    let mut env = Env::new();
    env.apply(WORK).unwrap();
    let link = env.profile("work").join("commands");
    assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
    assert_eq!(fs::read_link(&link).unwrap(), env.source().join("commands"));
    assert_eq!(
        fs::read_to_string(link.join("a.md")).unwrap(),
        "command a",
        "the link should resolve to real content"
    );
}

#[test]
fn copied_resources_are_real_independent_files() {
    let mut env = Env::new();
    env.apply(WORK).unwrap();
    let copied = env.profile("work").join("CLAUDE.md");
    assert!(!fs::symlink_metadata(&copied).unwrap().is_symlink());
    assert_eq!(fs::read_to_string(&copied).unwrap(), "# base\n");
}

#[test]
fn merged_settings_are_written_with_the_patch_applied() {
    let mut env = Env::new();
    env.apply(
        "version = 1\n[profiles.work.resources.settings]\nmode = \"merge\"\npatch = { model = \"sonnet\" }\n",
    )
    .unwrap();
    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(env.profile("work").join("settings.json")).unwrap())
            .unwrap();
    assert_eq!(written["model"], "sonnet");
}

#[test]
fn the_wrapper_is_written_executable() {
    let mut env = Env::new();
    env.apply(WORK).unwrap();
    let wrapper = env.home().join(".local/bin/claude-work");
    assert!(wrapper.is_file());
    assert_ne!(
        fs::metadata(&wrapper).unwrap().permissions().mode() & 0o111,
        0,
        "a wrapper nobody can execute is not a wrapper"
    );
}

#[test]
fn the_profile_directory_is_private() {
    let mut env = Env::new();
    env.apply(WORK).unwrap();
    let mode = fs::metadata(env.profile("work")).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700, "credentials live here; got {mode:o}");
}

#[test]
fn a_second_apply_has_nothing_left_to_do() {
    let mut env = Env::new();
    env.apply(WORK).unwrap();
    let replanned = env.plan(WORK);
    assert!(
        replanned.is_empty(),
        "apply must converge; still planned: {:#?}",
        replanned.actions
    );
}

#[test]
fn applying_twice_changes_nothing_on_disk() {
    let mut env = Env::new();
    env.apply(WORK).unwrap();
    let before = fs::read_to_string(env.profile("work").join("settings.json")).unwrap();
    env.apply(WORK).unwrap();
    assert_eq!(
        fs::read_to_string(env.profile("work").join("settings.json")).unwrap(),
        before
    );
}

#[test]
fn a_plan_that_writes_into_the_source_directory_is_rejected_whole() {
    let mut env = Env::new();
    let mut plan = Plan::default();
    let victim = env.source().join("settings.json");
    plan.push(
        Action::CreateDir {
            path: env.profile("work"),
        },
        Risk::Safe,
        "harmless",
        None,
    );
    plan.push(
        Action::WriteFile {
            path: victim.clone(),
            content: String::from("clobbered"),
            executable: false,
        },
        Risk::Safe,
        "writes into ~/.claude",
        None,
    );

    let source = env.source();
    let err = execute(&plan, &mut env.state, &source, &ExecuteOptions::default()).unwrap_err();
    assert!(matches!(err, ExecError::WritesIntoSource { .. }), "{err:?}");
    assert_eq!(
        fs::read_to_string(&victim).unwrap(),
        r#"{"model":"opus"}"#,
        "the source file must be untouched"
    );
    assert!(
        !env.profile("work").exists(),
        "the harmless action must not have run either"
    );
}

#[test]
fn overwriting_a_foreign_file_is_refused_without_force() {
    let mut env = Env::new();
    let bin = env.home().join(".local/bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("claude-work"), "#!/bin/sh\necho mine\n").unwrap();

    let err = env.apply(WORK).unwrap_err();
    assert!(matches!(err, ExecError::ForceRequired { .. }), "{err:?}");
    assert_eq!(
        fs::read_to_string(bin.join("claude-work")).unwrap(),
        "#!/bin/sh\necho mine\n"
    );
}

#[test]
fn a_refused_plan_performs_none_of_its_safe_actions_either() {
    let mut env = Env::new();
    let bin = env.home().join(".local/bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("claude-work"), "#!/bin/sh\necho mine\n").unwrap();

    assert!(env.apply(WORK).is_err());
    assert!(
        !env.profile("work").exists(),
        "validation must happen before any action runs"
    );
}

#[test]
fn force_backs_a_foreign_file_up_rather_than_destroying_it() {
    let mut env = Env::new();
    let bin = env.home().join(".local/bin");
    fs::create_dir_all(&bin).unwrap();
    let wrapper = bin.join("claude-work");
    fs::write(&wrapper, "#!/bin/sh\necho mine\n").unwrap();

    env.apply_with(WORK, &ExecuteOptions { force: true }).unwrap();

    assert!(
        fs::read_to_string(&wrapper).unwrap().contains(MARKER),
        "the wrapper should now be ours"
    );
    let backups: Vec<_> = fs::read_dir(&bin)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.contains(".cpx.bak"))
        .collect();
    assert_eq!(backups.len(), 1, "expected one backup, found {backups:?}");
    let restored = fs::read_to_string(bin.join(&backups[0])).unwrap();
    assert_eq!(restored, "#!/bin/sh\necho mine\n", "backup must be verbatim");
}

#[test]
fn successive_backups_do_not_overwrite_each_other() {
    let mut env = Env::new();
    let bin = env.home().join(".local/bin");
    fs::create_dir_all(&bin).unwrap();
    let wrapper = bin.join("claude-work");

    for _ in 0..2 {
        fs::write(&wrapper, "#!/bin/sh\necho mine\n").unwrap();
        env.state.forget(&wrapper);
        env.apply_with(WORK, &ExecuteOptions { force: true }).unwrap();
    }

    let backups = fs::read_dir(&bin)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains(".cpx.bak"))
        .count();
    assert_eq!(backups, 2, "each rescue needs its own backup file");
}

#[test]
fn removing_something_cpx_did_not_generate_is_refused() {
    let mut env = Env::new();
    let victim = env.home().join("precious.txt");
    fs::write(&victim, "irreplaceable").unwrap();

    let mut plan = Plan::default();
    plan.push(
        Action::RemoveGenerated {
            path: victim.clone(),
        },
        Risk::OverwritesGenerated,
        "remove",
        None,
    );

    let source = env.source();
    let err = execute(&plan, &mut env.state, &source, &ExecuteOptions { force: true }).unwrap_err();
    assert!(matches!(err, ExecError::NotOurs { .. }), "{err:?}");
    assert_eq!(fs::read_to_string(&victim).unwrap(), "irreplaceable");
}

#[test]
fn a_stale_wrapper_we_generated_is_removed() {
    let mut env = Env::new();
    env.apply("version = 1\n[profiles.work]\n[profiles.old]\n").unwrap();
    let stale = env.home().join(".local/bin/claude-old");
    assert!(stale.exists());

    env.apply(WORK).unwrap();
    assert!(!stale.exists(), "the wrapper for a removed profile should go");
    assert!(env.home().join(".local/bin/claude-work").exists());
}

#[test]
fn state_records_every_artifact_so_a_later_run_recognises_it() {
    let mut env = Env::new();
    env.apply(WORK).unwrap();
    let recorded = env.state.paths_for("work");
    for expected in [
        env.home().join(".local/bin/claude-work"),
        env.profile("work").join("settings.json"),
        env.profile("work").join("commands"),
    ] {
        assert!(
            recorded.contains(&expected),
            "{} not recorded in {recorded:#?}",
            expected.display()
        );
    }
}

#[test]
fn a_hand_edit_survives_a_later_apply_unless_forced() {
    let mut env = Env::new();
    env.apply(WORK).unwrap();
    let settings = env.profile("work").join("settings.json");
    fs::write(&settings, r#"{"model":"mine"}"#).unwrap();

    let err = env.apply(WORK).unwrap_err();
    assert!(matches!(err, ExecError::ForceRequired { .. }), "{err:?}");
    assert_eq!(fs::read_to_string(&settings).unwrap(), r#"{"model":"mine"}"#);
}

#[test]
fn copy_tree_copies_a_directory_recursively() {
    let mut env = Env::new();
    let plan_toml = "version = 1\n[profiles.work.resources]\ncommands = \"copy\"\n";
    env.apply(plan_toml).unwrap();
    let copied = env.profile("work").join("commands");
    assert!(!fs::symlink_metadata(&copied).unwrap().is_symlink());
    assert_eq!(fs::read_to_string(copied.join("a.md")).unwrap(), "command a");
}
