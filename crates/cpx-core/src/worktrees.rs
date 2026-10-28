//! Git worktrees, and which sessions of which account worked in each.
//!
//! Nothing records the link directly, so it is pieced together from what
//! the tools leave behind. Every transcript line carries the `cwd` it ran
//! in, so the distinct directories a session visited name the repositories
//! worth asking git about. Claude Code also keeps `<config>/sessions/<pid>.json`
//! for each session still running. A worktree a session made in its own
//! scratchpad has that session's id in its path, which links it even when
//! the session never changed directory into it.
//!
//! Transcripts are large and only ever appended to, so each is read once and
//! then from where the last read stopped.

use crate::config::Provider;
use crate::cost::{claude, codex};
use crate::usage::Target;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    #[error("{0} is not a git worktree")]
    NotAWorktree(PathBuf),

    #[error("{0} is the repository's main working tree; only added worktrees can be removed")]
    MainWorktree(PathBuf),

    #[error("{path} is still in use by {session}; pass --force to remove it anyway")]
    InUse { path: PathBuf, session: String },

    #[error("{0} has uncommitted or untracked files; pass --force to discard them")]
    Dirty(PathBuf),

    #[error("{0} is locked; pass --force to remove it anyway")]
    Locked(PathBuf),

    #[error("git: {0}")]
    Git(String),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// Repositories with at least one worktree besides the main one, most
    /// recently used first.
    pub repos: Vec<Repo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    /// The main working tree.
    pub path: PathBuf,
    pub name: String,
    /// The main working tree first, then the rest by last use.
    pub worktrees: Vec<Worktree>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Worktree {
    pub path: PathBuf,
    /// The checked-out branch, without `refs/heads/`; `None` when detached.
    pub branch: Option<String>,
    pub head: Option<String>,
    pub main: bool,
    pub locked: bool,
    /// Git still lists it, but its directory is gone.
    pub prunable: bool,
    /// Live sessions first, then by last activity.
    pub sessions: Vec<SessionRef>,
    /// The latest activity of any session here, in epoch seconds.
    pub last_active: Option<i64>,
    /// Disk use, filled in by [`fill_details`].
    pub size_bytes: Option<u64>,
    /// Files with uncommitted changes, filled in by [`fill_details`].
    pub changes: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRef {
    /// The profile name, or `@claude` / `@codex` for a default session.
    pub account: String,
    pub profile: Option<String>,
    pub provider: Provider,
    pub id: String,
    /// The name the session was given, or the one the tool generated.
    pub title: Option<String>,
    pub live: bool,
    /// What a running Claude session reports: `busy`, `idle`, ...
    pub status: Option<String>,
    pub last_active: i64,
    /// Where the session started, which is where resuming it works.
    pub resume_dir: Option<PathBuf>,
    /// The command that resumes it, run from `resume_dir`.
    pub resume: String,
}

// ---------------------------------------------------------------- transcripts

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct FileFacts {
    size: u64,
    mtime_ns: i128,
    /// Where the next read starts: just past the last complete line.
    offset: u64,
    session: Option<String>,
    /// Directories in the order first seen.
    cwds: Vec<String>,
    title: Option<String>,
    custom_title: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Cache {
    version: u32,
    files: BTreeMap<PathBuf, FileFacts>,
}

const CACHE_VERSION: u32 = 1;

/// Where the transcript index lives: under the cpx root, beside the cost cache.
pub fn cache_path(root: &Path) -> PathBuf {
    root.join("cache").join("worktrees.json")
}

/// The JSON string value that follows `key` in `line`, if any.
fn string_after(line: &str, key: &str) -> Option<String> {
    let at = line.find(key)? + key.len();
    let rest = line[at..].trim_start().strip_prefix(':')?.trim_start();
    serde_json::Deserializer::from_str(rest).into_iter::<String>().next()?.ok()
}

/// Every value of `key` in `line`; a Codex event can carry several.
fn strings_after(line: &str, key: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = line[from..].find(key) {
        let start = from + at;
        if let Some(value) = string_after(&line[start..], key) {
            found.push(value);
        }
        from = start + key.len();
    }
    found
}

/// The session a Claude transcript belongs to. A subagent's transcript sits
/// at `<project>/<session>/subagents/agent-*.jsonl` and counts as its parent.
fn claude_session_of(path: &Path) -> Option<String> {
    let parent = path.parent()?;
    if parent.file_name().is_some_and(|n| n == "subagents") {
        return Some(parent.parent()?.file_name()?.to_string_lossy().into_owned());
    }
    Some(path.file_stem()?.to_string_lossy().into_owned())
}

/// A Codex rollout is `rollout-<timestamp>-<uuid>.jsonl`.
fn codex_session_of(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy();
    (stem.len() >= 36).then(|| stem[stem.len() - 36..].to_string())
}

fn read_from(provider: Provider, path: &Path, facts: &mut FileFacts) -> std::io::Result<()> {
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(facts.offset))?;
    let mut reader = BufReader::new(file);
    let mut seen: BTreeSet<String> = facts.cwds.iter().cloned().collect();
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        let read = reader.read_until(b'\n', &mut buffer)?;
        // A line still being written is left for the next read.
        if read == 0 || buffer.last() != Some(&b'\n') {
            break;
        }
        facts.offset += read as u64;
        let Ok(line) = std::str::from_utf8(&buffer) else {
            continue;
        };
        for cwd in strings_after(line, "\"cwd\"") {
            if seen.insert(cwd.clone()) {
                facts.cwds.push(cwd);
            }
        }
        match provider {
            Provider::Claude => {
                if line.contains("\"type\":\"custom-title\"") {
                    facts.custom_title = string_after(line, "\"customTitle\"").or(facts.custom_title.take());
                } else if line.contains("\"type\":\"ai-title\"") {
                    facts.title = string_after(line, "\"aiTitle\"").or(facts.title.take());
                }
            }
            Provider::Codex => {
                if facts.session.is_none() && line.contains("\"session_meta\"") {
                    facts.session = string_after(line, "\"id\"");
                }
            }
        }
    }
    Ok(())
}

