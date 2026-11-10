//! The current view as text, for `c` (copy what's on screen).
//!
//! Lists become tab-separated rows with a header, so they paste into a
//! spreadsheet; every row passing the current filters is included, not just
//! the ones that fit on screen. The detail view gives its current tab:
//! the Overview as "label: value" lines, tags as tab-separated pairs, or the
//! JSON document.

use crate::detail;
use crate::scope_tree::TreeRowKind;
use crate::state::{AppState, DetailTab, ViewMode};

/// What `c` copies in the current view: a short description (for the
/// status bar, e.g. "42 resources") and the text. `None` when there is
/// nothing to copy.
pub fn view_text(state: &AppState) -> Option<(String, String)> {
    let (what, lines) = match state.view {
        ViewMode::SubscriptionList => {
            let rows = state.filtered_subscriptions();
            let lines = rows.iter().map(|s| tsv(&[&s.name, &s.state, &s.id]));
            (
                count(rows.len(), "subscription"),
                with_header("name\tstate\tid", lines),
            )
        }
        ViewMode::ResourceGroupList => {
            let rows = state.filtered_resource_groups();
            let lines = rows
                .iter()
                .map(|r| tsv(&[&r.name, &r.location, &r.subscription_id, &r.id]));
            (
                count(rows.len(), "resource group"),
                with_header("name\tlocation\tsubscription\tid", lines),
            )
        }
        ViewMode::ResourceList => {
            let rows = state.filtered_resources();
            let lines = rows.iter().map(|r| {
                tsv(&[
                    &r.name,
                    &r.resource_type,
                    &r.location,
                    &r.resource_group,
                    &r.subscription_id,
                    &r.id,
                ])
            });
            (
                count(rows.len(), "resource"),
                with_header(
                    "name\ttype\tlocation\tresource group\tsubscription\tid",
                    lines,
                ),
            )
        }
        ViewMode::ResourceTypeSummary => {
            let rows = state.type_summary();
            let lines = rows
                .iter()
                .map(|(t, n)| tsv(&[&n.to_string(), &crate::resource_type::short_label(t), t]));
            (
                count(rows.len(), "resource type"),
                with_header("count\talias\ttype", lines),
            )
        }
        ViewMode::PolicyAssignments => {
            let rows = state.policy_rows();
            let lines = rows.iter().map(|r| {
                let (nc, total) = r.compliance.map_or((String::new(), String::new()), |c| {
                    (c.non_compliant.to_string(), c.total.to_string())
                });
                tsv(&[
                    &r.assignment.display_name,
                    if r.assignment.is_initiative() {
                        "initiative"
                    } else {
                        "policy"
                    },
                    &r.assignment.enforcement,
                    &nc,
                    &total,
                    &r.origin_label,
                    &r.assignment.id,
                ])
            });
            (
                count(rows.len(), "policy assignment"),
                with_header(
                    "assignment\tkind\tenforcement\tnon-compliant\tevaluated\tassigned\tid",
                    lines,
                ),
            )
        }
        ViewMode::PolicyNonCompliant => {
            let rows = state.non_compliant_rows();
            let lines = rows.iter().map(|r| {
                tsv(&[
                    r.state.resource_name(),
                    &r.state.resource_type,
                    &r.state.resource_group,
                    &r.state.subscription_id,
                    &r.policies.join(", "),
                    &r.state.resource_id,
                ])
            });
            (
                count(rows.len(), "non-compliant resource"),
                with_header(
                    "name\ttype\tresource group\tsubscription\tfailing policies\tid",
                    lines,
                ),
            )
        }
        ViewMode::AppList => {
            let rows = state.app_rows();
            let lines = rows.iter().map(|r| {
                let soonest = r
                    .soonest
                    .as_ref()
                    .map(|(e, c)| {
                        format!(
                            "{} ({} {})",
                            crate::policy::expiry_label(*e),
                            c.kind,
                            c.name
                        )
                    })
                    .unwrap_or_default();
                tsv(&[
                    &r.name,
                    r.kind_label(),
                    &r.app_id,
                    &soonest,
                    r.app.as_ref().map_or("", |a| a.id.as_str()),
                    r.sp.as_ref().map_or("", |s| s.id.as_str()),
                ])
            });
            (
                count(rows.len(), "application"),
                with_header(
                    "name\tkind\tapp id\tsoonest credential expiry\tregistration object id\tenterprise app object id",
                    lines,
                ),
            )
        }
        ViewMode::AppDetail => {
            let row = state.app_open.as_ref()?;
            (
                format!("{} of {}", state.app_tab.title(), row.name),
                crate::app_view::app_text(state),
            )
        }
        ViewMode::GroupList => {
            let rows = state.group_rows();
            let lines = rows.iter().map(|g| {
                tsv(&[
                    &g.display_name,
                    g.kind_label(),
                    if g.dynamic { "dynamic" } else { "assigned" },
                    if g.role_assignable { "yes" } else { "" },
                    &g.mail,
                    &g.description,
                    &g.id,
                ])
            });
            (
                count(rows.len(), "group"),
                with_header(
                    "group\tkind\tmembership\trole-assignable\tmail\tdescription\tid",
                    lines,
                ),
            )
        }
        ViewMode::GroupMembers | ViewMode::MemberOf => {
            let (rows, noun) = if state.view == ViewMode::GroupMembers {
                (state.member_rows()?, "member")
            } else {
                (state.member_of_rows()?, "group")
            };
            let lines = rows
                .iter()
                .map(|p| tsv(&[&p.display_name, &p.kind, &p.detail, &p.id]));
            (
                count(rows.len(), noun),
                with_header("name\tkind\tsign-in, app id or mail\tid", lines),
            )
        }
        ViewMode::RoleAssignments => {
            let rows = state.role_assignment_rows();
            let lines = rows.iter().map(|r| {
                let a = &r.assignment;
                let kind = match r.custom {
                    Some(true) => "custom",
                    Some(false) => "built-in",
                    None => "",
                };
                tsv(&[
                    &r.role_name,
                    kind,
                    &r.principal_name,
                    &r.principal_detail,
                    &a.principal_type,
                    &a.principal_id,
                    &r.scope_label,
                    &r.origin_label,
                    &a.scope,
                    &a.id,
                ])
            });
            (
                count(rows.len(), "role assignment"),
                with_header(
                    "role\tkind\tprincipal\tsign-in or app id\tprincipal type\tprincipal id\tscope\treach\tscope id\tid",
                    lines,
                ),
            )
        }
        ViewMode::RoleDefinitions => {
            let rows = state.role_definition_rows();
            let lines = rows.iter().map(|r| {
                let d = &r.definition;
                tsv(&[
                    &d.role_name,
                    d.kind_label(),
                    &r.assignments.to_string(),
                    &d.description,
                    &d.id,
                ])
            });
            (
                count(rows.len(), "role"),
                with_header("role\tkind\tassignments\tdescription\tid", lines),
            )
        }
        ViewMode::ExemptionList => {
            let rows = state.exemption_rows();
            let lines = rows.iter().map(|r| {
                let e = &r.exemption;
                tsv(&[
                    &e.display_name,
                    &e.category,
                    &crate::policy::expiry_label(r.expiry),
                    &e.expires_on,
                    &r.assignment,
                    &r.scope_label,
                    &e.description,
                    &e.id,
                ])
            });
            (
                count(rows.len(), "exemption"),
                with_header(
                    "exemption\tcategory\texpiry\texpires on\tassignment\tscope\treason\tid",
                    lines,
                ),
            )
        }
        ViewMode::ComplianceRanking => {
            let rows = state.compliance_rows();
            let lines = rows.iter().map(|r| {
                let c = r.compliance;
                let percent = crate::policy::ComplianceSummary::from_subscriptions(
                    &std::collections::HashMap::from([(r.subscription_id.clone(), c)]),
                )
                .percent_compliant()
                .map(|p| format!("{p:.1}"))
                .unwrap_or_default();
                tsv(&[
                    &r.name,
                    &c.non_compliant.to_string(),
                    &c.total.to_string(),
                    &percent,
                    &r.group,
                    &r.subscription_id,
                ])
            });
            (
                count(rows.len(), "subscription"),
                with_header(
                    "subscription\tnon-compliant\tevaluated\tcompliant %\tmanagement group\tid",
                    lines,
                ),
            )
        }
        ViewMode::TenantList => {
            let rows = state.tenant_rows();
            let lines = rows.iter().filter_map(|r| {
                let id = r.id.as_deref()?;
                Some(tsv(&[
                    r.provider.as_str(),
                    &r.name,
                    r.domain.as_deref().unwrap_or(""),
                    id,
                    &r.subscriptions.to_string(),
                ]))
            });
            let lines = with_header("cloud\tname\tdomain or account\tid\tsubscriptions", lines);
            (count(lines.len().saturating_sub(1), "context"), lines)
        }
        ViewMode::TagSummary => {
            let rows = state.tag_summary();
            let lines = rows
                .iter()
                .map(|t| tsv(&[&t.count.to_string(), &t.key, &t.value]));
            (
                count(rows.len(), "tag"),
                with_header("count\tkey\tvalue", lines),
            )
        }
        ViewMode::ManagementGroupTree => {
            let rows = state.tree_rows();
            let lines = rows.iter().map(|r| {
                let indent = "  ".repeat(r.depth);
                match r.kind {
                    TreeRowKind::Subscription => format!("{indent}{}\t{}", r.label, r.key),
                    _ => format!(
                        "{indent}{}\t{} subscriptions",
                        r.label, r.subscription_count
                    ),
                }
            });
            (count(rows.len(), "tree row"), lines.collect())
        }
        ViewMode::ResourceDetail => {
            let resource = state.selected_resource_detail.as_ref()?;
            let doc = state.resource_document.as_ref();
            match state.detail_tab {
                DetailTab::Overview => {
                    let mut lines = Vec::new();
                    for section in detail::overview(doc, resource) {
                        if !lines.is_empty() {
                            lines.push(String::new());
                        }
                        lines.push(section.title.to_string());
                        lines.extend(
                            section
                                .rows
                                .iter()
                                .map(|(label, value)| format!("  {label}: {value}")),
                        );
                    }
                    (format!("overview of {}", resource.name), lines)
                }
                DetailTab::Tags => {
                    let tags = detail::tags(doc, resource);
                    let lines = tags.iter().map(|(k, v)| tsv(&[k, v])).collect();
                    (count(tags.len(), "tag"), lines)
                }
                DetailTab::Policy => {
                    let rows = state.detail_policy_states()?;
                    let lines = rows.iter().map(|(name, s)| {
                        let scope =
                            state.scope_label(crate::policy::assignment_scope(&s.assignment_id));
                        tsv(&[
                            &s.state,
                            s.policy_label(),
                            &s.source_label(),
                            name,
                            &scope,
                            &s.definition_id,
                            &s.assignment_id,
                        ])
                    });
                    (
                        count(rows.len(), "policy state"),
                        with_header(
                            "state\tpolicy\tvia\tassignment\tassigned at\tdefinition id\tassignment id",
                            lines,
                        ),
                    )
                }
                DetailTab::Json => {
                    // Only the real document: the placeholder is not worth copying.
                    let doc = doc?;
                    (
                        format!("JSON of {}", resource.name),
                        vec![serde_json::to_string_pretty(doc).ok()?],
                    )
                }
            }
        }
        ViewMode::Logs => return None,
        ViewMode::Cost => {
            let report = state.cost_report()?;
            let currency = report.currency_label(state.cost_settings.usd).to_string();
            let rows = state.cost_rows();
            let lines = rows.iter().map(|r| {
                tsv(&[
                    &r.name,
                    &format!("{:.2}", r.cost),
                    &currency,
                    &format!("{:.1}", r.share),
                ])
            });
            let group = report.query.group_by.label();
            (
                format!("cost of {}", count(rows.len(), group)),
                with_header(&format!("{group}\tcost\tcurrency\tshare %"), lines),
            )
        }
    };
    if lines.is_empty() {
        return None;
    }
    Some((what, lines.join("\n") + "\n"))
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// Tab-separated fields; tabs and newlines inside a field become spaces.
/// Fields come from the clouds and are pasted into spreadsheets: other
/// control characters are replaced, and a field a spreadsheet would read
/// as a formula (`=`, `+`, `-`, `@`) starts with `'` (see
/// [`crate::safe_text::spreadsheet_cell`]).
fn tsv(fields: &[&str]) -> String {
    fields
        .iter()
        .map(|f| crate::safe_text::spreadsheet_cell(&f.replace(['\t', '\n', '\r'], " ")))
        .collect::<Vec<_>>()
        .join("\t")
}

/// The header line, then the rows (no header when there are no rows).
fn with_header(header: &str, rows: impl Iterator<Item = String>) -> Vec<String> {
    let mut lines: Vec<String> = rows.collect();
    if !lines.is_empty() {
        lines.insert(0, header.to_string());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Resource, ResourceGroup};
    use std::collections::HashMap;

    fn res(name: &str, t: &str) -> Resource {
        Resource {
            id: format!("/subscriptions/s/resourceGroups/rg/providers/{t}/{name}"),
            name: name.into(),
            resource_type: t.into(),
            location: "westeurope".into(),
            resource_group: "rg".into(),
            subscription_id: "s".into(),
            tags: HashMap::from([("env".to_string(), "prod".to_string())]),
        }
    }

    #[test]
    fn lists_copy_every_filtered_row_with_a_header() {
        let mut state = AppState::new();
        state.view = ViewMode::ResourceList;
        state.resources = vec![
            res("vm-1", "Microsoft.Compute/virtualMachines"),
            res("kv\t1", "Microsoft.KeyVault/vaults"),
        ];
        let (what, text) = view_text(&state).unwrap();
        assert_eq!(what, "2 resources");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[0],
            "name\ttype\tlocation\tresource group\tsubscription\tid"
        );
        assert!(
            lines[2].starts_with("kv 1\t"),
            "tabs inside fields are escaped"
        );

        state.filter = "vm".into();
        let (what, _) = view_text(&state).unwrap();
        assert_eq!(what, "1 resource", "only what passes the filter");

        state.filter = "nothing".into();
        assert!(view_text(&state).is_none());
    }

    #[test]
    fn resource_groups_and_type_summary() {
        let mut state = AppState::new();
        state.view = ViewMode::ResourceGroupList;
        state.resource_groups = vec![ResourceGroup {
            id: "/subscriptions/s/resourceGroups/rg".into(),
            name: "rg".into(),
            location: "westeurope".into(),
            subscription_id: "s".into(),
            tags: HashMap::new(),
        }];
        let (_, text) = view_text(&state).unwrap();
        assert_eq!(
            text,
            "name\tlocation\tsubscription\tid\nrg\twesteurope\ts\t/subscriptions/s/resourceGroups/rg\n"
        );

        state.view = ViewMode::ResourceTypeSummary;
        state.resources = vec![res("vnet-1", "Microsoft.Network/virtualNetworks")];
        let (_, text) = view_text(&state).unwrap();
        assert_eq!(
            text,
            "count\talias\ttype\n1\tvnet\tmicrosoft.network/virtualnetworks\n"
        );
    }

    #[test]
    fn detail_copies_the_current_tab() {
        let mut state = AppState::new();
        state.view = ViewMode::ResourceDetail;
        state.selected_resource_detail = Some(res("vm-1", "Microsoft.Compute/virtualMachines"));

        let (what, text) = view_text(&state).unwrap();
        assert_eq!(what, "overview of vm-1");
        assert!(text.starts_with("Resource\n  Name: vm-1\n"), "{text}");

        state.detail_tab = DetailTab::Tags;
        assert_eq!(view_text(&state).unwrap().1, "env\tprod\n");

        state.detail_tab = DetailTab::Json;
        assert!(view_text(&state).is_none(), "nothing loaded yet");
        state.resource_document = Some(serde_json::json!({"name": "vm-1"}));
        assert_eq!(view_text(&state).unwrap().1, "{\n  \"name\": \"vm-1\"\n}\n");
    }
}
