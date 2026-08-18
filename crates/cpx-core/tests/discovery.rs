use cpx_core::discovery::*;
use cpx_core::layout::Layout;
use cpx_core::state::MARKER;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn executable(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn real_claude(dir: &Path) -> PathBuf {
    let p = dir.join("claude");
    executable(&p, "#!/bin/sh\necho real\n");
    p
}

fn path_of(dirs: &[&Path]) -> String {
    dirs.iter()
        .map(|d| d.display().to_string())
        .collect::<Vec<_>>()
        .join(":")
}

#[test]
fn the_first_claude_on_path_is_used() {
    let d = TempDir::new().unwrap();
    let bin = d.path().join("usr-bin");
    let expected = real_claude(&bin);
    assert_eq!(
        resolve_claude_binary(&path_of(&[&bin]), &Layout::new(d.path())),
        Some(expected)
    );
}

#[test]
fn a_generated_wrapper_earlier_on_path_is_skipped() {
    let d = TempDir::new().unwrap();
    let local = d.path().join(".local/bin");
    let usr = d.path().join("usr-bin");

    // A cpx wrapper named plainly `claude` would otherwise win the search.
    executable(
        &local.join("claude"),
        &format!("#!/usr/bin/env bash\n{MARKER}\nexec whatever\n"),
    );
    let expected = real_claude(&usr);

    assert_eq!(
        resolve_claude_binary(&path_of(&[&local, &usr]), &Layout::new(d.path())),
        Some(expected),
        "resolving to our own wrapper is the recursion we are avoiding"
    );
}

#[test]
fn a_profile_shim_earlier_on_path_is_skipped() {
    let d = TempDir::new().unwrap();
    let layout = Layout::new(d.path());
    let shim_dir = layout.profile_bin_dir("work");
    executable(&shim_dir.join("claude"), "#!/bin/sh\necho shim\n");

    let usr = d.path().join("usr-bin");
    let expected = real_claude(&usr);

    assert_eq!(
        resolve_claude_binary(&path_of(&[&shim_dir, &usr]), &layout),
        Some(expected),
        "a bound directory puts the shim first on PATH; it is not the real binary"
    );
}

#[test]
fn a_non_executable_file_named_claude_is_not_the_binary() {
    let d = TempDir::new().unwrap();
    let bin = d.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("claude"), "just a note").unwrap();
    assert_eq!(
        resolve_claude_binary(&path_of(&[&bin]), &Layout::new(d.path())),
        None
    );
}

#[test]
fn nothing_on_path_yields_nothing() {
    let d = TempDir::new().unwrap();
    assert_eq!(
        resolve_claude_binary(&path_of(&[d.path()]), &Layout::new(d.path())),
        None
    );
}

#[test]
fn empty_path_entries_are_ignored_rather_than_treated_as_cwd() {
    let d = TempDir::new().unwrap();
    let bin = d.path().join("bin");
    let expected = real_claude(&bin);
    assert_eq!(
        resolve_claude_binary(&format!("::{}", bin.display()), &Layout::new(d.path())),
        Some(expected)
    );
}

#[test]
fn our_own_scripts_are_recognised_by_marker_or_location() {
    let d = TempDir::new().unwrap();
    let layout = Layout::new(d.path());

    let marked = d.path().join("marked");
    executable(&marked, &format!("#!/usr/bin/env bash\n{MARKER}\n"));
    assert!(is_cpx_script(&marked, &layout));

    let shim = layout.shim_path("work");
    executable(&shim, "#!/bin/sh\n");
    assert!(is_cpx_script(&shim, &layout));

    let stranger = d.path().join("stranger");
    executable(&stranger, "#!/bin/sh\necho hello\n");
    assert!(!is_cpx_script(&stranger, &layout));
}
