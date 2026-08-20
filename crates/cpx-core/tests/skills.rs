//! Listing a profile's skills, and switching them off without losing them.

use cpx_core::config::Config;
use cpx_core::layout::Layout;
use cpx_core::skills::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

struct Env {
    dir: TempDir,
}

impl Env {
    fn new() -> Env {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".claude")).unwrap();
        Env { dir }
    }

    fn home(&self) -> &Path {
        self.dir.path()
    }

    fn layout(&self) -> Layout {
        Layout::new(self.home())
    }

    fn config(&self) -> Config {
        Config::parse("version = 1\n[profiles.work]\n", self.home()).unwrap()
    }

    fn profile_dir(&self) -> std::path::PathBuf {
        self.layout().profile_dir("work")
    }

    /// A skill of the user's own, with front matter like a real one.
    fn add_skill(&self, name: &str, description: &str) {
        let dir = self.profile_dir().join("skills").join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n\nBody.\n"),
        )
        .unwrap();
    }

    /// A plugin providing skills, laid out the way the cache does.
    fn add_plugin(&self, marketplace: &str, plugin: &str, version: &str, skills: &[&str]) {
        for skill in skills {
            let dir = self
                .profile_dir()
                .join("plugins/cache")
                .join(marketplace)
                .join(plugin)
                .join(version)
                .join("skills")
                .join(skill);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("SKILL.md"), "---\nname: x\n---\n").unwrap();
        }
    }

    fn set_enabled_plugins(&self, entries: &[(&str, bool)]) {
        let map: serde_json::Map<String, serde_json::Value> = entries
            .iter()
            .map(|(k, v)| (k.to_string(), serde_json::json!(v)))
            .collect();
        let settings = self.profile_dir().join("settings.json");
        fs::create_dir_all(settings.parent().unwrap()).unwrap();
        fs::write(
            &settings,
            serde_json::to_string(&serde_json::json!({ "enabledPlugins": map })).unwrap(),
        )
        .unwrap();
    }

    fn inventory(&self) -> Inventory {
        inventory(&self.config(), &self.layout(), "work").unwrap()
    }
}

#[test]
fn a_profile_with_nothing_has_an_empty_inventory() {
    let env = Env::new();
    let inv = env.inventory();
    assert!(inv.own.is_empty());
    assert!(inv.plugins.is_empty());
    assert!(!inv.shared);
}

#[test]
fn skills_of_your_own_are_listed_with_their_description() {
    let env = Env::new();
    env.add_skill("adr-writer", "Writes architecture decision records");
    let inv = env.inventory();
    assert_eq!(inv.own.len(), 1);
    assert_eq!(inv.own[0].name, "adr-writer");
    assert_eq!(
        inv.own[0].description.as_deref(),
        Some("Writes architecture decision records")
    );
    assert!(inv.own[0].enabled);
}

#[test]
fn a_directory_without_a_skill_file_is_not_a_skill() {
    let env = Env::new();
    fs::create_dir_all(env.profile_dir().join("skills").join("notes")).unwrap();
    assert!(env.inventory().own.is_empty());
}

#[test]
fn a_folded_description_is_read_from_the_lines_that_follow() {
    // Most real skills fold, because their descriptions run long.
    let env = Env::new();
    let dir = env.profile_dir().join("skills").join("aws-cdk");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("SKILL.md"),
        "---\nname: aws-cdk\ndescription: >-\n  Authors, deploys, and troubleshoots AWS\n  infrastructure using CDK.\n---\n",
    )
    .unwrap();
    assert_eq!(
        env.inventory().own[0].description.as_deref(),
        Some("Authors, deploys, and troubleshoots AWS infrastructure using CDK.")
    );
}

#[test]
fn only_the_first_sentence_of_a_long_description_is_kept() {
    let env = Env::new();
    env.add_skill(
        "verbose",
        "Does one thing. Then a great deal more that nobody reads in a list.",
    );
    assert_eq!(
        env.inventory().own[0].description.as_deref(),
        Some("Does one thing.")
    );
}

#[test]
fn a_folded_description_stops_at_the_next_key() {
    let env = Env::new();
    let dir = env.profile_dir().join("skills").join("bounded");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("SKILL.md"),
        "---\ndescription: >-\n  The description.\nname: bounded\n---\n",
    )
    .unwrap();
    assert_eq!(
        env.inventory().own[0].description.as_deref(),
        Some("The description.")
    );
}

#[test]
fn skills_are_listed_in_a_stable_order() {
    let env = Env::new();
    for name in ["zulu", "alpha", "mike"] {
        env.add_skill(name, "x");
    }
    let names: Vec<_> = env.inventory().own.into_iter().map(|s| s.name).collect();
    assert_eq!(names, vec!["alpha", "mike", "zulu"]);
}

