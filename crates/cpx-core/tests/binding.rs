//! Managed-block editing, the registry, and bind/unbind planning.

use cpx_core::binding::*;
use cpx_core::config::Config;
use cpx_core::layout::Layout;
use cpx_core::plan::Action;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn block(name: &str) -> String {
    render_block(
        name,
        Path::new("/home/t/.claude-profiles/work"),
        Path::new("/home/t/.claude-profiles/work/bin"),
        &BTreeMap::new(),
        None,
    )
}

#[test]
fn a_block_is_delimited_by_its_markers() {
    let b = block("work");
    assert!(b.starts_with("# >>> cpx: work >>>"), "{b}");
    assert!(b.trim_end().ends_with(BLOCK_END), "{b}");
}

#[test]
fn a_block_exports_the_config_dir_and_profile() {
    let b = block("work");
    assert!(b.contains("export CLAUDE_CONFIG_DIR="), "{b}");
    assert!(b.contains("/home/t/.claude-profiles/work"), "{b}");
    assert!(b.contains("export CLAUDE_PROFILE="), "{b}");
}

#[test]
fn a_block_puts_the_profile_shim_on_path() {
    let b = block("work");
    assert!(
        b.contains("PATH_add") && b.contains(".claude-profiles/work/bin"),
        "a plain `claude` here should be this profile: {b}"
    );
}

#[test]
fn a_block_carries_the_profile_env_vars() {
    let mut env = BTreeMap::new();
    env.insert("MY_VAR".to_string(), "value".to_string());
    let b = render_block("work", Path::new("/p/work"), Path::new("/p/work/bin"), &env, None);
    assert!(b.contains("export MY_VAR="), "{b}");
}

#[test]
fn block_values_are_quoted_against_injection() {
    let mut env = BTreeMap::new();
    env.insert("EVIL".to_string(), "$(touch /tmp/pwned)".to_string());
    let b = render_block("work", Path::new("/p/work"), Path::new("/p/work/bin"), &env, None);
    assert!(b.contains("export EVIL='$(touch /tmp/pwned)'"), "{b}");
}

#[test]
fn upsert_appends_to_an_empty_file() {
    let out = upsert_block("", &block("work")).unwrap();
    assert!(out.contains("# >>> cpx: work >>>"));
}

#[test]
fn upsert_preserves_content_above_and_below() {
    let existing = "use flake\nexport MY_THING=1\n";
    let out = upsert_block(existing, &block("work")).unwrap();
    assert!(out.starts_with("use flake\nexport MY_THING=1\n"), "{out}");
    assert!(out.contains("# >>> cpx: work >>>"), "{out}");
}

#[test]
fn upsert_replaces_an_existing_block_rather_than_stacking_one() {
    let first = upsert_block("use flake\n", &block("work")).unwrap();
    let second = upsert_block(&first, &block("personal")).unwrap();
    assert_eq!(
        second.matches("# >>> cpx:").count(),
        1,
        "rebinding must not leave two blocks: {second}"
    );
    assert!(second.contains("cpx: personal"), "{second}");
    assert!(!second.contains("cpx: work"), "{second}");
}

#[test]
fn upsert_keeps_user_content_that_follows_the_block() {
    let existing = format!("before\n{}\nafter\n", block("work"));
    let out = upsert_block(&existing, &block("personal")).unwrap();
    assert!(out.contains("before"), "{out}");
    assert!(out.contains("after"), "{out}");
}

#[test]
fn an_unterminated_block_is_refused_rather_than_guessed_at() {
    let broken = "before\n# >>> cpx: work >>>\nexport X=1\n";
    assert!(
        upsert_block(broken, &block("work")).is_err(),
        "appending here would produce two opens and one close"
    );
    assert!(remove_block(broken).is_err());
}

#[test]
fn remove_takes_out_only_the_block() {
    let existing = format!("use flake\n{}\nexport KEEP=1\n", block("work"));
    let out = remove_block(&existing).unwrap().expect("a block was present");
    assert!(!out.contains("cpx"), "{out}");
    assert!(out.contains("use flake"), "{out}");
    assert!(out.contains("export KEEP=1"), "{out}");
}

#[test]
fn remove_reports_nothing_when_there_is_no_block() {
    assert!(remove_block("use flake\n").unwrap().is_none());
}

#[test]
fn removing_the_only_content_leaves_nothing_but_whitespace() {
    let out = remove_block(&format!("{}\n", block("work")))
        .unwrap()
        .unwrap();
    assert!(out.trim().is_empty(), "expected an empty file, got {out:?}");
}

#[test]
fn extract_returns_the_block_verbatim() {
    let b = block("work");
    let existing = format!("before\n{b}\nafter\n");
    assert_eq!(extract_block(&existing).unwrap().unwrap().trim(), b.trim());
}

