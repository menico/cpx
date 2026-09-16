//! Managed-block editing, the registry, and bind/unbind planning.

use cpx_core::binding::*;
use cpx_core::config::{Config, Provider};
use cpx_core::layout::Layout;
use cpx_core::plan::Action;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn entry(name: &str, provider: Provider, dir: &str) -> BlockEntry {
    BlockEntry {
        name: name.to_string(),
        provider,
        profile_dir: PathBuf::from(dir),
        profile_bin_dir: PathBuf::from(dir).join("bin"),
        env: BTreeMap::new(),
        color: None,
    }
}

fn block(name: &str) -> String {
    render_block(&[entry(
        name,
        Provider::Claude,
        &format!("/home/t/.claude-profiles/{name}"),
    )])
}

fn with_env(env: BTreeMap<String, String>) -> String {
    render_block(&[BlockEntry {
        env,
        ..entry("work", Provider::Claude, "/p/work")
    }])
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
    let b = with_env(env);
    assert!(b.contains("export MY_VAR="), "{b}");
}

#[test]
fn block_values_are_quoted_against_injection() {
    let mut env = BTreeMap::new();
    env.insert("EVIL".to_string(), "$(touch /tmp/pwned)".to_string());
    let b = with_env(env);
    assert!(b.contains("export EVIL='$(touch /tmp/pwned)'"), "{b}");
}

#[test]
fn a_codex_block_exports_codex_home() {
    let b = render_block(&[entry("work", Provider::Codex, "/p/work")]);
    assert!(b.starts_with("# >>> cpx: work >>>"), "{b}");
    assert!(b.contains("export CODEX_HOME='/p/work'"), "{b}");
    assert!(b.contains("export CODEX_PROFILE='work'"), "{b}");
    assert!(!b.contains("CLAUDE_"), "{b}");
    assert!(b.contains("PATH_add '/p/work/bin'"), "{b}");
}

#[test]
fn a_block_can_hold_one_profile_per_provider() {
    let b = render_block(&[
        entry("work-codex", Provider::Codex, "/p/work-codex"),
        entry("work", Provider::Claude, "/p/work"),
    ]);
    // Claude first, whatever order they were given in.
    assert!(b.starts_with("# >>> cpx: work, work-codex >>>"), "{b}");
    assert!(b.contains("export CLAUDE_CONFIG_DIR='/p/work'"), "{b}");
    assert!(b.contains("export CODEX_HOME='/p/work-codex'"), "{b}");
    assert_eq!(b.matches("PATH_add").count(), 2, "{b}");
    assert_eq!(b.matches(BLOCK_END).count(), 1, "{b}");
    let claude = b.find("CLAUDE_CONFIG_DIR").unwrap();
    let codex = b.find("CODEX_HOME").unwrap();
    assert!(claude < codex, "{b}");
}

