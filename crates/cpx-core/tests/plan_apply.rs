//! Planner behaviour across every resource mode and prior filesystem state.

use cpx_core::config::Config;
use cpx_core::layout::Layout;
use cpx_core::materialize::{plan_apply, ApplyOptions};
use cpx_core::plan::{Action, Plan, Risk};
use cpx_core::state::{sha256_bytes, State};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

struct Env {
    dir: TempDir,
    layout: Layout,
}

impl Env {
    /// A home with a populated `~/.claude` to materialize from.
    fn new() -> Env {
        let dir = TempDir::new().unwrap();
        let home = dir.path().to_path_buf();
        let source = home.join(".claude");
        fs::create_dir_all(source.join("commands")).unwrap();
        fs::create_dir_all(source.join("skills")).unwrap();
        fs::create_dir_all(source.join("agents")).unwrap();
        fs::create_dir_all(source.join("plugins")).unwrap();
        fs::create_dir_all(source.join("hooks")).unwrap();
        fs::write(source.join("settings.json"), r#"{"model":"opus"}"#).unwrap();
        fs::write(source.join("CLAUDE.md"), "# base\n").unwrap();
        let layout = Layout::new(&home);
        Env { dir, layout }
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
        self.plan_with(toml, &State::default(), &ApplyOptions::default())
    }

    fn plan_synced(&self, toml: &str) -> Plan {
        self.plan_with(
            toml,
            &State::default(),
            &ApplyOptions {
                sync: true,
                ..Default::default()
            },
        )
    }

    fn plan_with(&self, toml: &str, state: &State, opts: &ApplyOptions) -> Plan {
        plan_apply(&self.config(toml), &self.layout, state, opts).expect("planning should succeed")
    }
}

const WORK: &str = "version = 1\n[profiles.work]\n";

/// The single action writing to `suffix`, or None.
fn action_for<'a>(plan: &'a Plan, suffix: &str) -> Option<&'a Action> {
    let matches: Vec<_> = plan
        .actions
        .iter()
        .filter(|a| a.action.target().to_string_lossy().ends_with(suffix))
        .collect();
    assert!(
        matches.len() <= 1,
        "{} actions target {suffix}: {matches:#?}",
        matches.len()
    );
    matches.first().map(|a| &a.action)
}

fn risk_for(plan: &Plan, suffix: &str) -> Option<Risk> {
    plan.actions
        .iter()
        .find(|a| a.action.target().to_string_lossy().ends_with(suffix))
        .map(|a| a.risk)
}

#[test]
fn a_fresh_profile_gets_its_directory() {
    let env = Env::new();
    assert!(matches!(
        action_for(&env.plan(WORK), ".claude-profiles/work"),
        Some(Action::CreateDir { .. })
    ));
}

#[test]
fn link_mode_symlinks_to_the_source_resource() {
    let env = Env::new();
    let plan = env.plan(WORK);
    match action_for(&plan, "work/commands") {
        Some(Action::Symlink { target, .. }) => {
            assert_eq!(*target, env.source().join("commands"))
        }
        other => panic!("expected a symlink, got {other:?}"),
    }
}

#[test]
fn link_mode_skips_a_resource_the_source_does_not_have() {
    let env = Env::new();
    fs::remove_dir_all(env.source().join("skills")).unwrap();
    let plan = env.plan(WORK);
    assert!(action_for(&plan, "work/skills").is_none());
    assert!(
        plan.notes.iter().any(|n| n.contains("skills")),
        "a skipped resource should be reported: {:?}",
        plan.notes
    );
}

#[test]
fn copy_mode_copies_when_the_target_is_absent() {
    let env = Env::new();
    assert!(matches!(
        action_for(&env.plan(WORK), "work/CLAUDE.md"),
        Some(Action::CopyFile { .. })
    ));
}

#[test]
fn copy_mode_leaves_an_existing_target_alone() {
    let env = Env::new();
    let dst = env.layout.profile_dir("work");
    fs::create_dir_all(&dst).unwrap();
    fs::write(dst.join("CLAUDE.md"), "# diverged\n").unwrap();
    assert!(
        action_for(&env.plan(WORK), "work/CLAUDE.md").is_none(),
        "copy is a one-time seed; divergence is the user's to keep"
    );
}