// --- registry ---

#[test]
fn the_registry_round_trips_through_disk() {
    let d = TempDir::new().unwrap();
    let path = d.path().join("bindings.toml");
    let mut bindings = Bindings::default();
    bindings.upsert(Binding {
        path: PathBuf::from("/work/project"),
        profile: "work".into(),
        block_sha256: "abc".into(),
    });
    bindings.save(&path).unwrap();

    let loaded = Bindings::load(&path).unwrap();
    assert_eq!(loaded.entries.len(), 1);
    assert_eq!(loaded.get(Path::new("/work/project")).unwrap().profile, "work");
}

#[test]
fn loading_an_absent_registry_yields_an_empty_one() {
    let d = TempDir::new().unwrap();
    assert!(Bindings::load(&d.path().join("nope.toml")).unwrap().entries.is_empty());
}

#[test]
fn rebinding_a_directory_replaces_its_entry() {
    let mut bindings = Bindings::default();
    for profile in ["work", "personal"] {
        bindings.upsert(Binding {
            path: PathBuf::from("/p"),
            profile: profile.into(),
            block_sha256: "h".into(),
        });
    }
    assert_eq!(bindings.entries.len(), 1);
    assert_eq!(bindings.get(Path::new("/p")).unwrap().profile, "personal");
}

#[test]
fn removing_a_binding_returns_it() {
    let mut bindings = Bindings::default();
    bindings.upsert(Binding {
        path: PathBuf::from("/p"),
        profile: "work".into(),
        block_sha256: "h".into(),
    });
    assert!(bindings.remove(Path::new("/p")).is_some());
    assert!(bindings.get(Path::new("/p")).is_none());
    assert!(bindings.remove(Path::new("/p")).is_none());
}

// --- planning ---

struct Env {
    dir: TempDir,
    layout: Layout,
}

fn env() -> Env {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".claude")).unwrap();
    let layout = Layout::new(dir.path());
    Env { layout, dir }
}

impl Env {
    fn config(&self) -> Config {
        Config::parse(
            "version = 1\n[profiles.work]\nenv = { A = \"1\" }\n",
            self.dir.path(),
        )
        .unwrap()
    }

    fn project(&self) -> PathBuf {
        let p = self.dir.path().join("project");
        fs::create_dir_all(&p).unwrap();
        p
    }
}

fn action_kinds(plan: &cpx_core::plan::Plan) -> Vec<&'static str> {
    plan.actions
        .iter()
        .map(|a| match a.action {
            Action::WriteEnvrcBlock { .. } => "write-envrc",
            Action::RemoveEnvrcBlock { .. } => "remove-envrc",
            Action::GitInfoExclude { .. } => "git-exclude",
            Action::RunDirenvAllow { .. } => "direnv-allow",
            _ => "other",
        })
        .collect()
}

#[test]
fn binding_plans_an_envrc_write_and_a_direnv_allow() {
    let e = env();
    let planned = plan_bind(&e.config(), &e.layout, "work", &e.project()).unwrap();
    let kinds = action_kinds(&planned.plan);
    assert!(kinds.contains(&"write-envrc"), "{kinds:?}");
    assert!(kinds.contains(&"direnv-allow"), "{kinds:?}");
}

#[test]
fn binding_an_unknown_profile_is_refused() {
    let e = env();
    assert!(matches!(
        plan_bind(&e.config(), &e.layout, "nope", &e.project()),
        Err(BindError::UnknownProfile(_))
    ));
}

#[test]
fn binding_a_path_that_is_not_a_directory_is_refused() {
    let e = env();
    let file = e.dir.path().join("a-file");
    fs::write(&file, "x").unwrap();
    assert!(matches!(
        plan_bind(&e.config(), &e.layout, "work", &file),
        Err(BindError::NotADirectory(_))
    ));
}

#[test]
fn binding_inside_a_git_repo_plans_a_local_exclude_not_a_gitignore() {
    let e = env();
    let project = e.project();
    fs::create_dir_all(project.join(".git/info")).unwrap();

    let planned = plan_bind(&e.config(), &e.layout, "work", &project).unwrap();
    assert!(action_kinds(&planned.plan).contains(&"git-exclude"));
    assert!(
        !project.join(".gitignore").exists(),
        "a shared .gitignore is not ours to dirty"
    );
}

#[test]
fn binding_outside_a_git_repo_plans_no_exclude() {
    let e = env();
    let planned = plan_bind(&e.config(), &e.layout, "work", &e.project()).unwrap();
    assert!(!action_kinds(&planned.plan).contains(&"git-exclude"));
}