#[test]
fn a_two_provider_block_renders_the_same_whichever_way_round() {
    let a = render_block(&[
        entry("work", Provider::Claude, "/p/work"),
        entry("work-codex", Provider::Codex, "/p/work-codex"),
    ]);
    let b = render_block(&[
        entry("work-codex", Provider::Codex, "/p/work-codex"),
        entry("work", Provider::Claude, "/p/work"),
    ]);
    assert_eq!(a, b);
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

fn binding(path: &str, profile: &str, provider: Provider) -> Binding {
    Binding {
        path: PathBuf::from(path),
        profile: profile.into(),
        provider,
        block_sha256: "h".into(),
    }
}

#[test]
fn the_registry_round_trips_through_disk() {
    let d = TempDir::new().unwrap();
    let path = d.path().join("bindings.toml");
    let mut bindings = Bindings::default();
    bindings.upsert(binding("/work/project", "work", Provider::Claude));
    bindings.save(&path).unwrap();

    let loaded = Bindings::load(&path).unwrap();
    assert_eq!(loaded.entries.len(), 1);
    assert_eq!(
        loaded
            .find(Path::new("/work/project"), Provider::Claude)
            .unwrap()
            .profile,
        "work"
    );
}

#[test]
fn a_registry_written_before_codex_support_still_loads() {
    let d = TempDir::new().unwrap();
    let path = d.path().join("bindings.toml");
    fs::write(
        &path,
        "[[bindings]]\npath = \"/work/project\"\nprofile = \"work\"\nblock_sha256 = \"abc\"\n",
    )
    .unwrap();
    let loaded = Bindings::load(&path).unwrap();
    assert_eq!(loaded.entries[0].provider, Provider::Claude);
}

#[test]
fn loading_an_absent_registry_yields_an_empty_one() {
    let d = TempDir::new().unwrap();
    assert!(Bindings::load(&d.path().join("nope.toml")).unwrap().entries.is_empty());
}

#[test]
fn rebinding_a_directory_replaces_its_entry_for_that_provider() {
    let mut bindings = Bindings::default();
    for profile in ["work", "personal"] {
        bindings.upsert(binding("/p", profile, Provider::Claude));
    }
    assert_eq!(bindings.entries.len(), 1);
    assert_eq!(
        bindings.find(Path::new("/p"), Provider::Claude).unwrap().profile,
        "personal"
    );
}

#[test]
fn a_directory_can_hold_a_binding_per_provider() {
    let mut bindings = Bindings::default();
    bindings.upsert(binding("/p", "work", Provider::Claude));
    bindings.upsert(binding("/p", "work-codex", Provider::Codex));
    assert_eq!(bindings.at(Path::new("/p")).len(), 2);
    assert_eq!(bindings.find(Path::new("/p"), Provider::Codex).unwrap().profile, "work-codex");
    assert!(bindings.is_bound(Path::new("/p")));
}

#[test]
fn removing_a_binding_returns_it() {
    let mut bindings = Bindings::default();
    bindings.upsert(binding("/p", "work", Provider::Claude));
    bindings.upsert(binding("/p", "work-codex", Provider::Codex));
    assert!(bindings.remove_provider(Path::new("/p"), Provider::Codex).is_some());
    assert!(bindings.find(Path::new("/p"), Provider::Codex).is_none());
    assert!(bindings.is_bound(Path::new("/p")));
    assert_eq!(bindings.remove(Path::new("/p")).len(), 1);
    assert!(!bindings.is_bound(Path::new("/p")));
    assert!(bindings.remove(Path::new("/p")).is_empty());
}

// --- planning ---

struct Env {
    dir: TempDir,
    layout: Layout,
}

fn env() -> Env {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".claude")).unwrap();
    fs::create_dir_all(dir.path().join(".codex")).unwrap();
    let layout = Layout::new(dir.path());
    Env { layout, dir }
}

impl Env {
    fn config(&self) -> Config {
        Config::parse(
            "version = 1\n[profiles.work]\nenv = { A = \"1\" }\n[profiles.work-codex]\nprovider = \"codex\"\n",
            self.dir.path(),
        )
        .unwrap()
    }