fn mtime_ns(meta: &std::fs::Metadata) -> i128 {
    meta.modified()
        .ok()
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i128)
        .unwrap_or(0)
}

/// The facts of one transcript, reading only what was appended since `cached`.
fn facts_of(provider: Provider, path: &Path, cached: Option<&FileFacts>) -> Option<FileFacts> {
    let meta = std::fs::metadata(path).ok()?;
    let (size, mtime) = (meta.len(), mtime_ns(&meta));
    let mut facts = match cached {
        Some(c) if c.size == size && c.mtime_ns == mtime => return Some(c.clone()),
        // Grown: carry on from where the last read stopped.
        Some(c) if size >= c.offset => c.clone(),
        _ => FileFacts::default(),
    };
    if facts.session.is_none() {
        facts.session = match provider {
            Provider::Claude => claude_session_of(path),
            Provider::Codex => None,
        };
    }
    if read_from(provider, path, &mut facts).is_err() {
        return None;
    }
    if provider == Provider::Codex && facts.session.is_none() {
        facts.session = codex_session_of(path);
    }
    facts.size = size;
    facts.mtime_ns = mtime;
    Some(facts)
}

/// Codex keeps thread names apart from rollouts, latest last.
fn codex_titles(home: &Path) -> HashMap<String, String> {
    let mut titles = HashMap::new();
    let Ok(text) = std::fs::read_to_string(home.join("session_index.jsonl")) else {
        return titles;
    };
    for line in text.lines() {
        if let (Some(id), Some(name)) = (string_after(line, "\"id\""), string_after(line, "\"thread_name\"")) {
            titles.insert(id, name);
        }
    }
    titles
}

// ------------------------------------------------------------------- sessions

/// A Claude session that is running now, from `<config>/sessions/<pid>.json`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveSession {
    pub pid: i32,
    pub session_id: String,
    pub cwd: PathBuf,
    pub name: Option<String>,
    pub status: Option<String>,
}

