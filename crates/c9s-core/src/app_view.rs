//! The tabs of an application, as lines of toned text: the TUI colours the
//! tones, `c` copies the text.

use crate::apps::{credential_expiry, permission_rows, AppRow, Consent};
use crate::policy::{expiry_label, Expiry, EXPIRY_WARNING_DAYS};
use crate::state::{AppState, AppTab};

/// How a piece of text should look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Heading,
    Label,
    Text,
    Muted,
    Accent,
    Good,
    Warn,
    Bad,
}

/// A line: pieces of text, each with its tone.
pub type Line = Vec<(String, Tone)>;

fn heading(text: &str) -> Line {
    vec![(text.to_string(), Tone::Heading)]
}

fn muted(text: &str) -> Line {
    vec![(text.to_string(), Tone::Muted)]
}

/// "label: value" rows of a section, labels padded to line up.
fn rows(out: &mut Vec<Line>, title: &str, rows: &[(&str, String)]) {
    if !out.is_empty() {
        out.push(vec![]);
    }
    out.push(heading(title));
    let width = rows.iter().map(|(l, _)| l.len()).max().unwrap_or(0);
    for (label, value) in rows {
        out.push(vec![
            (format!("  {label:<width$}  "), Tone::Label),
            (value.clone(), Tone::Text),
        ]);
    }
}

/// A list under a heading, or "(none)".
fn list(out: &mut Vec<Line>, title: &str, items: Vec<Line>) {
    if !out.is_empty() {
        out.push(vec![]);
    }
    out.push(heading(title));
    if items.is_empty() {
        out.push(muted("  (none)"));
    }
    for mut item in items {
        item.insert(0, ("  ".into(), Tone::Text));
        out.push(item);
    }
}

fn pad(text: &str, width: usize) -> String {
    let shown: String = text.chars().take(width.saturating_sub(1)).collect();
    format!("{shown:<width$}")
}

fn yes_no(b: bool) -> String {
    if b { "yes" } else { "no" }.into()
}

fn expiry_tone(e: Expiry) -> Tone {
    match e {
        Expiry::Expired(_) => Tone::Bad,
        Expiry::In(d) if d <= EXPIRY_WARNING_DAYS => Tone::Warn,
        Expiry::In(_) => Tone::Good,
        Expiry::Never => Tone::Muted,
    }
}

