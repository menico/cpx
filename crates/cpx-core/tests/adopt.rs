//! Adoption: taking over a directory without changing anything in it.

use cpx_core::adopt::*;
use cpx_core::config::{Config, ResourceKey, ResourceMode};
use cpx_core::execute::{execute, ExecuteOptions};
use cpx_core::layout::Layout;
use cpx_core::materialize::{plan_apply, ApplyOptions};
use cpx_core::state::State;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A home with a populated `~/.claude`, plus a hand-rolled `~/.claude-hd`
/// shaped like the real thing: its own settings, plugins and projects.
struct Env {
    dir: TempDir,
}

impl Env {
    fn new() -> Env {
        let dir = TempDir::new().unwrap();
        let home = dir.path();

        let source = home.join(".claude");
        for sub in ["commands", "skills", "agents", "plugins", "hooks"] {
            fs::create_dir_all(source.join(sub)).unwrap();
        }
        fs::write(source.join("settings.json"), r#"{"model":"opus"}"#).unwrap();
        fs::write(source.join("CLAUDE.md"), "# base\n").unwrap();

        let adopted = home.join(".claude-hd");
        fs::create_dir_all(adopted.join("plugins")).unwrap();
        fs::create_dir_all(adopted.join("projects")).unwrap();
        fs::create_dir_all(adopted.join("skills")).unwrap();
        fs::create_dir_all(adopted.join("sessions")).unwrap();
        fs::write(adopted.join("settings.json"), r#"{"model":"sonnet"}"#).unwrap();
        fs::write(adopted.join(".claude.json"), r#"{"oauthAccount":{"emailAddress":"me@co.io"}}"#).unwrap();
        fs::write(adopted.join("plugins").join("keep.json"), "precious").unwrap();

        Env { dir }
    }

    fn home(&self) -> &Path {
        self.dir.path()
    }

    fn source(&self) -> PathBuf {
        self.home().join(".claude")
    }

    fn adopted(&self) -> PathBuf {
        self.home().join(".claude-hd")
    }

    fn inspect(&self) -> Adoption {
        inspect(&self.adopted(), &self.source(), None).expect("adoption should be possible")
    }
}

#[test]
fn a_profile_name_is_derived_from_the_directory_name() {
    assert_eq!(derive_name(Path::new("/Users/x/.claude-hd")).as_deref(), Some("hd"));
    assert_eq!(derive_name(Path::new("/Users/x/.claude-personal")).as_deref(), Some("personal"));
    assert_eq!(derive_name(Path::new("/Users/x/work-claude")).as_deref(), Some("work-claude"));
}

#[test]
fn a_directory_name_that_yields_nothing_usable_has_no_derived_name() {
    assert_eq!(derive_name(Path::new("/Users/x/.claude-")), None);
    assert_eq!(derive_name(Path::new("/")), None);
}

#[test]
fn adopting_a_missing_directory_is_refused() {
    let env = Env::new();
    assert!(matches!(
        inspect(&env.home().join("nope"), &env.source(), None),
        Err(AdoptError::Missing(_))
    ));
}

#[test]
fn adopting_a_file_is_refused() {
    let env = Env::new();
    let file = env.home().join("a-file");
    fs::write(&file, "x").unwrap();
    assert!(matches!(
        inspect(&file, &env.source(), None),
        Err(AdoptError::NotADirectory(_))
    ));
}

#[test]
fn adopting_a_directory_that_is_not_a_claude_config_dir_is_refused() {
    let env = Env::new();
    let plain = env.home().join("Documents");
    fs::create_dir_all(&plain).unwrap();
    assert!(matches!(
        inspect(&plain, &env.source(), None),
        Err(AdoptError::NotAConfigDir(_))
    ));
}

#[test]
fn adopting_the_source_directory_itself_is_refused() {
    let env = Env::new();
    assert!(
        matches!(
            inspect(&env.source(), &env.source(), None),
            Err(AdoptError::IsSourceDir(_))
        ),
        "~/.claude is what profiles are built from"
    );
}

#[test]
fn resources_the_directory_already_has_become_its_own() {
    let adoption = Env::new().inspect();
    for key in [ResourceKey::Settings, ResourceKey::Plugins, ResourceKey::Projects, ResourceKey::Skills] {
        assert_eq!(
            adoption.resources[&key],
            ResourceMode::Own,
            "{} should be left alone, not managed",
            key.config_name()
        );
    }
}

#[test]
fn resources_the_directory_does_not_have_are_ignored() {
    let adoption = Env::new().inspect();
    for key in [ResourceKey::Commands, ResourceKey::Agents, ResourceKey::Hooks, ResourceKey::ClaudeMd] {
        assert_eq!(
            adoption.resources[&key],
            ResourceMode::Ignore,
            "{} is absent, so cpx should not create it",
            key.config_name()
        );
    }
}

#[test]
fn every_resource_gets_a_decision() {
    let adoption = Env::new().inspect();
    let decided: BTreeSet<_> = adoption.resources.keys().copied().collect();
    let all: BTreeSet<_> = ResourceKey::ALL.into_iter().collect();
    assert_eq!(decided, all);
}

#[test]
fn no_resource_is_linked_or_merged_by_adoption() {
    // Linking would replace a real directory with a symlink; merging would
    // rewrite a real settings.json. Neither belongs in adoption.
    let adoption = Env::new().inspect();
    for (key, mode) in &adoption.resources {
        assert!(
            matches!(mode, ResourceMode::Own | ResourceMode::Ignore),
            "{} was set to {:?}",
            key.config_name(),
            mode
        );
    }
}

#[test]
fn the_report_names_what_was_found() {
    let adoption = Env::new().inspect();
    let joined = adoption.found.join(" ");
    assert!(joined.contains("plugins"), "{joined}");
    assert!(joined.contains("settings"), "{joined}");
}

#[test]
fn an_explicit_name_overrides_the_derived_one() {
    let env = Env::new();
    let adoption = inspect(&env.adopted(), &env.source(), Some("work")).unwrap();
    assert_eq!(adoption.name, "work");
}

// --- what applying an adopted profile actually does ---

fn config_for(env: &Env) -> Config {
    let text = format!(
        r#"
version = 1
source_dir = "{}"
[profiles.hd]
dir = "{}"
[profiles.hd.resources]
settings = "own"
settings_local = "ignore"
"CLAUDE.md" = "ignore"
commands = "ignore"
skills = "own"
agents = "ignore"
plugins = "own"
hooks = "ignore"
projects = "own"
"#,
        env.source().display(),
        env.adopted().display()
    );
    Config::parse(&text, env.home()).unwrap()
}

#[test]
fn applying_an_adopted_profile_writes_only_the_wrapper_and_the_shim() {
    let env = Env::new();
    let config = config_for(&env);
    let layout = Layout::new(env.home());
    let plan = plan_apply(&config, &layout, &State::default(), &ApplyOptions::default()).unwrap();

    let targets: Vec<String> = plan
        .actions
        .iter()
        .map(|a| a.action.target().display().to_string())
        .collect();

    assert!(
        targets.iter().any(|t| t.ends_with(".local/bin/claude-hd")),
        "{targets:#?}"
    );
    assert!(
        targets.iter().any(|t| t.ends_with(".claude-profiles/hd/bin/claude")),
        "{targets:#?}"
    );
    for target in &targets {
        assert!(
            !target.starts_with(env.adopted().to_str().unwrap()),
            "adoption must not touch the adopted directory, but plans {target}"
        );
    }
}

#[test]
fn applying_an_adopted_profile_leaves_its_contents_untouched() {
    let env = Env::new();
    let config = config_for(&env);
    let layout = Layout::new(env.home());
    let mut state = State::default();

    let before = fs::read_to_string(env.adopted().join("settings.json")).unwrap();
    let plan = plan_apply(&config, &layout, &state, &ApplyOptions::default()).unwrap();
    execute(&plan, &mut state, &config.source_dir, &ExecuteOptions::default()).unwrap();

    assert_eq!(
        fs::read_to_string(env.adopted().join("settings.json")).unwrap(),
        before
    );
    assert_eq!(
        fs::read_to_string(env.adopted().join("plugins").join("keep.json")).unwrap(),
        "precious"
    );
    assert!(
        !env.adopted().join("bin").exists(),
        "the shim belongs under the cpx root, not in the adopted directory"
    );
    assert!(
        !env.adopted().join("commands").exists(),
        "an ignored resource must not be created"
    );
}

#[test]
fn the_wrapper_for_an_adopted_profile_points_at_the_adopted_directory() {
    let env = Env::new();
    let config = config_for(&env);
    let layout = Layout::new(env.home());
    let mut state = State::default();

    let plan = plan_apply(&config, &layout, &state, &ApplyOptions::default()).unwrap();
    execute(&plan, &mut state, &config.source_dir, &ExecuteOptions::default()).unwrap();

    let wrapper = fs::read_to_string(env.home().join(".local/bin/claude-hd")).unwrap();
    assert!(
        wrapper.contains(env.adopted().to_str().unwrap()),
        "the wrapper must export the adopted config dir: {wrapper}"
    );
}

#[test]
fn an_adopted_profile_keeps_the_login_its_directory_already_has() {
    // The Keychain service is derived from the config directory's path, so
    // adopting in place is exactly what avoids a re-login.
    let env = Env::new();
    let config = config_for(&env);
    let layout = Layout::new(env.home());
    assert_eq!(
        cpx_core::credentials::keychain_service(&config.config_dir(&layout, "hd")),
        cpx_core::credentials::keychain_service(&env.adopted())
    );
}

#[test]
fn applying_an_adopted_profile_converges() {
    let env = Env::new();
    let config = config_for(&env);
    let layout = Layout::new(env.home());
    let mut state = State::default();

    let plan = plan_apply(&config, &layout, &state, &ApplyOptions::default()).unwrap();
    execute(&plan, &mut state, &config.source_dir, &ExecuteOptions::default()).unwrap();

    let again = plan_apply(&config, &layout, &state, &ApplyOptions::default()).unwrap();
    assert!(again.is_empty(), "still planned: {:#?}", again.actions);
}

// --- discovery ---

#[test]
fn discovery_finds_hand_rolled_config_directories() {
    let env = Env::new();
    let found = candidates(env.home(), &env.source(), &env.home().join(".claude-profiles"));
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].name, "hd");
}

#[test]
fn discovery_skips_the_source_directory_and_the_cpx_root() {
    let env = Env::new();
    let root = env.home().join(".claude-profiles");
    // The cpx root is itself full of config directories; none are adoptable.
    fs::create_dir_all(root.join("work/projects")).unwrap();

    let names: Vec<_> = candidates(env.home(), &env.source(), &root)
        .into_iter()
        .map(|a| a.name)
        .collect();
    assert_eq!(names, vec!["hd"], "got {names:?}");
}

#[test]
fn discovery_ignores_directories_that_are_not_config_dirs() {
    let env = Env::new();
    fs::create_dir_all(env.home().join(".claude-notes")).unwrap();
    let found = candidates(env.home(), &env.source(), &env.home().join(".claude-profiles"));
    assert!(
        found.iter().all(|a| a.name != "notes"),
        "an empty directory is not a profile: {found:#?}"
    );
}

#[test]
fn discovery_returns_directories_in_a_stable_order() {
    let env = Env::new();
    for name in [".claude-zulu", ".claude-alpha"] {
        let dir = env.home().join(name);
        fs::create_dir_all(dir.join("projects")).unwrap();
    }
    let names: Vec<_> = candidates(env.home(), &env.source(), &env.home().join(".claude-profiles"))
        .into_iter()
        .map(|a| a.name)
        .collect();
    assert_eq!(names, vec!["alpha", "hd", "zulu"]);
}