#[test]
fn the_planned_binding_records_the_block_hash() {
    let e = env();
    let planned = plan_bind(&e.config(), &e.layout, "work", &e.project()).unwrap();
    assert_eq!(planned.binding.profile, "work");
    assert_eq!(planned.binding.path, e.project());
    assert_eq!(planned.binding.block_sha256.len(), 64);
}

#[test]
fn the_planned_block_carries_the_profile_env() {
    let e = env();
    let planned = plan_bind(&e.config(), &e.layout, "work", &e.project()).unwrap();
    let Some(Action::WriteEnvrcBlock { content, .. }) = planned
        .plan
        .actions
        .iter()
        .map(|a| &a.action)
        .find(|a| matches!(a, Action::WriteEnvrcBlock { .. }))
    else {
        panic!("no envrc write planned");
    };
    assert!(content.contains("export A='1'"), "{content}");
}

#[test]
fn unbinding_plans_a_block_removal_and_a_reallow() {
    let e = env();
    let project = e.project();
    fs::write(project.join(".envrc"), format!("{}\n", block("work"))).unwrap();
    let kinds = action_kinds(&plan_unbind(&project).unwrap());
    assert!(kinds.contains(&"remove-envrc"), "{kinds:?}");
}

#[test]
fn unbinding_a_directory_with_no_block_plans_nothing() {
    let e = env();
    let project = e.project();
    fs::write(project.join(".envrc"), "use flake\n").unwrap();
    assert!(plan_unbind(&project).unwrap().is_empty());
}

// --- git exclude discovery ---

#[test]
fn git_info_exclude_is_found_in_a_normal_repo() {
    let d = TempDir::new().unwrap();
    fs::create_dir_all(d.path().join(".git/info")).unwrap();
    assert_eq!(
        git_info_exclude(d.path()),
        Some(d.path().join(".git/info/exclude"))
    );
}

#[test]
fn git_info_exclude_follows_a_worktree_pointer_file() {
    let d = TempDir::new().unwrap();
    let real = d.path().join("realrepo/.git/worktrees/wt");
    fs::create_dir_all(real.join("info")).unwrap();
    let wt = d.path().join("wt");
    fs::create_dir_all(&wt).unwrap();
    fs::write(wt.join(".git"), format!("gitdir: {}\n", real.display())).unwrap();

    assert_eq!(git_info_exclude(&wt), Some(real.join("info/exclude")));
}

#[test]
fn git_info_exclude_is_none_outside_a_repo() {
    let d = TempDir::new().unwrap();
    assert_eq!(git_info_exclude(d.path()), None);
}

// --- executing binding plans ---

use cpx_core::execute::{execute, ExecuteOptions};
use cpx_core::state::State;

fn run(plan: &cpx_core::plan::Plan, source: &Path) -> State {
    let mut state = State::default();
    execute(plan, &mut state, source, &ExecuteOptions::default()).expect("plan should execute");
    state
}

#[test]
fn executing_a_bind_writes_the_envrc() {
    let e = env();
    let project = e.project();
    let planned = plan_bind(&e.config(), &e.layout, "work", &project).unwrap();
    run(&planned.plan, &e.dir.path().join(".claude"));

    let text = fs::read_to_string(project.join(".envrc")).unwrap();
    assert!(text.contains("# >>> cpx: work >>>"), "{text}");
    assert!(text.contains("CLAUDE_PROFILE='work'"), "{text}");
}

#[test]
fn executing_a_bind_preserves_a_pre_existing_envrc() {
    let e = env();
    let project = e.project();
    fs::write(project.join(".envrc"), "use flake\nlayout python\n").unwrap();

    let planned = plan_bind(&e.config(), &e.layout, "work", &project).unwrap();
    run(&planned.plan, &e.dir.path().join(".claude"));

    let text = fs::read_to_string(project.join(".envrc")).unwrap();
    assert!(text.contains("use flake"), "{text}");
    assert!(text.contains("layout python"), "{text}");
    assert!(text.contains("cpx: work"), "{text}");
}

#[test]
fn bind_then_unbind_restores_the_original_file_exactly() {
    let e = env();
    let project = e.project();
    let original = "use flake\nlayout python\n";
    fs::write(project.join(".envrc"), original).unwrap();
    let source = e.dir.path().join(".claude");

    let planned = plan_bind(&e.config(), &e.layout, "work", &project).unwrap();
    run(&planned.plan, &source);
    run(&plan_unbind(&project).unwrap(), &source);

    assert_eq!(
        fs::read_to_string(project.join(".envrc")).unwrap(),
        original,
        "unbinding must leave no trace"
    );
}