/// The lines of the open application's current tab.
pub fn app_lines(state: &AppState) -> Vec<Line> {
    let Some(row) = state.app_open.as_ref() else {
        return vec![];
    };
    let detail = state.open_app_detail();
    let loading = || vec![muted("Loading from Microsoft Graph…")];
    let mut out: Vec<Line> = Vec::new();
    match state.app_tab {
        AppTab::Overview => overview(&mut out, row, state),
        AppTab::Permissions => {
            let Some(detail) = detail else {
                return loading();
            };
            let perms = permission_rows(row.app.as_ref(), detail);
            if perms.is_empty() {
                return vec![muted("No API permissions requested or granted.")];
            }
            out.push(vec![(
                format!(
                    "{}{}{}{}Description",
                    pad("Status", 40),
                    pad("Type", 13),
                    pad("API", 26),
                    pad("Permission", 34)
                ),
                Tone::Heading,
            )]);
            for p in perms {
                let tone = match (&p.consent, p.requested) {
                    (Consent::NotGranted, _) => Tone::Bad,
                    (_, false) => Tone::Warn,
                    (Consent::Users(_), _) => Tone::Warn,
                    (Consent::Admin, true) => Tone::Good,
                };
                out.push(vec![
                    (pad(&p.status(), 40), tone),
                    (pad(p.kind, 13), Tone::Accent),
                    (pad(&p.api, 26), Tone::Muted),
                    (pad(&p.permission, 34), Tone::Text),
                    (p.description, Tone::Muted),
                ]);
            }
        }
        AppTab::ExposedApi => {
            let Some(app) = row.app.as_ref() else {
                return vec![muted("No app registration in this tenant: what it exposes is defined in its home tenant.")];
            };
            list(
                &mut out,
                "Identifier URIs",
                app.identifier_uris
                    .iter()
                    .map(|u| vec![(u.clone(), Tone::Text)])
                    .collect(),
            );
            let perm_line = |p: &c9s_azure::apps::ApiPermission| {
                let mut line = vec![
                    (pad(&p.value, 34), Tone::Text),
                    (pad(&p.audience, 22), Tone::Accent),
                    (p.display.clone(), Tone::Muted),
                ];
                if !p.enabled {
                    line.push(("  (disabled)".into(), Tone::Warn));
                }
                line
            };
            list(
                &mut out,
                "Delegated permissions (scopes) · who can consent",
                app.scopes.iter().map(perm_line).collect(),
            );
            list(
                &mut out,
                "App roles · who can be given them",
                app.roles.iter().map(perm_line).collect(),
            );
        }
        AppTab::Owners => {
            let Some(detail) = detail else {
                return loading();
            };
            let owner = |p: &c9s_azure::directory::Principal| {
                vec![
                    (pad(&p.kind, 18), Tone::Accent),
                    (pad(&p.display_name, 34), Tone::Text),
                    (p.detail.clone(), Tone::Muted),
                ]
            };
            if row.app.is_some() {
                list(
                    &mut out,
                    "App registration owners",
                    detail.app_owners.iter().map(owner).collect(),
                );
            }
            if row.sp.is_some() {
                list(
                    &mut out,
                    "Enterprise application owners",
                    detail.sp_owners.iter().map(owner).collect(),
                );
            }
        }
        AppTab::Credentials => {
            let now = chrono::Utc::now().timestamp();
            let mut creds: Vec<(Expiry, &str, &crate::apps::Credential)> = row
                .credentials()
                .into_iter()
                .map(|(on, c)| (credential_expiry(c, now), on, c))
                .collect();
            creds.sort_by_key(|c| c.0);
            if creds.is_empty() {
                return vec![muted("No secrets or certificates.")];
            }
            out.push(vec![(
                format!(
                    "{}{}{}{}Expires on",
                    pad("Expiry", 24),
                    pad("Kind", 13),
                    pad("On", 16),
                    pad("Name", 34)
                ),
                Tone::Heading,
            )]);
            for (e, on, c) in creds {
                out.push(vec![
                    (pad(&expiry_label(e), 24), expiry_tone(e)),
                    (pad(&c.kind, 13), Tone::Accent),
                    (pad(on, 16), Tone::Muted),
                    (pad(&c.name, 34), Tone::Text),
                    (c.end.clone(), Tone::Muted),
                ]);
            }
        }
        AppTab::Assigned => {
            let Some(detail) = detail else {
                return loading();
            };
            if detail.assigned.is_empty() {
                let required = row.sp.as_ref().is_some_and(|s| s.assignment_required);
                return vec![muted(if required {
                    "Nobody is assigned, and assignment is required: nobody can sign in."
                } else {
                    "Nobody is assigned (assignment isn't required: everyone in the tenant can sign in)."
                })];
            }
            let role_name = |id: &str| -> String {
                if id.chars().all(|c| c == '0' || c == '-') {
                    return "default access".into();
                }
                row.app
                    .as_ref()
                    .and_then(|a| a.roles.iter().find(|r| r.id == id))
                    .map_or(id.to_string(), |r| r.value.clone())
            };
            for a in &detail.assigned {
                out.push(vec![
                    (pad(&a.principal_type, 18), Tone::Accent),
                    (pad(&a.principal_name, 40), Tone::Text),
                    (role_name(&a.app_role_id), Tone::Muted),
                ]);
            }
        }
        AppTab::Json => {
            let Some(detail) = detail else {
                return loading();
            };
            for (title, doc) in [
                ("App registration", &detail.application_json),
                ("Enterprise application", &detail.service_principal_json),
            ] {
                if doc.is_null() {
                    continue;
                }
                if !out.is_empty() {
                    out.push(vec![]);
                }
                out.push(heading(title));
                let text = serde_json::to_string_pretty(doc).unwrap_or_default();
                out.extend(text.lines().map(|l| vec![(l.to_string(), Tone::Text)]));
            }
        }
    }
    out
}