#[test]
fn disabling_moves_a_skill_out_of_the_way_rather_than_deleting_it() {
    let env = Env::new();
    env.add_skill("noisy", "x");
    let moved = set_enabled(&env.config(), &env.layout(), "work", "noisy", false).unwrap();

    assert!(moved.join("SKILL.md").is_file(), "the skill must still exist");
    assert!(
        !env.profile_dir().join("skills/noisy").exists(),
        "Claude scans skills/, so it must not be there any more"
    );
    let inv = env.inventory();
    assert_eq!(inv.own.len(), 1, "a disabled skill is still listed");
    assert!(!inv.own[0].enabled);
}

#[test]
fn enabling_puts_it_back() {
    let env = Env::new();
    env.add_skill("noisy", "x");
    let config = env.config();
    let layout = env.layout();

    set_enabled(&config, &layout, "work", "noisy", false).unwrap();
    set_enabled(&config, &layout, "work", "noisy", true).unwrap();

    assert!(env.profile_dir().join("skills/noisy/SKILL.md").is_file());
    assert!(env.inventory().own[0].enabled);
}

#[test]
fn disabling_a_skill_that_is_not_there_is_refused() {
    let env = Env::new();
    assert!(matches!(
        set_enabled(&env.config(), &env.layout(), "work", "ghost", false),
        Err(SkillError::UnknownSkill(_))
    ));
}

#[test]
fn disabling_will_not_overwrite_a_skill_already_parked_under_that_name() {
    let env = Env::new();
    env.add_skill("dup", "active");
    let parked = env.profile_dir().join("skills.disabled/dup");
    fs::create_dir_all(&parked).unwrap();
    fs::write(parked.join("SKILL.md"), "---\nname: dup\n---\nolder\n").unwrap();

    assert!(matches!(
        set_enabled(&env.config(), &env.layout(), "work", "dup", false),
        Err(SkillError::AlreadyExists(_))
    ));
    assert_eq!(
        fs::read_to_string(parked.join("SKILL.md")).unwrap(),
        "---\nname: dup\n---\nolder\n",
        "the parked copy must be untouched"
    );
}

#[test]
fn removing_keeps_the_skill_where_it_can_be_recovered() {
    let env = Env::new();
    env.add_skill("gone", "x");
    let moved = remove(&env.config(), &env.layout(), "work", "gone").unwrap();

    assert!(moved.join("SKILL.md").is_file(), "cpx does not delete");
    assert!(!env.profile_dir().join("skills/gone").exists());
    assert!(env.inventory().own.is_empty(), "a removed skill is gone from the list");
}

#[test]
fn a_disabled_skill_can_be_removed_too() {
    let env = Env::new();
    env.add_skill("gone", "x");
    let config = env.config();
    let layout = env.layout();
    set_enabled(&config, &layout, "work", "gone", false).unwrap();
    assert!(remove(&config, &layout, "work", "gone").is_ok());
    assert!(env.inventory().own.is_empty());
}

#[test]
fn a_shared_skills_directory_is_flagged() {
    // With skills/ symlinked, switching one off would switch it off for every
    // profile sharing it.
    let env = Env::new();
    let shared = env.home().join(".claude/skills");
    fs::create_dir_all(&shared).unwrap();
    fs::create_dir_all(env.profile_dir()).unwrap();
    std::os::unix::fs::symlink(&shared, env.profile_dir().join("skills")).unwrap();

    assert!(env.inventory().shared);
}

// --- plugin skills ---

#[test]
fn plugins_are_listed_with_how_many_skills_they_bring() {
    let env = Env::new();
    env.add_plugin("official", "superpowers", "1.0.0", &["brainstorming", "tdd"]);
    env.set_enabled_plugins(&[("superpowers@official", true)]);

    let inv = env.inventory();
    assert_eq!(inv.plugins.len(), 1);
    let plugin = &inv.plugins[0];
    assert_eq!(plugin.plugin, "superpowers");
    assert_eq!(plugin.marketplace, "official");
    assert!(plugin.enabled);
    assert_eq!(plugin.skills, 2);
    assert_eq!(plugin.names, vec!["brainstorming", "tdd"]);
}

#[test]
fn only_the_newest_installed_version_of_a_plugin_counts() {
    // Old versions stay in the cache; counting them would double everything.
    let env = Env::new();
    env.add_plugin("official", "devtools", "1.6.0", &["old-one", "shared"]);
    env.add_plugin("official", "devtools", "1.7.0", &["shared"]);
    env.set_enabled_plugins(&[("devtools@official", true)]);

    let plugin = &env.inventory().plugins[0];
    assert_eq!(plugin.skills, 1, "got {:?}", plugin.names);
    assert_eq!(plugin.names, vec!["shared"]);
}

