//! The typed contract between the app's Rust side and its UI.
//!
//! These are views, not core types: the UI gets exactly what it renders, with
//! paths already stringified, so it never reimplements a decision.

use cpx_core::binding::{self, Bindings};
use cpx_core::config::Config;
use cpx_core::credentials::{self, CredentialSource};
use cpx_core::doctor::{Check, Severity};
use cpx_core::layout::Layout;
use cpx_core::plan::{Plan, Risk};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileRow {
    pub name: String,
    /// True when the config directory is one the user already had, managed in
    /// place rather than created under the cpx root.
    pub adopted: bool,
    pub description: String,
    pub color: Option<String>,
    pub command: String,
    pub model: Option<String>,
    pub directory: String,
    /// Whether `apply` has built this profile yet.
    pub applied: bool,
    pub signed_in: bool,
    pub account: Option<String>,
    /// "keychain", "file", or "none".
    pub credential_source: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceRow {
    pub resource: String,
    pub mode: String,
    pub is_directory: bool,
    /// Only JSON resources can use `merge`.
    pub supports_merge: bool,
    pub has_patch: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileDetail {
    pub row: ProfileRow,
    pub add_dirs: Vec<String>,
    pub env: Vec<(String, String)>,
    pub resources: Vec<ResourceRow>,
    pub keychain_service: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanLine {
    /// "safe" | "generated" | "foreign"
    pub risk: String,
    pub verb: String,
    pub target: String,
    pub description: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanView {
    pub lines: Vec<PlanLine>,
    pub notes: Vec<String>,
    pub requires_force: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingRow {
    pub path: String,
    pub profile: String,
    pub color: Option<String>,
    /// "healthy" | "directoryMissing" | "profileMissing" | "blockAbsent"
    /// | "blockEdited" | "notAllowed"
    pub health: String,
    pub healthy: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckView {
    pub name: String,
    /// "ok" | "warning" | "error"
    pub severity: String,
    pub detail: String,
    pub remedy: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatuslineView {
    /// Whether a cpx badge is installed for this target.
    pub badge: bool,
    /// The statusline the badge delegates to, or what is configured now.
    pub delegate: Option<String>,
    /// Where the generated script would live.
    pub script: String,
    /// True when the change is recorded in config.toml and needs an apply.
    pub needs_apply: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultSessionView {
    pub dir: String,
    pub account: Option<String>,
    pub signed_in: bool,
    /// True when this is also the directory profiles inherit from.
    pub is_source: bool,
    /// Set when a profile already manages this directory, in which case it
    /// needs no separate mention.
    pub claimed_by: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptionRow {
    pub name: String,
    pub dir: String,
    /// What the directory already holds, and will keep untouched.
    pub keeps: Vec<String>,
    /// True when a profile of that name is already configured.
    pub taken: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyView {
    pub performed: usize,
    pub backups: Vec<(String, String)>,
}

pub fn risk_name(risk: Risk) -> &'static str {
    match risk {
        Risk::Safe => "safe",
        Risk::OverwritesGenerated => "generated",
        Risk::OverwritesForeign => "foreign",
    }
}

pub fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Ok => "ok",
        Severity::Warning => "warning",
        Severity::Error => "error",
    }
}

/// The leading word of a plan line: what is about to happen.
pub fn action_verb(action: &cpx_core::plan::Action) -> &'static str {
    use cpx_core::plan::Action::*;
    match action {
        CreateDir { .. } => "create",
        Symlink { .. } => "link",
        CopyFile { .. } | CopyTree { .. } => "copy",
        WriteFile { .. } => "write",
        Backup { .. } => "back up",
        RemoveGenerated { .. } => "remove",
        WriteEnvrcBlock { .. } => "bind",
        RemoveEnvrcBlock { .. } => "unbind",
        GitInfoExclude { .. } => "exclude",
        RunDirenvAllow { .. } => "allow",
    }
}

pub fn profile_row(config: &Config, layout: &Layout, name: &str) -> ProfileRow {
    let profile = &config.profiles[name];
    let dir = config.config_dir(layout, name);
    let status = credentials::status(&dir, &layout.home);

    ProfileRow {
        name: name.to_string(),
        adopted: profile.dir.is_some(),
        description: profile.description.clone(),
        color: profile.color.clone(),
        command: format!("{}{name}", config.wrapper_prefix),
        model: profile.model.clone(),
        directory: dir.display().to_string(),
        applied: dir.is_dir(),
        signed_in: status.authenticated,
        account: status.account,
        credential_source: match status.source {
            CredentialSource::Keychain => "keychain",
            CredentialSource::File => "file",
            CredentialSource::None => "none",
        }
        .to_string(),
    }
}

pub fn profile_detail(config: &Config, layout: &Layout, name: &str) -> ProfileDetail {
    let profile = &config.profiles[name];
    let dir = config.config_dir(layout, name);

    ProfileDetail {
        row: profile_row(config, layout, name),
        add_dirs: profile
            .add_dirs
            .iter()
            .map(|d| d.display().to_string())
            .collect(),
        env: profile
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        resources: profile
            .resources
            .iter()
            .map(|(key, spec)| ResourceRow {
                resource: key.config_name().to_string(),
                mode: spec.mode.as_str().to_string(),
                is_directory: key.is_dir(),
                supports_merge: key.is_json(),
                has_patch: spec.patch.is_some(),
            })
            .collect(),
        keychain_service: credentials::keychain_service_for(&dir, &layout.home),
    }
}

pub fn plan_view(plan: &Plan) -> PlanView {
    PlanView {
        lines: plan
            .actions
            .iter()
            .map(|planned| PlanLine {
                risk: risk_name(planned.risk).to_string(),
                verb: action_verb(&planned.action).to_string(),
                target: planned.action.target().display().to_string(),
                description: planned.description.clone(),
            })
            .collect(),
        notes: plan.notes.clone(),
        requires_force: plan.requires_force(),
    }
}

pub fn binding_rows(bindings: &Bindings, config: &Config) -> Vec<BindingRow> {
    bindings
        .entries
        .iter()
        .map(|entry| {
            let health = binding::health(entry, config);
            BindingRow {
                path: entry.path.display().to_string(),
                profile: entry.profile.clone(),
                color: config
                    .profiles
                    .get(&entry.profile)
                    .and_then(|p| p.color.clone()),
                health: match health {
                    binding::BindingHealth::Healthy => "healthy",
                    binding::BindingHealth::DirectoryMissing => "directoryMissing",
                    binding::BindingHealth::ProfileMissing => "profileMissing",
                    binding::BindingHealth::BlockAbsent => "blockAbsent",
                    binding::BindingHealth::BlockEdited => "blockEdited",
                    binding::BindingHealth::NotAllowed => "notAllowed",
                }
                .to_string(),
                healthy: health == binding::BindingHealth::Healthy,
            }
        })
        .collect()
}

pub fn default_session_view(
    session: &cpx_core::default_session::DefaultSession,
) -> DefaultSessionView {
    DefaultSessionView {
        dir: session.dir.display().to_string(),
        account: session.account.clone(),
        signed_in: session.signed_in,
        is_source: session.is_source,
        claimed_by: session.claimed_by.clone(),
    }
}

pub fn adoption_rows(found: &[cpx_core::adopt::Adoption], config: &Config) -> Vec<AdoptionRow> {
    found
        .iter()
        .map(|adoption| AdoptionRow {
            name: adoption.name.clone(),
            dir: adoption.dir.display().to_string(),
            keeps: adoption.found.clone(),
            taken: config.profiles.contains_key(&adoption.name),
        })
        .collect()
}

pub fn check_views(checks: &[Check]) -> Vec<CheckView> {
    checks
        .iter()
        .map(|check| CheckView {
            name: check.name.clone(),
            severity: severity_name(check.severity).to_string(),
            detail: check.detail.clone(),
            remedy: check.remedy.clone(),
        })
        .collect()
}