fn overview(out: &mut Vec<Line>, row: &AppRow, state: &AppState) {
    let mut main = vec![
        ("Name", row.name.clone()),
        ("Kind", row.kind.label().to_string()),
        ("App ID", row.app_id.clone()),
    ];
    match &row.app {
        Some(app) => {
            main.push(("Registration object ID", app.id.clone()));
            main.push(("Sign-in audience", app.sign_in_audience.clone()));
            main.push(("Created", app.created.clone()));
        }
        None => main.push(("Registration", "none in this tenant".into())),
    }
    rows(out, "Application", &main);
    if let Some(mi) = row.managed_identity() {
        managed_identity(out, &mi, state);
    }
    match &row.sp {
        Some(sp) => {
            let publisher = if sp.is_microsoft() {
                "Microsoft".to_string()
            } else if state
                .scope_tree
                .as_ref()
                .is_some_and(|t| t.root.eq_ignore_ascii_case(&sp.owner_tenant))
                || row.app.is_some()
            {
                "this tenant".to_string()
            } else {
                sp.owner_tenant.clone()
            };
            rows(
                out,
                "Enterprise application",
                &[
                    ("Object ID", sp.id.clone()),
                    ("Type", sp.sp_type.clone()),
                    ("Enabled", yes_no(sp.enabled)),
                    ("Assignment required", yes_no(sp.assignment_required)),
                    ("Publisher tenant", publisher),
                ],
            );
        }
        None => rows(
            out,
            "Enterprise application",
            &[("Object ID", "none yet (nobody has consented)".into())],
        ),
    }
    if let Some(app) = &row.app {
        list(
            out,
            "Redirect URIs",
            app.redirect_uris
                .iter()
                .map(|u| vec![(u.clone(), Tone::Text)])
                .collect(),
        );
    }
    if let Some((e, c)) = &row.soonest {
        rows(
            out,
            "Credentials",
            &[(
                "Soonest",
                format!("{} ({} {})", expiry_label(*e), c.kind, c.name),
            )],
        );
    }
}

/// The parts of an ARM ID: subscription, resource group, resource type
/// (`Microsoft.Compute/virtualMachines`) and name.
fn arm_parts(id: &str) -> (String, String, String, String) {
    let parts: Vec<&str> = id.split('/').filter(|p| !p.is_empty()).collect();
    let after = |key: &str| {
        parts
            .windows(2)
            .find(|w| w[0].eq_ignore_ascii_case(key))
            .map(|w| w[1].to_string())
            .unwrap_or_default()
    };
    let resource_type = parts
        .iter()
        .rposition(|p| p.eq_ignore_ascii_case("providers"))
        .and_then(|i| Some(format!("{}/{}", parts.get(i + 1)?, parts.get(i + 2)?)))
        .unwrap_or_default();
    let name = parts.last().map(|n| n.to_string()).unwrap_or_default();
    (
        after("subscriptions"),
        after("resourceGroups"),
        resource_type,
        name,
    )
}

/// The Azure Portal page of a managed identity's resource, in the
/// resource's own tenant when its subscription is known.
fn identity_portal_url(resource_id: &str, state: &AppState) -> String {
    let (sub, ..) = arm_parts(resource_id);
    let tenant = state
        .subscriptions
        .iter()
        .find(|s| s.id.eq_ignore_ascii_case(&sub))
        .map(|s| s.tenant_id.clone())
        .filter(|t| !t.is_empty())
        .or_else(|| state.tenant.clone())
        .or_else(|| state.portal_tenant());
    crate::detail::portal_url(resource_id, tenant.as_deref())
}