fn alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // Signal 0 checks for the process without touching it; EPERM means it
    // exists but belongs to someone else.
    let result = unsafe { libc::kill(pid, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// The sessions still running under a Claude config directory.
pub fn live_sessions(config_dir: &Path) -> Vec<LiveSession> {
    let Ok(entries) = std::fs::read_dir(config_dir.join("sessions")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter_map(|p| serde_json::from_str::<LiveSession>(&std::fs::read_to_string(p).ok()?).ok())
        .filter(|s| alive(s.pid))
        .collect()
}

struct Session {
    target: usize,
    id: String,
    /// Where it started, which is where resuming it works.
    start: Option<PathBuf>,
    cwds: Vec<PathBuf>,
    title: Option<String>,
    last_active: i64,
    live: Option<LiveSession>,
}

fn resume_command(target: &Target, id: &str) -> String {
    match target.provider {
        Provider::Claude => format!("{} --resume {id}", target.command),
        Provider::Codex => format!("{} resume {id}", target.command),
    }
}

/// `/tmp` and `/private/tmp` are the same place, and git reports the latter.
fn canonical(path: &Path, memo: &mut HashMap<PathBuf, Option<PathBuf>>) -> Option<PathBuf> {
    memo.entry(path.to_path_buf())
        .or_insert_with(|| std::fs::canonicalize(path).ok())
        .clone()
}

fn sessions(targets: &[Target], cache: &mut Cache, memo: &mut HashMap<PathBuf, Option<PathBuf>>) -> Vec<Session> {
    let mut fresh = BTreeMap::new();
    let mut by_key: BTreeMap<(usize, String), Session> = BTreeMap::new();

    for (index, target) in targets.iter().enumerate() {
        let files = match target.provider {
            Provider::Claude => claude::transcript_files(&target.dir),
            Provider::Codex => codex::rollout_files(&target.dir),
        };
        let codex_titles = match target.provider {
            Provider::Codex => codex_titles(&target.dir),
            Provider::Claude => HashMap::new(),
        };
        for path in files {
            let Some(facts) = facts_of(target.provider, &path, cache.files.get(&path)) else {
                continue;
            };
            if let Some(id) = facts.session.clone() {
                let session = by_key.entry((index, id.clone())).or_insert_with(|| Session {
                    target: index,
                    id: id.clone(),
                    start: None,
                    cwds: Vec::new(),
                    title: None,
                    last_active: 0,
                    live: None,
                });
                session.last_active = session.last_active.max((facts.mtime_ns / 1_000_000_000) as i64);
                // The parent's own transcript names the session, not a subagent's.
                let own = path.file_stem().is_some_and(|s| s.to_string_lossy() == id);
                if own || session.title.is_none() {
                    let title = facts.custom_title.clone().or(facts.title.clone()).or_else(|| codex_titles.get(&id).cloned());
                    if title.is_some() {
                        session.title = title;
                    }
                }
                // The session's own transcript says where it started.
                if own {
                    if let Some(first) = facts.cwds.first() {
                        session.start = Some(PathBuf::from(first));
                    }
                }
                for cwd in &facts.cwds {
                    let cwd = PathBuf::from(cwd);
                    if !session.cwds.contains(&cwd) {
                        session.cwds.push(cwd);
                    }
                }
            }
            fresh.insert(path, facts);
        }

        if target.provider == Provider::Claude {
            for live in live_sessions(&target.dir) {
                let session = by_key.entry((index, live.session_id.clone())).or_insert_with(|| Session {
                    target: index,
                    id: live.session_id.clone(),
                    start: Some(live.cwd.clone()),
                    cwds: vec![live.cwd.clone()],
                    title: None,
                    last_active: now(),
                    live: None,
                });
                if !session.cwds.contains(&live.cwd) {
                    session.cwds.push(live.cwd.clone());
                }
                session.last_active = now();
                session.live = Some(live);
            }
        }
    }
    cache.files = fresh;

    let mut all: Vec<Session> = by_key.into_values().collect();
    for session in &mut all {
        let originals = std::mem::take(&mut session.cwds);
        let mut canon: Vec<PathBuf> = Vec::new();
        for cwd in &originals {
            if let Some(c) = canonical(cwd, memo) {
                if !canon.contains(&c) {
                    canon.push(c);
                }
            }
        }
        session.cwds = canon;
        if session.start.is_none() {
            session.start = originals.first().cloned();
        }
    }
    all
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ------------------------------------------------------------------------ git

/// The repository's shared git directory for anything inside a working tree.
fn common_dir(path: &Path) -> Option<PathBuf> {
    let mut dir = Some(path);
    while let Some(current) = dir {
        let dot_git = current.join(".git");
        if dot_git.is_dir() {
            return Some(dot_git);
        }
        if dot_git.is_file() {
            // A worktree's `.git` is a file naming its private git dir,
            // whose `commondir` leads back to the shared one.
            let text = std::fs::read_to_string(&dot_git).ok()?;
            let private = current.join(text.trim().strip_prefix("gitdir:")?.trim());
            let common = std::fs::read_to_string(private.join("commondir")).ok()?;
            return std::fs::canonicalize(private.join(common.trim())).ok();
        }
        dir = current.parent();
    }
    None
}

fn git(common: &Path, args: &[&str]) -> Result<String, WorktreeError> {
    let output = Command::new("git")
        .arg("--git-dir")
        .arg(common)
        .args(args)
        .output()
        .map_err(|e| WorktreeError::Git(e.to_string()))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(WorktreeError::Git(String::from_utf8_lossy(&output.stderr).trim().to_string()))
    }
}

/// `git worktree list --porcelain`, the main working tree first.
fn list(common: &Path) -> Vec<Worktree> {
    let Ok(text) = git(common, &["worktree", "list", "--porcelain"]) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for block in text.split("\n\n") {
        let mut tree = Worktree {
            path: PathBuf::new(),
            branch: None,
            head: None,
            main: found.is_empty(),
            locked: false,
            prunable: false,
            sessions: Vec::new(),
            last_active: None,
            size_bytes: None,
            changes: None,
        };
        let mut bare = false;
        for line in block.lines() {
            let (key, value) = line.split_once(' ').unwrap_or((line, ""));
            match key {
                "worktree" => tree.path = PathBuf::from(value),
                "HEAD" => tree.head = Some(value.to_string()),
                "branch" => tree.branch = Some(value.trim_start_matches("refs/heads/").to_string()),
                "locked" => tree.locked = true,
                "prunable" => tree.prunable = true,
                "bare" => bare = true,
                _ => {}
            }
        }
        if !tree.path.as_os_str().is_empty() && !bare {
            found.push(tree);
        }
    }
    found
}

// --------------------------------------------------------------------- report

/// Every repository the accounts' sessions worked in that has worktrees,
/// with the sessions that worked in each.
pub fn scan(targets: &[Target], cache_file: Option<&Path>) -> Report {
    let mut cache: Cache = cache_file
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .filter(|c: &Cache| c.version == CACHE_VERSION)
        .unwrap_or_default();
    cache.version = CACHE_VERSION;

    let mut memo = HashMap::new();
    let sessions = sessions(targets, &mut cache, &mut memo);

    if let Some(path) = cache_file {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string(&cache) {
            let _ = std::fs::write(path, text);
        }
    }

    report(targets, &sessions)
}

fn report(targets: &[Target], sessions: &[Session]) -> Report {
    let mut commons: BTreeSet<PathBuf> = BTreeSet::new();
    let mut seen_dirs: HashMap<PathBuf, ()> = HashMap::new();
    for session in sessions {
        for cwd in &session.cwds {
            if seen_dirs.insert(cwd.clone(), ()).is_none() {
                if let Some(common) = common_dir(cwd) {
                    commons.insert(common);
                }
            }
        }
    }

    let ids: HashMap<&str, ()> = sessions.iter().map(|s| (s.id.as_str(), ())).collect();
    let mut repos = Vec::new();
    for common in commons {
        let mut trees = list(&common);
        if trees.len() < 2 {
            continue;
        }
        for (index, session) in sessions.iter().enumerate() {
            let mut homes: BTreeSet<usize> = BTreeSet::new();
            for cwd in &session.cwds {
                // The deepest tree holding the directory: worktrees often
                // live inside the main one.
                let deepest = trees
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| cwd.starts_with(&t.path))
                    .max_by_key(|(_, t)| t.path.components().count());
                if let Some((at, _)) = deepest {
                    homes.insert(at);
                }
            }
            for at in homes {
                trees[at].sessions.push(session_ref(targets, sessions, index));
            }
        }
        // A worktree made in a session's scratchpad carries its id.
        for tree in &mut trees {
            let path = tree.path.to_string_lossy().into_owned();
            let Some(owner) = path.split('/').collect::<Vec<_>>().windows(2).find_map(|w| {
                (w[1] == "scratchpad" && ids.contains_key(w[0])).then(|| w[0].to_string())
            }) else {
                continue;
            };
            for (index, session) in sessions.iter().enumerate() {
                if session.id == owner && !tree.sessions.iter().any(|s| s.id == owner) {
                    tree.sessions.push(session_ref(targets, sessions, index));
                }
            }
        }
        for tree in &mut trees {
            tree.sessions.sort_by(|a, b| b.live.cmp(&a.live).then(b.last_active.cmp(&a.last_active)));
            tree.last_active = tree.sessions.iter().map(|s| s.last_active).max();
        }
        let main = trees.remove(0);
        trees.sort_by(|a, b| b.last_active.cmp(&a.last_active).then(a.path.cmp(&b.path)));
        trees.insert(0, main);
        let path = trees[0].path.clone();
        repos.push(Repo {
            name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            path,
            worktrees: trees,
        });
    }
    repos.sort_by_key(|r| std::cmp::Reverse(r.worktrees.iter().filter_map(|t| t.last_active).max()));
    Report { repos }
}

fn session_ref(targets: &[Target], sessions: &[Session], index: usize) -> SessionRef {
    let session = &sessions[index];
    let target = &targets[session.target];
    SessionRef {
        account: target.profile.clone().unwrap_or_else(|| format!("@{}", target.provider.as_str())),
        profile: target.profile.clone(),
        provider: target.provider,
        id: session.id.clone(),
        title: session.title.clone().or_else(|| session.live.as_ref().and_then(|l| l.name.clone())),
        live: session.live.is_some(),
        status: session.live.as_ref().and_then(|l| l.status.clone()),
        last_active: session.last_active,
        resume_dir: session.start.clone(),
        resume: resume_command(target, &session.id),
    }
}

/// Measure each added worktree: its size and its uncommitted files. Both
/// mean walking the tree, so they are separate from the listing and run in
/// parallel.
pub fn fill_details(report: &mut Report) {
    let trees: Vec<&mut Worktree> = report
        .repos
        .iter_mut()
        .flat_map(|r| r.worktrees.iter_mut())
        .filter(|t| !t.main && !t.prunable)
        .collect();
    // A few at a time: each one walks a whole checkout, `node_modules` and all.
    let queue = std::sync::Mutex::new(trees);
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| loop {
                let Some(tree) = queue.lock().unwrap().pop() else {
                    break;
                };
                tree.size_bytes = size_of(&tree.path);
                tree.changes = changes_in(&tree.path);
            });
        }
    });
}