    fn project(&self) -> PathBuf {
        let p = self.dir.path().join("project");
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn sources(&self) -> Vec<PathBuf> {
        self.config().source_dirs()
    }

    fn bind(&self, bindings: &Bindings, profile: &str, dir: &Path) -> BindPlan {
        plan_bind(&self.config(), &self.layout, bindings, profile, dir).unwrap()
    }

    fn unbind(&self, bindings: &Bindings, dir: &Path, provider: Option<Provider>) -> UnbindPlan {
        plan_unbind(&self.config(), &self.layout, bindings, dir, provider).unwrap()
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

fn envrc_content(plan: &cpx_core::plan::Plan) -> String {
    plan.actions
        .iter()
        .find_map(|a| match &a.action {
            Action::WriteEnvrcBlock { content, .. } => Some(content.clone()),
            _ => None,
        })
        .expect("no envrc write planned")
}

#[test]
fn binding_plans_an_envrc_write_and_a_direnv_allow() {
    let e = env();
    let planned = e.bind(&Bindings::default(), "work", &e.project());
    let kinds = action_kinds(&planned.plan);
    assert!(kinds.contains(&"write-envrc"), "{kinds:?}");
    assert!(kinds.contains(&"direnv-allow"), "{kinds:?}");
}

#[test]
fn binding_an_unknown_profile_is_refused() {
    let e = env();
    assert!(matches!(
        plan_bind(&e.config(), &e.layout, &Bindings::default(), "nope", &e.project()),
        Err(BindError::UnknownProfile(_))
    ));
}

#[test]
fn binding_a_path_that_is_not_a_directory_is_refused() {
    let e = env();
    let file = e.dir.path().join("a-file");
    fs::write(&file, "x").unwrap();
    assert!(matches!(
        plan_bind(&e.config(), &e.layout, &Bindings::default(), "work", &file),
        Err(BindError::NotADirectory(_))
    ));
}

#[test]
fn binding_inside_a_git_repo_plans_a_local_exclude_not_a_gitignore() {
    let e = env();
    let project = e.project();
    fs::create_dir_all(project.join(".git/info")).unwrap();

    let planned = e.bind(&Bindings::default(), "work", &project);
    assert!(action_kinds(&planned.plan).contains(&"git-exclude"));
    assert!(
        !project.join(".gitignore").exists(),
        "a shared .gitignore is not ours to dirty"
    );
}

#[test]
fn binding_outside_a_git_repo_plans_no_exclude() {
    let e = env();
    let planned = e.bind(&Bindings::default(), "work", &e.project());
    assert!(!action_kinds(&planned.plan).contains(&"git-exclude"));
}

#[test]
fn the_planned_binding_records_the_block_hash() {
    let e = env();
    let planned = e.bind(&Bindings::default(), "work", &e.project());
    assert_eq!(planned.bindings.len(), 1);
    assert_eq!(planned.bindings[0].profile, "work");
    assert_eq!(planned.bindings[0].provider, Provider::Claude);
    assert_eq!(planned.bindings[0].path, e.project());
    assert_eq!(planned.bindings[0].block_sha256.len(), 64);
}

#[test]
fn the_planned_block_carries_the_profile_env() {
    let e = env();
    let planned = e.bind(&Bindings::default(), "work", &e.project());
    assert!(envrc_content(&planned.plan).contains("export A='1'"));
}

#[test]
fn binding_a_codex_profile_beside_a_claude_one_shares_the_block() {
    let e = env();
    let project = e.project();
    let mut bindings = Bindings::default();

    let first = e.bind(&bindings, "work", &project);
    for b in first.bindings {
        bindings.upsert(b);
    }
    let second = e.bind(&bindings, "work-codex", &project);

    let content = envrc_content(&second.plan);
    assert_eq!(content.matches("# >>> cpx:").count(), 1, "{content}");
    assert!(content.contains("CLAUDE_CONFIG_DIR"), "{content}");
    assert!(content.contains("CODEX_HOME"), "{content}");

    // Both entries are re-recorded with the new block's hash.
    assert_eq!(second.bindings.len(), 2);
    let hashes: Vec<&str> = second.bindings.iter().map(|b| b.block_sha256.as_str()).collect();
    assert_eq!(hashes[0], hashes[1]);
    assert!(second.bindings.iter().any(|b| b.profile == "work"));
    assert!(second.bindings.iter().any(|b| b.profile == "work-codex"));
}

#[test]
fn rebinding_the_same_provider_replaces_rather_than_adds() {
    let e = env();
    let project = e.project();
    let mut bindings = Bindings::default();
    bindings.upsert(binding(project.to_str().unwrap(), "work", Provider::Claude));
    bindings.upsert(binding(project.to_str().unwrap(), "work-codex", Provider::Codex));

    let planned = e.bind(&bindings, "work", &project);
    assert_eq!(planned.bindings.len(), 2, "{:?}", planned.bindings);
    let content = envrc_content(&planned.plan);
    assert_eq!(content.matches("CLAUDE_CONFIG_DIR").count(), 1, "{content}");
    assert_eq!(content.matches("CODEX_HOME").count(), 1, "{content}");
}

#[test]
fn unbinding_plans_a_block_removal_and_a_reallow() {
    let e = env();
    let project = e.project();
    fs::write(project.join(".envrc"), format!("{}\n", block("work"))).unwrap();
    let kinds = action_kinds(&e.unbind(&Bindings::default(), &project, None).plan);
    assert!(kinds.contains(&"remove-envrc"), "{kinds:?}");
}

#[test]
fn unbinding_a_directory_with_no_block_plans_nothing() {
    let e = env();
    let project = e.project();
    fs::write(project.join(".envrc"), "use flake\n").unwrap();
    assert!(e.unbind(&Bindings::default(), &project, None).plan.is_empty());
}

#[test]
fn unbinding_one_provider_keeps_the_other_in_the_block() {
    let e = env();
    let project = e.project();
    let mut bindings = Bindings::default();
    let first = e.bind(&bindings, "work", &project);
    for b in first.bindings {
        bindings.upsert(b);
    }
    let second = e.bind(&bindings, "work-codex", &project);
    for b in second.bindings {
        bindings.upsert(b);
    }
    fs::write(project.join(".envrc"), envrc_content(&second.plan)).unwrap();

    let planned = e.unbind(&bindings, &project, Some(Provider::Codex));
    let kinds = action_kinds(&planned.plan);
    assert!(kinds.contains(&"write-envrc"), "{kinds:?}");
    assert!(!kinds.contains(&"remove-envrc"), "{kinds:?}");
    let content = envrc_content(&planned.plan);
    assert!(content.contains("CLAUDE_CONFIG_DIR"), "{content}");
    assert!(!content.contains("CODEX_HOME"), "{content}");
    assert_eq!(planned.remaining.len(), 1);
    assert_eq!(planned.remaining[0].profile, "work");
}

#[test]
fn unbinding_the_last_provider_removes_the_block() {
    let e = env();
    let project = e.project();
    let mut bindings = Bindings::default();
    let first = e.bind(&bindings, "work", &project);
    for b in first.bindings {
        bindings.upsert(b);
    }
    fs::write(project.join(".envrc"), envrc_content(&first.plan)).unwrap();

    let planned = e.unbind(&bindings, &project, Some(Provider::Claude));
    assert!(action_kinds(&planned.plan).contains(&"remove-envrc"));
    assert!(planned.remaining.is_empty());
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

fn run(plan: &cpx_core::plan::Plan, sources: &[PathBuf]) -> State {
    let mut state = State::default();
    execute(plan, &mut state, sources, &ExecuteOptions::default()).expect("plan should execute");
    state
}

#[test]
fn executing_a_bind_writes_the_envrc() {
    let e = env();
    let project = e.project();
    let planned = e.bind(&Bindings::default(), "work", &project);
    run(&planned.plan, &e.sources());

    let text = fs::read_to_string(project.join(".envrc")).unwrap();
    assert!(text.contains("# >>> cpx: work >>>"), "{text}");
    assert!(text.contains("CLAUDE_PROFILE='work'"), "{text}");
}

#[test]
fn executing_a_bind_preserves_a_pre_existing_envrc() {
    let e = env();
    let project = e.project();
    fs::write(project.join(".envrc"), "use flake\nlayout python\n").unwrap();

    let planned = e.bind(&Bindings::default(), "work", &project);
    run(&planned.plan, &e.sources());

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

    let planned = e.bind(&Bindings::default(), "work", &project);
    run(&planned.plan, &e.sources());
    run(&e.unbind(&Bindings::default(), &project, None).plan, &e.sources());

    assert_eq!(
        fs::read_to_string(project.join(".envrc")).unwrap(),
        original,
        "unbinding must leave no trace"
    );
}

#[test]
fn bind_both_then_unbind_both_restores_the_original_file_exactly() {
    let e = env();
    let project = e.project();
    let original = "use flake\n";
    fs::write(project.join(".envrc"), original).unwrap();
    let mut bindings = Bindings::default();

    for profile in ["work", "work-codex"] {
        let planned = e.bind(&bindings, profile, &project);
        run(&planned.plan, &e.sources());
        for b in planned.bindings {
            bindings.upsert(b);
        }
    }
    let text = fs::read_to_string(project.join(".envrc")).unwrap();
    assert!(text.contains("CODEX_HOME") && text.contains("CLAUDE_CONFIG_DIR"), "{text}");

    for provider in [Provider::Codex, Provider::Claude] {
        let planned = e.unbind(&bindings, &project, Some(provider));
        run(&planned.plan, &e.sources());
        bindings.remove_provider(&project, provider);
        for b in planned.remaining {
            bindings.upsert(b);
        }
    }
    assert_eq!(fs::read_to_string(project.join(".envrc")).unwrap(), original);
}

#[test]
fn unbinding_removes_an_envrc_that_only_held_our_block() {
    let e = env();
    let project = e.project();

    let planned = e.bind(&Bindings::default(), "work", &project);
    run(&planned.plan, &e.sources());
    assert!(project.join(".envrc").exists());

    run(&e.unbind(&Bindings::default(), &project, None).plan, &e.sources());
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

    for _ in 0..3 {
        let planned = e.bind(&Bindings::default(), "work", &project);
        run(&planned.plan, &e.sources());
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

    let planned = e.bind(&Bindings::default(), "work", &project);
    run(&planned.plan, &e.sources());

    let exclude = fs::read_to_string(project.join(".git/info/exclude")).unwrap();
    assert!(exclude.contains("*.log"), "{exclude}");
    assert!(exclude.contains(".envrc"), "{exclude}");
}

#[test]
fn binding_twice_leaves_exactly_one_block() {
    let e = env();
    let project = e.project();

    for profile in ["work", "work"] {
        let planned = e.bind(&Bindings::default(), profile, &project);
        run(&planned.plan, &e.sources());
    }
    let text = fs::read_to_string(project.join(".envrc")).unwrap();
    assert_eq!(text.matches("# >>> cpx:").count(), 1, "{text}");
}

#[test]
fn a_healthy_binding_is_reported_healthy() {
    let e = env();
    let project = e.project();
    let planned = e.bind(&Bindings::default(), "work", &project);
    run(&planned.plan, &e.sources());

    // direnv trust is environment-dependent; everything else must be right.
    let status = health(&planned.bindings[0], &e.config());
    assert!(
        matches!(status, BindingHealth::Healthy | BindingHealth::NotAllowed),
        "got {status:?}"
    );
}

#[test]
fn both_entries_of_a_shared_block_are_healthy() {
    let e = env();
    let project = e.project();
    let mut bindings = Bindings::default();
    let first = e.bind(&bindings, "work", &project);
    run(&first.plan, &e.sources());
    for b in first.bindings {
        bindings.upsert(b);
    }
    let second = e.bind(&bindings, "work-codex", &project);
    run(&second.plan, &e.sources());

    for b in &second.bindings {
        let status = health(b, &e.config());
        assert!(
            matches!(status, BindingHealth::Healthy | BindingHealth::NotAllowed),
            "{}: {status:?}",
            b.profile
        );
    }
}

#[test]
fn a_hand_edited_block_is_reported_as_edited() {
    let e = env();
    let project = e.project();
    let planned = e.bind(&Bindings::default(), "work", &project);
    run(&planned.plan, &e.sources());

    let envrc = project.join(".envrc");
    let text = fs::read_to_string(&envrc)
        .unwrap()
        .replace("CLAUDE_PROFILE='work'", "CLAUDE_PROFILE='sneaky'");
    fs::write(&envrc, text).unwrap();

    assert_eq!(
        health(&planned.bindings[0], &e.config()),
        BindingHealth::BlockEdited
    );
}

#[test]
fn a_binding_whose_profile_was_deleted_is_reported() {
    let e = env();
    let project = e.project();
    let planned = e.bind(&Bindings::default(), "work", &project);
    run(&planned.plan, &e.sources());

    let without = Config::parse("version = 1\n[profiles.other]\n", e.dir.path()).unwrap();
    assert_eq!(
        health(&planned.bindings[0], &without),
        BindingHealth::ProfileMissing
    );
}

#[test]
fn a_binding_whose_directory_vanished_is_reported() {
    let e = env();
    let project = e.project();
    let planned = e.bind(&Bindings::default(), "work", &project);
    fs::remove_dir_all(&project).unwrap();
    assert_eq!(
        health(&planned.bindings[0], &e.config()),
        BindingHealth::DirectoryMissing
    );
}

#[test]
fn a_block_exports_the_identity_colour_for_the_statusline_badge() {
    let b = render_block(&[BlockEntry {
        color: Some("#5c8dff".into()),
        ..entry("work", Provider::Claude, "/p/work")
    }]);
    assert!(b.contains("export CPX_PROFILE_COLOR='#5c8dff'"), "{b}");
}

#[test]
fn a_block_omits_the_colour_when_the_profile_has_none() {
    let b = block("work");
    assert!(!b.contains("CPX_PROFILE_COLOR"), "{b}");
}

#[test]
fn only_the_claude_entry_exports_the_badge_colour() {
    // The badge is a Claude statusline; a Codex colour would only overwrite
    // the one that is actually read.
    let b = render_block(&[
        BlockEntry {
            color: Some("#5c8dff".into()),
            ..entry("work", Provider::Claude, "/p/work")
        },
        BlockEntry {
            color: Some("#d4676b".into()),
            ..entry("work-codex", Provider::Codex, "/p/work-codex")
        },
    ]);
    assert!(b.contains("CPX_PROFILE_COLOR='#5c8dff'"), "{b}");
    assert!(!b.contains("#d4676b"), "{b}");
}