#[test]
fn sync_refreshes_a_copied_resource() {
    let env = Env::new();
    let dst = env.layout.profile_dir("work");
    fs::create_dir_all(&dst).unwrap();
    fs::write(dst.join("CLAUDE.md"), "# diverged\n").unwrap();
    assert!(matches!(
        action_for(&env.plan_synced(WORK), "work/CLAUDE.md"),
        Some(Action::CopyFile { .. })
    ));
}

#[test]
fn sync_over_a_hand_written_file_is_foreign_risk() {
    let env = Env::new();
    let dst = env.layout.profile_dir("work");
    fs::create_dir_all(&dst).unwrap();
    fs::write(dst.join("CLAUDE.md"), "# mine\n").unwrap();
    assert_eq!(
        risk_for(&env.plan_synced(WORK), "work/CLAUDE.md"),
        Some(Risk::OverwritesForeign)
    );
}

#[test]
fn own_mode_creates_an_empty_directory_for_claude_to_fill() {
    let env = Env::new();
    assert!(matches!(
        action_for(&env.plan(WORK), "work/projects"),
        Some(Action::CreateDir { .. })
    ));
}

#[test]
fn own_mode_on_a_file_plans_nothing() {
    let env = Env::new();
    let plan = env.plan("version = 1\n[profiles.work.resources]\nsettings = \"own\"\n");
    assert!(action_for(&plan, "work/settings.json").is_none());
}

#[test]
fn merge_mode_writes_the_source_content_when_there_is_no_patch() {
    let env = Env::new();
    match action_for(&env.plan(WORK), "work/settings.json") {
        Some(Action::WriteFile { content, .. }) => {
            let parsed: serde_json::Value = serde_json::from_str(content).unwrap();
            assert_eq!(parsed["model"], "opus");
        }
        other => panic!("expected a merged write, got {other:?}"),
    }
}

#[test]
fn merge_mode_applies_the_profile_patch_over_the_source() {
    let env = Env::new();
    let plan = env.plan(
        r#"
version = 1
[profiles.work.resources.settings]
mode = "merge"
patch = { model = "sonnet", statusLine = { type = "command" } }
"#,
    );
    match action_for(&plan, "work/settings.json") {
        Some(Action::WriteFile { content, .. }) => {
            let parsed: serde_json::Value = serde_json::from_str(content).unwrap();
            assert_eq!(parsed["model"], "sonnet", "patch should win");
            assert_eq!(parsed["statusLine"]["type"], "command");
        }
        other => panic!("expected a merged write, got {other:?}"),
    }
}

#[test]
fn merge_mode_works_when_the_source_file_is_absent() {
    let env = Env::new();
    fs::remove_file(env.source().join("settings.json")).unwrap();
    let plan = env.plan(
        "version = 1\n[profiles.work.resources.settings]\nmode = \"merge\"\npatch = { model = \"sonnet\" }\n",
    );
    match action_for(&plan, "work/settings.json") {
        Some(Action::WriteFile { content, .. }) => {
            let parsed: serde_json::Value = serde_json::from_str(content).unwrap();
            assert_eq!(parsed["model"], "sonnet");
        }
        other => panic!("expected a merged write, got {other:?}"),
    }
}

#[test]
fn merge_mode_plans_nothing_when_the_target_already_matches() {
    let env = Env::new();
    let plan = env.plan(WORK);
    let Some(Action::WriteFile { path, content, .. }) = action_for(&plan, "work/settings.json")
    else {
        panic!("expected a merged write");
    };
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();

    let mut state = State::default();
    state.record(path, sha256_bytes(content.as_bytes()), "work");

    let replanned = env.plan_with(WORK, &state, &ApplyOptions::default());
    assert!(
        action_for(&replanned, "work/settings.json").is_none(),
        "an already-correct target needs no action"
    );
}

