use cpx_core::config::Config;
use cpx_core::config_edit::*;
use std::path::Path;

fn home() -> &'static Path {
    Path::new("/Users/tester")
}

fn parse(text: &str) -> Config {
    Config::parse(text, home()).expect("edited config should still parse")
}

const WITH_COMMENTS: &str = r#"# my settings
version = 1
source_dir = "~/.claude"   # where the base lives

[profiles.work]
description = "Company"
model = "sonnet"
add_dirs = ["~/Work"]

[profiles.work.env]
ANTHROPIC_LOG = "debug"
"#;

#[test]
fn a_starter_config_parses() {
    let text = starter_config(&[("work".into(), "Company account".into())]);
    let cfg = parse(&text);
    assert_eq!(cfg.profiles["work"].description, "Company account");
    assert_eq!(cfg.source_dir, home().join(".claude"));
}

#[test]
fn a_starter_config_with_no_profiles_is_still_valid() {
    assert!(parse(&starter_config(&[])).profiles.is_empty());
}

#[test]
fn adding_a_profile_keeps_the_existing_ones() {
    let out = add_profile(WITH_COMMENTS, "personal", "Mine").unwrap();
    let cfg = parse(&out);
    assert_eq!(cfg.profiles.len(), 2);
    assert_eq!(cfg.profiles["personal"].description, "Mine");
    assert_eq!(cfg.profiles["work"].model.as_deref(), Some("sonnet"));
}

#[test]
fn adding_a_profile_preserves_comments_and_layout() {
    let out = add_profile(WITH_COMMENTS, "personal", "Mine").unwrap();
    assert!(out.contains("# my settings"), "{out}");
    assert!(out.contains("# where the base lives"), "{out}");
}

#[test]
fn adding_a_profile_that_exists_is_refused() {
    assert!(matches!(
        add_profile(WITH_COMMENTS, "work", "again"),
        Err(EditError::ProfileExists(_))
    ));
}

#[test]
fn removing_a_profile_takes_its_subtables_with_it() {
    let out = remove_profile(WITH_COMMENTS, "work").unwrap();
    assert!(parse(&out).profiles.is_empty());
    assert!(!out.contains("ANTHROPIC_LOG"), "orphaned subtable left: {out}");
}

#[test]
fn removing_a_profile_that_does_not_exist_is_refused() {
    assert!(matches!(
        remove_profile(WITH_COMMENTS, "nope"),
        Err(EditError::UnknownProfile(_))
    ));
}

#[test]
fn cloning_copies_every_setting() {
    let out = clone_profile(WITH_COMMENTS, "work", "work2").unwrap();
    let cfg = parse(&out);
    let (from, to) = (&cfg.profiles["work"], &cfg.profiles["work2"]);
    assert_eq!(to.model, from.model);
    assert_eq!(to.add_dirs, from.add_dirs);
    assert_eq!(to.env, from.env);
}

#[test]
fn cloning_leaves_the_original_untouched() {
    let out = clone_profile(WITH_COMMENTS, "work", "work2").unwrap();
    let cfg = parse(&out);
    assert_eq!(cfg.profiles["work"].description, "Company");
    assert_eq!(cfg.profiles["work"].env["ANTHROPIC_LOG"], "debug");
}

#[test]
fn cloning_onto_an_existing_name_is_refused() {
    let with_two = add_profile(WITH_COMMENTS, "personal", "Mine").unwrap();
    assert!(matches!(
        clone_profile(&with_two, "work", "personal"),
        Err(EditError::ProfileExists(_))
    ));
}

#[test]
fn cloning_an_unknown_profile_is_refused() {
    assert!(matches!(
        clone_profile(WITH_COMMENTS, "nope", "new"),
        Err(EditError::UnknownProfile(_))
    ));
}

#[test]
fn an_invalid_profile_name_is_refused_rather_than_written() {
    // The name becomes a directory and part of a wrapper filename.
    assert!(add_profile(WITH_COMMENTS, "a/b", "bad").is_err());
    assert!(clone_profile(WITH_COMMENTS, "work", "..").is_err());
}

// --- resource and field editing (used by the desktop app) ---

#[test]
fn setting_a_resource_mode_records_it_for_that_profile_only() {
    let text = add_profile(WITH_COMMENTS, "personal", "Mine").unwrap();
    let out = set_resource_mode(&text, "work", "projects", "link").unwrap();
    let cfg = parse(&out);
    assert_eq!(
        cfg.profiles["work"].resources[&cpx_core::config::ResourceKey::Projects].mode,
        cpx_core::config::ResourceMode::Link
    );
    assert_eq!(
        cfg.profiles["personal"].resources[&cpx_core::config::ResourceKey::Projects].mode,
        cpx_core::config::ResourceMode::Own,
        "other profiles keep the default"
    );
}

#[test]
fn setting_a_resource_mode_twice_does_not_duplicate_the_key() {
    let once = set_resource_mode(WITH_COMMENTS, "work", "projects", "link").unwrap();
    let twice = set_resource_mode(&once, "work", "projects", "copy").unwrap();
    assert_eq!(twice.matches("projects").count(), 1, "{twice}");
    assert_eq!(
        parse(&twice).profiles["work"].resources[&cpx_core::config::ResourceKey::Projects].mode,
        cpx_core::config::ResourceMode::Copy
    );
}

#[test]
fn setting_a_resource_mode_preserves_comments() {
    let out = set_resource_mode(WITH_COMMENTS, "work", "projects", "link").unwrap();
    assert!(out.contains("# my settings"), "{out}");
}

#[test]
fn an_impossible_resource_mode_is_refused_and_nothing_is_returned() {
    // merge only applies to JSON files; commands is a directory.
    assert!(set_resource_mode(WITH_COMMENTS, "work", "commands", "merge").is_err());
    assert!(set_resource_mode(WITH_COMMENTS, "work", "settingz", "copy").is_err());
    assert!(set_resource_mode(WITH_COMMENTS, "work", "projects", "symlink").is_err());
}

#[test]
fn setting_a_resource_mode_on_an_unknown_profile_is_refused() {
    assert!(matches!(
        set_resource_mode(WITH_COMMENTS, "nope", "projects", "link"),
        Err(EditError::UnknownProfile(_))
    ));
}

#[test]
fn a_profile_field_can_be_set_and_cleared() {
    let set = set_profile_field(WITH_COMMENTS, "work", "model", Some("opus")).unwrap();
    assert_eq!(parse(&set).profiles["work"].model.as_deref(), Some("opus"));

    let cleared = set_profile_field(&set, "work", "model", None).unwrap();
    assert_eq!(parse(&cleared).profiles["work"].model, None);
}

#[test]
fn setting_the_description_replaces_it_rather_than_appending() {
    let out = set_profile_field(WITH_COMMENTS, "work", "description", Some("New name")).unwrap();
    let cfg = parse(&out);
    assert_eq!(cfg.profiles["work"].description, "New name");
    assert_eq!(out.matches("description").count(), 1, "{out}");
}

#[test]
fn an_unwritable_profile_field_is_refused() {
    // Only fields the app is meant to edit are writable; `resources` is a
    // table with its own validation and must not be set as a string.
    assert!(set_profile_field(WITH_COMMENTS, "work", "resources", Some("x")).is_err());
    assert!(set_profile_field(WITH_COMMENTS, "work", "nonsense", Some("x")).is_err());
}