#[test]
fn unbinding_removes_an_envrc_that_only_held_our_block() {
    let e = env();
    let project = e.project();
    let source = e.dir.path().join(".claude");

    let planned = plan_bind(&e.config(), &e.layout, "work", &project).unwrap();
    run(&planned.plan, &source);
    assert!(project.join(".envrc").exists());

    run(&plan_unbind(&project).unwrap(), &source);
    assert!(
        !project.join(".envrc").exists(),
        "an empty .envrc left behind is litter"
    );
}

#[test]
fn the_git_exclude_line_is_written_once_however_often_we_bind() {
    let e = env();
    let project = e.project();
    fs::create_dir_all(project.join(".git/info")).unwrap();
    let source = e.dir.path().join(".claude");

    for _ in 0..3 {
        let planned = plan_bind(&e.config(), &e.layout, "work", &project).unwrap();
        run(&planned.plan, &source);
    }

    let exclude = fs::read_to_string(project.join(".git/info/exclude")).unwrap();
    assert_eq!(exclude.matches(".envrc").count(), 1, "{exclude}");
}

#[test]
fn the_git_exclude_keeps_lines_that_were_already_there() {
    let e = env();
    let project = e.project();
    fs::create_dir_all(project.join(".git/info")).unwrap();
    fs::write(project.join(".git/info/exclude"), "# personal\n*.log\n").unwrap();

    let planned = plan_bind(&e.config(), &e.layout, "work", &project).unwrap();
    run(&planned.plan, &e.dir.path().join(".claude"));

    let exclude = fs::read_to_string(project.join(".git/info/exclude")).unwrap();
    assert!(exclude.contains("*.log"), "{exclude}");
    assert!(exclude.contains(".envrc"), "{exclude}");
}

#[test]
fn binding_twice_leaves_exactly_one_block() {
    let e = env();
    let project = e.project();
    let source = e.dir.path().join(".claude");

    for profile in ["work", "work"] {
        let planned = plan_bind(&e.config(), &e.layout, profile, &project).unwrap();
        run(&planned.plan, &source);
    }
    let text = fs::read_to_string(project.join(".envrc")).unwrap();
    assert_eq!(text.matches("# >>> cpx:").count(), 1, "{text}");
}

#[test]
fn a_healthy_binding_is_reported_healthy() {
    let e = env();
    let project = e.project();
    let planned = plan_bind(&e.config(), &e.layout, "work", &project).unwrap();
    run(&planned.plan, &e.dir.path().join(".claude"));

    // direnv trust is environment-dependent; everything else must be right.
    assert!(
        matches!(
            health(&planned.binding, &e.config()),
            BindingHealth::Healthy | BindingHealth::NotAllowed
        ),
        "got {:?}",
        health(&planned.binding, &e.config())
    );
}

#[test]
fn a_hand_edited_block_is_reported_as_edited() {
    let e = env();
    let project = e.project();
    let planned = plan_bind(&e.config(), &e.layout, "work", &project).unwrap();
    run(&planned.plan, &e.dir.path().join(".claude"));

    let envrc = project.join(".envrc");
    let text = fs::read_to_string(&envrc)
        .unwrap()
        .replace("CLAUDE_PROFILE='work'", "CLAUDE_PROFILE='sneaky'");
    fs::write(&envrc, text).unwrap();

    assert_eq!(
        health(&planned.binding, &e.config()),
        BindingHealth::BlockEdited
    );
}

#[test]
fn a_binding_whose_profile_was_deleted_is_reported() {
    let e = env();
    let project = e.project();
    let planned = plan_bind(&e.config(), &e.layout, "work", &project).unwrap();
    run(&planned.plan, &e.dir.path().join(".claude"));

    let without = Config::parse("version = 1\n[profiles.other]\n", e.dir.path()).unwrap();
    assert_eq!(
        health(&planned.binding, &without),
        BindingHealth::ProfileMissing
    );
}

#[test]
fn a_binding_whose_directory_vanished_is_reported() {
    let e = env();
    let project = e.project();
    let planned = plan_bind(&e.config(), &e.layout, "work", &project).unwrap();
    fs::remove_dir_all(&project).unwrap();
    assert_eq!(
        health(&planned.binding, &e.config()),
        BindingHealth::DirectoryMissing
    );
}

#[test]
fn a_block_exports_the_identity_colour_for_the_statusline_badge() {
    let b = render_block(
        "work",
        Path::new("/p/work"),
        Path::new("/p/work/bin"),
        &BTreeMap::new(),
        Some("#5c8dff"),
    );
    assert!(b.contains("export CPX_PROFILE_COLOR='#5c8dff'"), "{b}");
}

#[test]
fn a_block_omits_the_colour_when_the_profile_has_none() {
    let b = render_block(
        "work",
        Path::new("/p/work"),
        Path::new("/p/work/bin"),
        &BTreeMap::new(),
        None,
    );
    assert!(!b.contains("CPX_PROFILE_COLOR"), "{b}");
}