#[test]
fn a_hand_edited_generated_file_is_foreign_risk() {
    let env = Env::new();
    let dst = env.layout.profile_dir("work");
    fs::create_dir_all(&dst).unwrap();
    let settings = dst.join("settings.json");
    fs::write(&settings, r#"{"model":"opus"}"#).unwrap();

    let mut state = State::default();
    state.record(&settings, sha256_bytes(b"something cpx wrote earlier"), "work");

    assert_eq!(
        risk_for(
            &env.plan_with(WORK, &state, &ApplyOptions::default()),
            "work/settings.json"
        ),
        Some(Risk::OverwritesForeign),
        "a hand edit must be backed up, not silently regenerated"
    );
}

#[test]
fn replacing_our_own_unmodified_file_is_only_generated_risk() {
    let env = Env::new();
    let dst = env.layout.profile_dir("work");
    fs::create_dir_all(&dst).unwrap();
    let settings = dst.join("settings.json");
    fs::write(&settings, r#"{"model":"stale"}"#).unwrap();

    let mut state = State::default();
    state.record(&settings, sha256_bytes(br#"{"model":"stale"}"#), "work");

    assert_eq!(
        risk_for(
            &env.plan_with(WORK, &state, &ApplyOptions::default()),
            "work/settings.json"
        ),
        Some(Risk::OverwritesGenerated)
    );
}

#[test]
fn a_foreign_directory_where_a_symlink_belongs_is_foreign_risk() {
    let env = Env::new();
    let commands = env.layout.profile_dir("work").join("commands");
    fs::create_dir_all(&commands).unwrap();
    fs::write(commands.join("mine.md"), "handwritten").unwrap();
    assert_eq!(
        risk_for(&env.plan(WORK), "work/commands"),
        Some(Risk::OverwritesForeign)
    );
}

#[test]
fn the_wrapper_and_shim_are_planned() {
    let env = Env::new();
    let plan = env.plan(WORK);
    assert!(
        matches!(action_for(&plan, ".local/bin/claude-work"), Some(Action::WriteFile { executable: true, .. })),
        "wrapper missing from {:#?}",
        plan.actions
    );
    assert!(matches!(
        action_for(&plan, "work/bin/claude"),
        Some(Action::WriteFile { executable: true, .. })
    ));
}

#[test]
fn the_wrapper_execs_the_configured_claude_binary() {
    let env = Env::new();
    let plan = env.plan_with(
        WORK,
        &State::default(),
        &ApplyOptions {
            sync: false,
            claude_binary: PathBuf::from("/opt/claude/bin/claude"),
        },
    );
    match action_for(&plan, ".local/bin/claude-work") {
        Some(Action::WriteFile { content, .. }) => {
            assert!(content.contains("/opt/claude/bin/claude"), "{content}")
        }
        other => panic!("expected the wrapper, got {other:?}"),
    }
}

#[test]
fn a_foreign_wrapper_is_flagged_rather_than_overwritten_silently() {
    let env = Env::new();
    let bin = env.home().join(".local/bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("claude-work"), "#!/bin/sh\nexec somebody-elses-tool\n").unwrap();
    assert_eq!(
        risk_for(&env.plan(WORK), ".local/bin/claude-work"),
        Some(Risk::OverwritesForeign)
    );
}

#[test]
fn no_action_ever_writes_inside_the_source_directory() {
    let env = Env::new();
    let plan = env.plan("version = 1\n[profiles.work]\n[profiles.personal]\n");
    for action in &plan.actions {
        assert!(
            !action.action.target().starts_with(env.source()),
            "{:?} writes into ~/.claude",
            action.action
        );
    }
}

#[test]
fn every_configured_profile_is_planned() {
    let env = Env::new();
    let plan = env.plan("version = 1\n[profiles.work]\n[profiles.personal]\n");
    assert!(action_for(&plan, ".local/bin/claude-work").is_some());
    assert!(action_for(&plan, ".local/bin/claude-personal").is_some());
}

#[test]
fn a_wrapper_for_a_deleted_profile_is_removed() {
    let env = Env::new();
    let bin = env.home().join(".local/bin");
    fs::create_dir_all(&bin).unwrap();
    let stale = bin.join("claude-gone");
    fs::write(&stale, format!("#!/usr/bin/env bash\n{}\n", cpx_core::state::MARKER)).unwrap();

    let plan = env.plan(WORK);
    assert!(
        matches!(action_for(&plan, "claude-gone"), Some(Action::RemoveGenerated { .. })),
        "a wrapper we generated for a profile that no longer exists should go"
    );
}

#[test]
fn a_foreign_binary_in_bin_dir_is_left_completely_alone() {
    let env = Env::new();
    let bin = env.home().join(".local/bin");
    fs::create_dir_all(&bin).unwrap();
    // This machine really has one of these, from an unrelated tool.
    fs::write(bin.join("claude-company"), "#!/usr/bin/env bash\nexec other\n").unwrap();

    let plan = env.plan(WORK);
    assert!(
        action_for(&plan, "claude-company").is_none(),
        "cpx must not touch a claude-* binary it did not generate"
    );
}
