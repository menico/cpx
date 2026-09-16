//! Human-readable output. Every command also has a `--json` form, which is
//! rendered from the same values.

use cpx_core::doctor::{Check, Severity};
use cpx_core::plan::{Action, Plan, Risk};
use serde_json::json;

pub fn risk_label(risk: Risk) -> &'static str {
    match risk {
        Risk::Safe => "     ",
        Risk::OverwritesGenerated => "  ~  ",
        Risk::OverwritesForeign => "  !  ",
    }
}

/// The name the app shows for a risk level.
pub fn risk_name(risk: Risk) -> &'static str {
    match risk {
        Risk::Safe => "safe",
        Risk::OverwritesGenerated => "generated",
        Risk::OverwritesForeign => "foreign",
    }
}

/// The leading word of a plan line: what is about to happen.
pub fn action_verb(action: &Action) -> &'static str {
    match action {
        Action::CreateDir { .. } => "create",
        Action::Symlink { .. } => "link",
        Action::CopyFile { .. } | Action::CopyTree { .. } => "copy",
        Action::WriteFile { .. } => "write",
        Action::Backup { .. } => "back up",
        Action::RemoveGenerated { .. } => "remove",
        Action::WriteEnvrcBlock { .. } => "bind",
        Action::RemoveEnvrcBlock { .. } => "unbind",
        Action::GitInfoExclude { .. } => "exclude",
        Action::RunDirenvAllow { .. } => "allow",
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

/// The plan as the app renders it: one typed line per action.
pub fn plan_json(plan: &Plan) -> serde_json::Value {
    json!({
        "actions": plan.actions.iter().map(|a| json!({
            "risk": risk_name(a.risk),
            "verb": action_verb(&a.action),
            "target": a.action.target(),
            "description": a.description,
            "profile": a.owner,
        })).collect::<Vec<_>>(),
        "pendingActions": plan.actions.len(),
        "notes": plan.notes,
        "requiresForce": plan.requires_force(),
    })
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
