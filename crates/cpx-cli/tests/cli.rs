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
        let codex = dir.path().join(".codex");
        for sub in ["skills", "prompts", "rules", "hooks", "plugins"] {
            fs::create_dir_all(codex.join(sub)).unwrap();
        }
        fs::write(codex.join("config.toml"), "model = \"gpt-5\"\n\n[mcp_servers.figma]\ncommand = \"figma-mcp\"\n").unwrap();
        fs::write(codex.join("AGENTS.md"), "# base\n").unwrap();
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
            .env_remove("CODEX_HOME")
            .env_remove("CODEX_PROFILE")
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


// --- Codex profiles ---

#[test]
fn a_codex_profile_gets_a_codex_command_and_a_merged_config() {
    let cpx = Cpx::new();
    cpx.initialised();
    cpx.ok(&["profile", "add", "gpt", "--provider", "codex"]);
    let listed = cpx.ok(&["list"]);
    assert!(listed.contains("codex-gpt"), "{listed}");
    assert!(listed.contains("codex"), "{listed}");

    cpx.ok(&["apply"]);
    let wrapper = fs::read_to_string(cpx.home().join(".local/bin/codex-gpt")).unwrap();
    assert!(wrapper.contains("export CODEX_HOME="), "{wrapper}");
    assert!(cpx.profile_dir("gpt").join("bin/codex").is_file());

    let config = fs::read_to_string(cpx.profile_dir("gpt").join("config.toml")).unwrap();
    assert!(config.contains("figma-mcp"), "merged from the source: {config}");
    assert!(cpx.profile_dir("gpt").join("skills").is_symlink());
    assert!(cpx.profile_dir("gpt").join("sessions").is_dir());
    assert!(!cpx.profile_dir("gpt").join("settings.json").exists());
}

#[test]
fn a_codex_profile_model_lands_in_config_toml_through_a_patch() {
    let cpx = Cpx::new();
    cpx.initialised();
    cpx.ok(&["profile", "add", "gpt", "--provider", "codex"]);
    let path = cpx.home().join(".claude-profiles/config.toml");
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str("\n[profiles.gpt.resources.config]\nmode = \"merge\"\npatch = { model = \"gpt-5.5\" }\n");
    fs::write(&path, text).unwrap();
    cpx.ok(&["apply"]);
    let config = fs::read_to_string(cpx.profile_dir("gpt").join("config.toml")).unwrap();
    assert!(config.contains("gpt-5.5"), "{config}");
    assert!(config.contains("figma-mcp"), "{config}");
}

#[test]
fn a_directory_can_be_bound_to_a_claude_and_a_codex_profile() {
    let cpx = Cpx::new();
    cpx.initialised();
    cpx.ok(&["profile", "add", "gpt", "--provider", "codex"]);
    cpx.ok(&["apply"]);
    let project = cpx.home().join("project");
    fs::create_dir_all(&project).unwrap();

    cpx.ok(&["bind", "work", project.to_str().unwrap()]);
    cpx.ok(&["bind", "gpt", project.to_str().unwrap()]);

    let envrc = fs::read_to_string(project.join(".envrc")).unwrap();
    assert!(envrc.contains("CLAUDE_CONFIG_DIR"), "{envrc}");
    assert!(envrc.contains("CODEX_HOME"), "{envrc}");
    assert_eq!(envrc.matches("# >>> cpx:").count(), 1, "{envrc}");

    let bound = cpx.ok(&["--json", "bindings"]);
    let rows: Vec<serde_json::Value> = serde_json::from_str(&bound).unwrap();
    assert_eq!(rows.len(), 2, "{bound}");

    cpx.ok(&["unbind", project.to_str().unwrap(), "--provider", "codex"]);
    let envrc = fs::read_to_string(project.join(".envrc")).unwrap();
    assert!(envrc.contains("CLAUDE_CONFIG_DIR"), "{envrc}");
    assert!(!envrc.contains("CODEX_HOME"), "{envrc}");

    cpx.ok(&["unbind", project.to_str().unwrap()]);
    assert!(!project.join(".envrc").exists());
}

#[test]
fn which_reports_each_provider_from_the_environment() {
    let cpx = Cpx::new();
    cpx.initialised();
    let out = Command::new(env!("CARGO_BIN_EXE_cpx"))
        .args(["which"])
        .env("CPX_HOME", cpx.home())
        .env("CPX_ROOT", cpx.home().join(".claude-profiles"))
        .env("CLAUDE_PROFILE", "work")
        .env("CODEX_PROFILE", "gpt")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("claude   work"), "{text}");
    assert!(text.contains("codex    gpt"), "{text}");
}

#[test]
fn a_codex_profile_has_no_statusline() {
    let cpx = Cpx::new();
    cpx.initialised();
    cpx.ok(&["profile", "add", "gpt", "--provider", "codex"]);
    cpx.ok(&["apply"]);
    assert!(cpx.fails(&["statusline", "set", "gpt"]).contains("Codex"));
    // But `show` simply leaves it out rather than failing.
    let shown = cpx.ok(&["statusline", "show"]);
    assert!(!shown.contains("gpt"), "{shown}");
}