/// For a managed identity: the Portal page of its Azure resource (the
/// user-assigned identity, or the resource a system-assigned one belongs
/// to), which `o` opens instead of the enterprise application.
pub fn managed_identity_url(row: &AppRow, state: &AppState) -> Option<String> {
    let id = row.managed_identity()?.resource_id?;
    Some(identity_portal_url(&id, state))
}

/// Where a managed identity lives in Azure: the user-assigned identity
/// resource, or the resource a system-assigned one belongs to, with its
/// Portal link (`o` opens it).
fn managed_identity(out: &mut Vec<Line>, mi: &crate::apps::ManagedIdentity, state: &AppState) {
    let kind = match mi.user_assigned {
        Some(true) => "User-assigned (an Azure resource of its own)",
        Some(false) => "System-assigned (part of the resource below)",
        None => "Unknown (Microsoft Graph doesn't say)",
    };
    let mut items = vec![("Type", kind.to_string())];
    match &mi.resource_id {
        Some(id) => {
            let (sub, rg, resource_type, name) = arm_parts(id);
            let known = state
                .subscriptions
                .iter()
                .find(|s| s.id.eq_ignore_ascii_case(&sub));
            let subscription = known.map(|s| format!("{} ({sub})", s.name)).unwrap_or(sub);
            let resource_label = if mi.user_assigned == Some(false) {
                "Assigned to"
            } else {
                "Resource"
            };
            items.push((resource_label, name));
            items.push(("Resource type", resource_type));
            items.push(("Subscription", subscription));
            items.push(("Resource group", rg));
            items.push(("Resource ID", id.clone()));
            items.push(("Portal (o)", identity_portal_url(id, state)));
        }
        None => items.push(("Resource", "not given by Microsoft Graph".into())),
    }
    rows(out, "Managed identity", &items);
}

/// Which page of an application a field belongs to, for `o`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// The app registration (or the app in general)
    Registration,
    EnterpriseApp,
    /// The managed identity's Azure resource
    Resource,
    Other,
}

/// A label and value on the Overview: what `j`/`k` select and `y`/`c`
/// copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// Its line in the Overview
    pub line: usize,
    pub label: String,
    pub value: String,
    pub section: Section,
}

/// The Overview's fields, in order: every "label  value" row, with the
/// section it sits in.
pub fn overview_fields(state: &AppState) -> Vec<Field> {
    let Some(row) = state.app_open.as_ref() else {
        return vec![];
    };
    let mut lines = Vec::new();
    overview(&mut lines, row, state);
    let mut section = Section::Other;
    let mut fields = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        match line.as_slice() {
            [(title, Tone::Heading)] => {
                section = match title.as_str() {
                    "Application" => Section::Registration,
                    "Enterprise application" => Section::EnterpriseApp,
                    "Managed identity" => Section::Resource,
                    _ => Section::Other,
                }
            }
            [(label, Tone::Label), (value, _), ..] if !value.trim().is_empty() => {
                fields.push(Field {
                    line: i,
                    label: label.trim().to_string(),
                    value: value.trim().to_string(),
                    section,
                })
            }
            [(indent, Tone::Text), rest @ ..] if indent == "  " && !rest.is_empty() => {
                // A list item (redirect URIs): its text is the value.
                let value: String = rest.iter().map(|(t, _)| t.as_str()).collect();
                fields.push(Field {
                    line: i,
                    label: String::new(),
                    value: value.trim().to_string(),
                    section,
                })
            }
            _ => {}
        }
    }
    fields
}

/// The field the Overview cursor starts on: the app ID, which `y` has
/// always copied.
pub fn default_field(fields: &[Field]) -> usize {
    fields.iter().position(|f| f.label == "App ID").unwrap_or(0)
}

/// The current tab as plain text, for `c`.
pub fn app_text(state: &AppState) -> Vec<String> {
    app_lines(state)
        .into_iter()
        .map(|l| {
            l.into_iter()
                .map(|(t, _)| t)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}
