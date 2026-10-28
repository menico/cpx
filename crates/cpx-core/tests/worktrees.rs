//! Worktrees and the sessions that worked in them, against a real git
//! repository and a made-up account.

use cpx_core::config::Provider;
use cpx_core::usage::Target;
use cpx_core::worktrees::{self, WorktreeError};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

struct Fixture {
    _tmp: TempDir,
    repo: PathBuf,
    tree: PathBuf,
    account: Target,
}

/// A repository with one added worktree, and a Claude account whose only
/// session started in the repository and then worked in the worktree.
fn fixture() -> Fixture {
    let tmp = TempDir::new().unwrap();
    let base = fs::canonicalize(tmp.path()).unwrap();
    let repo = base.join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    fs::write(repo.join("a.txt"), "a").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "first"]);
    let tree = base.join("trees/feature");
    git(&repo, &["worktree", "add", "-q", "-b", "feature", tree.to_str().unwrap()]);

    let config = base.join("account");
    let project = config.join("projects/-repo");
    fs::create_dir_all(&project).unwrap();
    let lines = [
        serde_json::json!({ "type": "user", "cwd": repo, "sessionId": "s1" }),
        serde_json::json!({ "type": "ai-title", "aiTitle": "Build the feature", "sessionId": "s1" }),
        serde_json::json!({ "type": "user", "cwd": tree, "sessionId": "s1" }),
    ];
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    fs::write(project.join("s1.jsonl"), text).unwrap();

    let account = Target {
        profile: Some("work".into()),
        provider: Provider::Claude,
        dir: config,
        command: "claude-work".into(),
    };
    Fixture { _tmp: tmp, repo, tree, account }
}

#[test]
fn a_worktree_lists_the_session_that_worked_in_it() {
    let f = fixture();
    let report = worktrees::scan(std::slice::from_ref(&f.account), None);
    assert_eq!(report.repos.len(), 1);
    let repo = &report.repos[0];
    assert_eq!(repo.path, f.repo);
    assert!(repo.worktrees[0].main);

    let tree = repo.worktrees.iter().find(|t| t.path == f.tree).expect("worktree listed");
    assert_eq!(tree.branch.as_deref(), Some("feature"));
    assert_eq!(tree.sessions.len(), 1);
    let session = &tree.sessions[0];
    assert_eq!(session.account, "work");
    assert_eq!(session.title.as_deref(), Some("Build the feature"));
    assert!(!session.live);
    assert_eq!(session.resume_dir.as_deref(), Some(f.repo.as_path()));
    assert_eq!(session.resume, "claude-work --resume s1");
}

#[test]
fn the_cache_picks_up_what_a_transcript_appends() {
    let f = fixture();
    let cache = f.account.dir.join("cache.json");
    worktrees::scan(std::slice::from_ref(&f.account), Some(&cache));

    let line = serde_json::json!({ "type": "custom-title", "customTitle": "Renamed", "sessionId": "s1" });
    let path = f.account.dir.join("projects/-repo/s1.jsonl");
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str(&format!("{line}\n"));
    fs::write(&path, text).unwrap();

    let report = worktrees::scan(std::slice::from_ref(&f.account), Some(&cache));
    let tree = report.repos[0].worktrees.iter().find(|t| t.path == f.tree).unwrap();
    assert_eq!(tree.sessions[0].title.as_deref(), Some("Renamed"));
}

#[test]
fn details_measure_size_and_changes() {
    let f = fixture();
    fs::write(f.tree.join("new.txt"), "x").unwrap();
    let mut report = worktrees::scan(std::slice::from_ref(&f.account), None);
    worktrees::fill_details(&mut report);
    let tree = report.repos[0].worktrees.iter().find(|t| t.path == f.tree).unwrap();
    assert_eq!(tree.changes, Some(1));
    assert!(tree.size_bytes.unwrap() > 0);
    assert_eq!(report.repos[0].worktrees[0].size_bytes, None, "the main tree is not measured");
}

#[test]
fn the_main_working_tree_is_never_removed() {
    let f = fixture();
    let error = worktrees::remove(&f.repo, None, &[], true).unwrap_err();
    assert!(matches!(error, WorktreeError::MainWorktree(_)), "{error}");
    assert!(f.repo.join("a.txt").exists());
}

#[test]
fn uncommitted_changes_need_force() {
    let f = fixture();
    fs::write(f.tree.join("new.txt"), "x").unwrap();
    let error = worktrees::remove(&f.tree, None, &[], false).unwrap_err();
    assert!(matches!(error, WorktreeError::Dirty(_)), "{error}");
    assert!(f.tree.exists());

    let removed = worktrees::remove(&f.tree, None, &[], true).unwrap();
    assert_eq!(removed.branch.as_deref(), Some("feature"));
    assert!(!f.tree.exists());
}

#[test]
fn a_running_session_keeps_its_worktree() {
    let f = fixture();
    let sessions = f.account.dir.join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    let me = std::process::id();
    let live = serde_json::json!({ "pid": me, "sessionId": "s2", "cwd": f.tree, "name": "busy-one", "status": "busy" });
    fs::write(sessions.join(format!("{me}.json")), live.to_string()).unwrap();

    let error = worktrees::remove(&f.tree, None, std::slice::from_ref(&f.account), false).unwrap_err();
    assert!(matches!(error, WorktreeError::InUse { .. }), "{error}");
    assert!(error.to_string().contains("busy-one (work)"));

    let report = worktrees::scan(std::slice::from_ref(&f.account), None);
    let tree = report.repos[0].worktrees.iter().find(|t| t.path == f.tree).unwrap();
    assert!(tree.sessions[0].live, "the running session comes first");
    assert_eq!(tree.sessions[0].status.as_deref(), Some("busy"));
}

#[test]
fn a_clean_worktree_is_removed_and_its_branch_kept() {
    let f = fixture();
    let removed = worktrees::remove(&f.tree, None, std::slice::from_ref(&f.account), false).unwrap();
    assert_eq!(removed.repo, f.repo);
    assert!(!f.tree.exists());
    let branches = Command::new("git").arg("-C").arg(&f.repo).args(["branch", "--list", "feature"]).output().unwrap();
    assert!(String::from_utf8_lossy(&branches.stdout).contains("feature"));
}

#[test]
fn a_worktree_whose_directory_is_gone_is_pruned() {
    let f = fixture();
    fs::remove_dir_all(&f.tree).unwrap();
    worktrees::remove(&f.tree, Some(&f.repo), &[], false).unwrap();
    let report = worktrees::scan(std::slice::from_ref(&f.account), None);
    assert!(report.repos.is_empty(), "only the main tree is left, so the repo drops out");
}
