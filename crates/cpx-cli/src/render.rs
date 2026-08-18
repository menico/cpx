//! Human-readable output. Every command also has a `--json` form, which is
//! rendered from the same values.

use cpx_core::doctor::{Check, Severity};
use cpx_core::plan::{Action, Plan, Risk};

pub fn risk_label(risk: Risk) -> &'static str {
    match risk {
        Risk::Safe => "     ",
        Risk::OverwritesGenerated => "  ~  ",
        Risk::OverwritesForeign => "  !  ",
    }
}

pub fn action_summary(action: &Action) -> String {
    match action {
        Action::CreateDir { path } => format!("mkdir   {}", path.display()),
        Action::Symlink { link, target } => {
            format!("link    {} -> {}", link.display(), target.display())
        }
        Action::CopyFile { src, dst } => format!("copy    {} <- {}", dst.display(), src.display()),
        Action::CopyTree { src, dst } => format!("copy -r {} <- {}", dst.display(), src.display()),
        Action::WriteFile { path, .. } => format!("write   {}", path.display()),
        Action::Backup { path, to } => format!("backup  {} -> {}", path.display(), to.display()),
        Action::RemoveGenerated { path } => format!("remove  {}", path.display()),
        Action::WriteEnvrcBlock { envrc, .. } => format!("envrc   {}", envrc.display()),
        Action::RemoveEnvrcBlock { envrc } => format!("envrc   {} (remove block)", envrc.display()),
        Action::GitInfoExclude { repo, line } => {
            format!("exclude {} += {line}", repo.display())
        }
        Action::RunDirenvAllow { dir } => format!("direnv  allow {}", dir.display()),
    }
}

pub fn print_plan(plan: &Plan) {
    if plan.is_empty() {
        println!("Nothing to do — everything is already as configured.");
    } else {
        for planned in &plan.actions {
            println!("{}{}", risk_label(planned.risk), action_summary(&planned.action));
        }
    }
    for note in &plan.notes {
        println!("  note: {note}");
    }
    if plan.requires_force() {
        println!();
        println!("Lines marked ! would replace files cpx did not write.");
        println!("Re-run with --force to back each one up and continue.");
    }
}

pub fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Ok => "ok  ",
        Severity::Warning => "warn",
        Severity::Error => "FAIL",
    }
}

pub fn print_checks(checks: &[Check], verbose: bool) {
    for check in checks {
        if check.severity == Severity::Ok && !verbose {
            continue;
        }
        println!("{}  {}: {}", severity_label(check.severity), check.name, check.detail);
        if let Some(remedy) = &check.remedy {
            println!("        -> {remedy}");
        }
    }
}