fn size_of(path: &Path) -> Option<u64> {
    let output = Command::new("du").arg("-sk").arg(path).output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.split_whitespace().next()?.parse::<u64>().ok().map(|kb| kb * 1024)
}

fn changes_in(path: &Path) -> Option<usize> {
    // A repository's own config can name an fsmonitor program that `status`
    // would run; measuring must never execute anything a repo asks for.
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["-c", "core.fsmonitor=false", "--no-optional-locks", "status", "--porcelain"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).lines().count())
}

// --------------------------------------------------------------------- remove

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Removed {
    pub path: PathBuf,
    pub repo: PathBuf,
    /// The branch it had checked out, which is kept.
    pub branch: Option<String>,
}

/// Remove an added worktree. Refuses the main working tree always, and,
/// unless `force`, one with changes, a lock, or a session still running in it.
/// `repo` locates the repository when the worktree's directory is already gone.
pub fn remove(path: &Path, repo: Option<&Path>, targets: &[Target], force: bool) -> Result<Removed, WorktreeError> {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let common = common_dir(&path)
        .or_else(|| repo.and_then(common_dir))
        .ok_or_else(|| WorktreeError::NotAWorktree(path.clone()))?;
    let trees = list(&common);
    let main = trees.first().map(|t| t.path.clone()).ok_or_else(|| WorktreeError::NotAWorktree(path.clone()))?;
    let tree = trees.iter().find(|t| t.path == path).ok_or_else(|| WorktreeError::NotAWorktree(path.clone()))?;
    if tree.main {
        return Err(WorktreeError::MainWorktree(path));
    }

    if !force {
        for target in targets.iter().filter(|t| t.provider == Provider::Claude) {
            for live in live_sessions(&target.dir) {
                let cwd = std::fs::canonicalize(&live.cwd).unwrap_or(live.cwd.clone());
                if cwd.starts_with(&path) {
                    let account = target.profile.clone().unwrap_or_else(|| "@claude".into());
                    let name = live.name.unwrap_or(live.session_id);
                    return Err(WorktreeError::InUse { path, session: format!("{name} ({account})") });
                }
            }
        }
    }

    let result = if tree.prunable {
        git(&common, &["worktree", "prune"]).map(|_| ())
    } else {
        let target = path.to_string_lossy();
        let mut args = vec!["worktree", "remove"];
        if force {
            // Twice overrides a lock as well as changes.
            args.extend(["--force", "--force"]);
        }
        args.push(&target);
        // `worktree remove` runs from inside the main tree, not via --git-dir.
        let output = Command::new("git")
            .arg("-C")
            .arg(&main)
            .args(&args)
            .output()
            .map_err(|e| WorktreeError::Git(e.to_string()))?;
        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(if stderr.contains("modified or untracked") {
                WorktreeError::Dirty(path.clone())
            } else if stderr.contains("locked") {
                WorktreeError::Locked(path.clone())
            } else {
                WorktreeError::Git(stderr)
            })
        }
    };
    result.map(|_| Removed { path, repo: main, branch: tree.branch.clone() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_json_string_after_a_key() {
        let line = r#"{"cwd":"/a/b \"c\"","x":1}"#;
        assert_eq!(string_after(line, "\"cwd\""), Some("/a/b \"c\"".into()));
        assert_eq!(string_after(line, "\"nope\""), None);
    }

    #[test]
    fn finds_every_cwd_in_a_line() {
        let line = r#"{"cwd":"/one","payload":{"cwd":"/two"}}"#;
        assert_eq!(strings_after(line, "\"cwd\""), vec!["/one", "/two"]);
    }

    #[test]
    fn a_subagent_transcript_belongs_to_its_parent() {
        let path = Path::new("/c/projects/-x/abc/subagents/agent-1.jsonl");
        assert_eq!(claude_session_of(path), Some("abc".into()));
        assert_eq!(claude_session_of(Path::new("/c/projects/-x/abc.jsonl")), Some("abc".into()));
    }

    #[test]
    fn a_codex_rollout_names_its_session() {
        let path = Path::new("/h/sessions/rollout-2026-10-12T06-29-52-01a12ecd-a9f6-70a1-881e-56b962785752.jsonl");
        assert_eq!(codex_session_of(path), Some("01a12ecd-a9f6-70a1-881e-56b962785752".into()));
    }
}