#[test]
fn profile_set_and_resource_edit_the_config_in_place() {
    let cpx = Cpx::new();
    cpx.initialised();
    cpx.ok(&["profile", "set", "work", "model", "sonnet"]);
    cpx.ok(&["profile", "set", "work", "color", "#5c8dff"]);
    cpx.ok(&["profile", "resource", "work", "projects", "link"]);
    let shown = cpx.ok(&["--json", "show", "work"]);
    let value: serde_json::Value = serde_json::from_str(&shown).unwrap();
    assert_eq!(value["model"], "sonnet");
    assert_eq!(value["color"], "#5c8dff");
    let projects = value["resources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["resource"] == "projects")
        .unwrap();
    assert_eq!(projects["mode"], "link");

    cpx.ok(&["profile", "set", "work", "model"]);
    let shown = cpx.ok(&["--json", "show", "work"]);
    let value: serde_json::Value = serde_json::from_str(&shown).unwrap();
    assert!(value["model"].is_null());

    assert!(cpx.fails(&["profile", "set", "work", "provider", "codex"]).contains("provider"));
    assert!(cpx.fails(&["profile", "resource", "work", "config", "own"]).contains("claude"));
}

#[test]
fn adopting_a_codex_home_registers_a_codex_profile() {
    let cpx = Cpx::new();
    cpx.initialised();
    let dir = cpx.home().join(".codex-gpt");
    fs::create_dir_all(dir.join("sessions")).unwrap();
    fs::write(dir.join("config.toml"), "model = \"gpt-5\"\n").unwrap();
    fs::write(dir.join("auth.json"), r#"{"tokens":{"access_token":"x"}}"#).unwrap();

    let listed = cpx.ok(&["adopt"]);
    assert!(listed.contains("codex"), "{listed}");
    let out = cpx.ok(&["adopt", dir.to_str().unwrap()]);
    assert!(out.contains("Codex profile `gpt`"), "{out}");
    cpx.ok(&["apply"]);
    assert!(cpx.home().join(".local/bin/codex-gpt").is_file());
    assert_eq!(fs::read_to_string(dir.join("config.toml")).unwrap(), "model = \"gpt-5\"\n");
}

// --- usage and cost ---

#[test]
fn cost_reads_local_transcripts_for_every_account() {
    let cpx = Cpx::new();
    cpx.initialised();
    cpx.ok(&["profile", "add", "gpt", "--provider", "codex"]);
    cpx.ok(&["apply"]);

    let today = chrono_like_today();
    let projects = cpx.profile_dir("work").join("projects/p");
    fs::create_dir_all(&projects).unwrap();
    fs::write(
        projects.join("s.jsonl"),
        format!(
            r#"{{"type":"assistant","timestamp":"{today}T08:00:00Z","requestId":"r1","message":{{"id":"m1","model":"claude-sonnet-4-6","usage":{{"input_tokens":1000000,"output_tokens":0}}}}}}
"#
        ),
    )
    .unwrap();
    let sessions = cpx.profile_dir("gpt").join("sessions/2026/09/16");
    fs::create_dir_all(&sessions).unwrap();
    fs::write(
        sessions.join("rollout-x.jsonl"),
        format!(
            r#"{{"timestamp":"{today}T10:00:00Z","type":"turn_context","payload":{{"model":"gpt-5.4"}}}}
{{"timestamp":"{today}T10:00:02Z","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":100000,"cached_input_tokens":0,"output_tokens":10000}},"last_token_usage":{{"input_tokens":100000,"output_tokens":10000}}}}}}}}
"#
        ),
    )
    .unwrap();

    let out = cpx.ok(&["--json", "cost"]);
    let value: serde_json::Value = serde_json::from_str(&out).unwrap();
    let accounts = value["accounts"].as_array().unwrap();
    let work = accounts.iter().find(|a| a["profile"] == "work").unwrap();
    let gpt = accounts.iter().find(|a| a["profile"] == "gpt").unwrap();
    assert!((work["cost"]["today"]["costUsd"].as_f64().unwrap() - 3.0).abs() < 1e-6, "{work}");
    assert!((gpt["cost"]["today"]["costUsd"].as_f64().unwrap() - 0.4).abs() < 1e-6, "{gpt}");
    assert_eq!(gpt["cost"]["filesScanned"], 1);

    // The second run comes from the cache under the cpx root.
    let again: serde_json::Value = serde_json::from_str(&cpx.ok(&["--json", "cost", "gpt"])).unwrap();
    assert_eq!(again["accounts"][0]["cost"]["filesFromCache"], 1);
    assert!(cpx.home().join(".claude-profiles/cache/cost").is_dir());

    let text = cpx.ok(&["cost", "work"]);
    assert!(text.contains("$3.00"), "{text}");
}

/// Today's local date as YYYY-MM-DD, without pulling a date crate into the
/// CLI tests: the binary formats it the same way.
fn chrono_like_today() -> String {
    let out = Command::new("date").arg("+%Y-%m-%d").output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn usage_reports_a_missing_login_without_failing() {
    let cpx = Cpx::new();
    cpx.initialised();
    cpx.ok(&["profile", "add", "gpt", "--provider", "codex"]);
    cpx.ok(&["apply"]);
    let out = cpx.ok(&["--json", "usage", "gpt"]);
    let value: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(value["accounts"][0]["error"]["kind"], "notSignedIn", "{out}");
    assert!(value["accounts"][0]["usage"].is_null());
    assert!(cpx.ok(&["usage", "gpt"]).contains("not signed in"));
    assert!(cpx.fails(&["usage", "nope"]).contains("@claude"));
}

#[test]
fn paths_say_where_everything_is() {
    let cpx = Cpx::new();
    let before: serde_json::Value = serde_json::from_str(&cpx.ok(&["--json", "paths"])).unwrap();
    assert_eq!(before["initialised"], false);
    cpx.initialised();
    let after: serde_json::Value = serde_json::from_str(&cpx.ok(&["--json", "paths"])).unwrap();
    assert_eq!(after["initialised"], true);
    assert!(after["config"].as_str().unwrap().ends_with("config.toml"));
}