#[test]
fn a_disabled_plugin_is_listed_as_disabled() {
    let env = Env::new();
    env.add_plugin("official", "figma", "2.0.0", &["design"]);
    env.set_enabled_plugins(&[("figma@official", false)]);
    assert!(!env.inventory().plugins[0].enabled);
}

#[test]
fn a_plugin_with_no_skills_still_appears_with_a_count_of_none() {
    let env = Env::new();
    env.set_enabled_plugins(&[("toolsonly@official", true)]);
    let plugin = &env.inventory().plugins[0];
    assert_eq!(plugin.skills, 0);
}

#[test]
fn an_unknown_profile_is_refused() {
    let env = Env::new();
    assert!(matches!(
        inventory(&env.config(), &env.layout(), "nope"),
        Err(SkillError::UnknownProfile(_))
    ));
}

// --- turning a plugin off ---

const OWNED: &str = "version = 1\n[profiles.work.resources]\nsettings = \"own\"\n";
const MERGED: &str = "version = 1\n[profiles.work]\n";

fn config_of(env: &Env, toml: &str) -> Config {
    Config::parse(toml, env.home()).unwrap()
}

#[test]
fn disabling_a_plugin_writes_the_profiles_own_settings() {
    let env = Env::new();
    env.add_plugin("official", "figma", "1.0.0", &["design"]);
    env.set_enabled_plugins(&[("figma@official", true)]);
    let config = config_of(&env, OWNED);

    let change = set_plugin_enabled(&config, &env.layout(), "work", "figma@official", false, OWNED)
        .unwrap();
    assert!(change.settings_path.is_some());
    assert!(!change.needs_apply);

    let inv = inventory(&config, &env.layout(), "work").unwrap();
    assert!(!inv.plugins[0].enabled);
}

#[test]
fn toggling_a_plugin_leaves_other_settings_alone() {
    let env = Env::new();
    let settings = env.profile_dir().join("settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(
        &settings,
        r#"{"theme":"dark","enabledPlugins":{"a@m":true,"b@m":true}}"#,
    )
    .unwrap();

    let config = config_of(&env, OWNED);
    set_plugin_enabled(&config, &env.layout(), "work", "a@m", false, OWNED).unwrap();

    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(written["theme"], "dark");
    assert_eq!(written["enabledPlugins"]["a@m"], false);
    assert_eq!(written["enabledPlugins"]["b@m"], true, "the other plugin is untouched");
}

#[test]
fn a_merged_profile_records_the_change_in_the_config_instead() {
    let env = Env::new();
    let config = config_of(&env, MERGED);
    let change =
        set_plugin_enabled(&config, &env.layout(), "work", "figma@official", false, MERGED).unwrap();

    assert!(change.settings_path.is_none(), "the file is generated, not edited");
    assert!(change.needs_apply);
    let text = change.config_text.unwrap();
    let reparsed = Config::parse(&text, env.home()).unwrap();
    let patch = reparsed.profiles["work"].resources[&cpx_core::config::ResourceKey::Settings]
        .patch
        .clone()
        .unwrap();
    assert_eq!(patch["enabledPlugins"]["figma@official"], false);
}

#[test]
fn toggling_a_second_plugin_keeps_the_first_in_the_patch() {
    let env = Env::new();
    let config = config_of(&env, MERGED);
    let once =
        set_plugin_enabled(&config, &env.layout(), "work", "a@m", false, MERGED).unwrap().config_text.unwrap();

    let config = Config::parse(&once, env.home()).unwrap();
    let twice =
        set_plugin_enabled(&config, &env.layout(), "work", "b@m", false, &once).unwrap().config_text.unwrap();

    let reparsed = Config::parse(&twice, env.home()).unwrap();
    let patch = reparsed.profiles["work"].resources[&cpx_core::config::ResourceKey::Settings]
        .patch
        .clone()
        .unwrap();
    assert_eq!(patch["enabledPlugins"]["a@m"], false);
    assert_eq!(patch["enabledPlugins"]["b@m"], false);
}

#[test]
fn a_plugin_toggle_on_an_unknown_profile_is_refused() {
    let env = Env::new();
    assert!(matches!(
        set_plugin_enabled(&config_of(&env, OWNED), &env.layout(), "nope", "a@m", false, OWNED),
        Err(SkillError::UnknownProfile(_))
    ));
}
