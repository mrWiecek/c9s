//! Azure RBAC: role assignments and role definitions.
//!
//! A role assignment grants a principal (user, group, service principal or
//! managed identity) a role at a scope, and so at every scope below it. At a
//! subscription, access therefore comes from assignments on the
//! subscription, on each management group above it (inherited), and on its
//! resource groups and resources (below, for part of it).

use crate::policy::{ancestors, within, PolicyScope};
use crate::scope_tree::ScopeTree;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A principal's name from the directory (Microsoft Graph).
pub use c9s_azure::directory::{Group, Principal};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleAssignment {
    pub id: String,
    /// ARM ID of the scope it is made at ("/" for the tenant root)
    pub scope: String,
    pub role_definition_id: String,
    pub principal_id: String,
    /// "User", "Group", "ServicePrincipal", …
    pub principal_type: String,
    pub created_on: String,
    pub description: String,
    /// An ABAC condition, if any
    pub condition: String,
    /// PIM: eligible (must be activated before use), not active
    #[serde(default)]
    pub eligible: bool,
    /// PIM: when the eligibility ends (RFC 3339; empty: permanent)
    #[serde(default)]
    pub ends: String,
}

impl RoleAssignment {
    /// From a row of `resource_graph::role_assignments_query`.
    pub fn from_row(row: &Value) -> Option<Self> {
        let text = |key: &str| {
            row.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let id = text("id");
        if id.is_empty() {
            return None;
        }
        Some(Self {
            id,
            scope: text("scope"),
            role_definition_id: text("roleDefinitionId"),
            principal_id: text("principalId"),
            principal_type: text("principalType"),
            created_on: text("createdOn"),
            description: text("description"),
            condition: text("condition"),
            ..Default::default()
        })
    }

    /// From a PIM role eligibility instance (ARM). `None` for one that
    /// ARM expands from elsewhere (`memberType` Group or Inherited: c9s
    /// works out groups and inheritance itself) or that has ended at `now`
    /// (Unix seconds).
    pub fn from_eligibility(item: &Value, now: i64) -> Option<Self> {
        let p = item.get("properties")?;
        let text = |key: &str| {
            p.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let member = text("memberType");
        if !member.is_empty() && !member.eq_ignore_ascii_case("Direct") {
            return None;
        }
        let ends = text("endDateTime");
        if let Ok(end) = chrono::DateTime::parse_from_rfc3339(&ends) {
            if end.timestamp() <= now {
                return None;
            }
        }
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if id.is_empty() {
            return None;
        }
        let scope = text("scope");
        Some(Self {
            id,
            scope: if scope.is_empty() { "/".into() } else { scope },
            role_definition_id: text("roleDefinitionId"),
            principal_id: text("principalId").to_ascii_lowercase(),
            principal_type: text("principalType"),
            created_on: text("createdOn"),
            description: String::new(),
            condition: text("condition"),
            eligible: true,
            ends,
        })
    }

    /// "eligible", "eligible until 2027-03-01", or "" for an active one.
    pub fn pim_label(&self) -> String {
        if !self.eligible {
            return String::new();
        }
        match self.ends.get(..10) {
            Some(day) if !self.ends.is_empty() => format!("eligible until {day}"),
            _ => "eligible".into(),
        }
    }

    /// The role's GUID (lowercase), which identifies it whatever scope its
    /// ID is written at.
    pub fn role_guid(&self) -> String {
        role_guid(&self.role_definition_id)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleDefinition {
    pub id: String,
    pub role_name: String,
    /// "BuiltInRole" or "CustomRole"
    pub role_type: String,
    pub description: String,
    pub assignable_scopes: Vec<String>,
    pub actions: Vec<String>,
    pub not_actions: Vec<String>,
    pub data_actions: Vec<String>,
    pub not_data_actions: Vec<String>,
}

impl RoleDefinition {
    /// From a row of `resource_graph::role_definitions_query`.
    pub fn from_row(row: &Value) -> Option<Self> {
        let text = |key: &str| {
            row.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let id = text("id");
        if id.is_empty() {
            return None;
        }
        let strings = |v: Option<&Value>| -> Vec<String> {
            v.and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        // `permissions` is a list of blocks; one is usual, merge them all.
        let permissions: Vec<&Value> = row
            .get("permissions")
            .and_then(Value::as_array)
            .map(|a| a.iter().collect())
            .unwrap_or_default();
        let all = |key: &str| -> Vec<String> {
            permissions
                .iter()
                .flat_map(|p| strings(p.get(key)))
                .collect()
        };
        Some(Self {
            role_name: Some(text("roleName"))
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| text("name")),
            role_type: text("roleType"),
            description: text("description"),
            assignable_scopes: strings(row.get("assignableScopes")),
            actions: all("actions"),
            not_actions: all("notActions"),
            data_actions: all("dataActions"),
            not_data_actions: all("notDataActions"),
            id,
        })
    }

    pub fn is_custom(&self) -> bool {
        self.role_type.eq_ignore_ascii_case("CustomRole")
    }

    /// "custom" or "built-in".
    pub fn kind_label(&self) -> &'static str {
        if self.is_custom() {
            "custom"
        } else {
            "built-in"
        }
    }

    pub fn guid(&self) -> String {
        role_guid(&self.id)
    }
}

/// The GUID at the end of a role definition ID, lowercase.
pub fn role_guid(id: &str) -> String {
    id.rsplit('/').next().unwrap_or(id).to_ascii_lowercase()
}

/// How a role assignment relates to the scope being looked at.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reach {
    /// On a scope above (management group or the tenant root): applies to
    /// all of it. The number orders from the top (0: highest).
    Inherited(usize),
    /// Made at the scope itself
    Here,
    /// On a scope below (child group, subscription, resource group or
    /// resource): applies to part of it
    Below,
}

/// The role assignments that matter at `target` (all of them for `None`,
/// the whole tenant), with how each reaches it.
pub fn assignments_at<'a>(
    assignments: &'a [RoleAssignment],
    target: Option<&PolicyScope>,
    tree: Option<&ScopeTree>,
) -> Vec<(&'a RoleAssignment, Reach)> {
    let Some(target) = target else {
        return assignments.iter().map(|a| (a, Reach::Here)).collect();
    };
    // The scope and its ancestors, the scope first; "/" is above them all.
    let chain = ancestors(target, tree);
    let here = target.arm_id();
    assignments
        .iter()
        .filter_map(|a| {
            let scope = a.scope.to_ascii_lowercase();
            let scope = scope.trim_end_matches('/');
            if scope == here {
                return Some((a, Reach::Here));
            }
            if scope.is_empty() {
                // "/": the tenant root, above everything.
                return Some((a, Reach::Inherited(0)));
            }
            if let Some(i) = chain.iter().position(|c| c == scope) {
                return Some((a, Reach::Inherited(chain.len() - i)));
            }
            within(scope, target, tree).then_some((a, Reach::Below))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pim_eligibility_instances_become_eligible_assignments() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-08T00:00:00Z")
            .unwrap()
            .timestamp();
        let item = |member: &str, end: &str| {
            json!({
                "id": "/providers/Microsoft.Authorization/roleEligibilityScheduleInstances/e1",
                "properties": {
                    "scope": "/subscriptions/s1",
                    "roleDefinitionId": "/subscriptions/s1/providers/Microsoft.Authorization/roleDefinitions/8E3AF657",
                    "principalId": "ADA", "principalType": "User",
                    "memberType": member, "endDateTime": end, "status": "Provisioned"
                }
            })
        };
        let a =
            RoleAssignment::from_eligibility(&item("Direct", "2027-03-01T10:00:00Z"), now).unwrap();
        assert!(a.eligible);
        assert_eq!(a.principal_id, "ada");
        assert_eq!(a.role_guid(), "8e3af657");
        assert_eq!(a.pim_label(), "eligible until 2027-03-01");
        let permanent = RoleAssignment::from_eligibility(&item("Direct", ""), now).unwrap();
        assert_eq!(permanent.pim_label(), "eligible");
        // c9s expands groups and inheritance itself; ended ones are gone.
        for (member, end) in [
            ("Group", ""),
            ("Inherited", ""),
            ("Direct", "2026-01-01T00:00:00Z"),
        ] {
            assert!(
                RoleAssignment::from_eligibility(&item(member, end), now).is_none(),
                "{member} {end}"
            );
        }
        let active = RoleAssignment::default();
        assert_eq!(active.pim_label(), "");
    }
    use crate::scope_tree::{GroupNode, SubscriptionNode};
    use serde_json::json;

    #[test]
    fn rows_parse_and_roles_are_built_in_or_custom() {
        let ra = RoleAssignment::from_row(&json!({
            "id": "/subscriptions/s1/providers/Microsoft.Authorization/roleAssignments/a",
            "scope": "/subscriptions/s1",
            "roleDefinitionId": "/subscriptions/s1/providers/Microsoft.Authorization/roleDefinitions/ACDD72A7",
            "principalId": "p1", "principalType": "User"
        }))
        .unwrap();
        assert_eq!(ra.role_guid(), "acdd72a7");
        let rd = RoleDefinition::from_row(&json!({
            "id": "/providers/Microsoft.Authorization/RoleDefinitions/acdd72a7", "name": "acdd72a7",
            "roleName": "Reader", "roleType": "BuiltInRole",
            "assignableScopes": ["/"],
            "permissions": [{"actions": ["*/read"], "notActions": []},
                            {"dataActions": ["x/read"]}]
        }))
        .unwrap();
        assert_eq!((rd.guid(), rd.kind_label()), (ra.role_guid(), "built-in"));
        assert_eq!(
            (rd.actions.clone(), rd.data_actions.clone()),
            (vec!["*/read".to_string()], vec!["x/read".to_string()])
        );
        assert!(
            RoleDefinition::from_row(&json!({"id": "x", "roleType": "CustomRole"}))
                .unwrap()
                .is_custom()
        );
        assert!(RoleAssignment::from_row(&json!({})).is_none());
    }

    #[test]
    fn assignments_reach_a_scope_from_above_here_or_below() {
        let tree = ScopeTree {
            root: "root".into(),
            root_display_name: "Root".into(),
            groups: vec![GroupNode {
                name: "corp".into(),
                display_name: "Corp".into(),
                parent: "root".into(),
            }],
            subscriptions: vec![SubscriptionNode {
                id: "s1".into(),
                display_name: "s1".into(),
                parent: "corp".into(),
            }],
        };
        let at = |scope: &str| RoleAssignment {
            id: format!("{scope}/x"),
            scope: scope.into(),
            ..Default::default()
        };
        let all = vec![
            at("/"),
            at("/providers/Microsoft.Management/managementGroups/root"),
            at("/providers/Microsoft.Management/managementGroups/corp"),
            at("/subscriptions/S1"),
            at("/subscriptions/s1/resourceGroups/rg"),
            at("/subscriptions/s2"),
        ];
        let sub = PolicyScope::Subscription("s1".into());
        let reach: Vec<(String, Reach)> = assignments_at(&all, Some(&sub), Some(&tree))
            .into_iter()
            .map(|(a, r)| (a.scope.clone(), r))
            .collect();
        assert_eq!(
            reach,
            [
                ("/".to_string(), Reach::Inherited(0)),
                (
                    "/providers/Microsoft.Management/managementGroups/root".to_string(),
                    Reach::Inherited(1)
                ),
                (
                    "/providers/Microsoft.Management/managementGroups/corp".to_string(),
                    Reach::Inherited(2)
                ),
                ("/subscriptions/S1".to_string(), Reach::Here),
                (
                    "/subscriptions/s1/resourceGroups/rg".to_string(),
                    Reach::Below
                ),
            ],
            "another subscription's assignment is left out"
        );
        let corp = PolicyScope::Group("corp".into());
        let below = assignments_at(&all, Some(&corp), Some(&tree))
            .into_iter()
            .filter(|(_, r)| *r == Reach::Below)
            .count();
        assert_eq!(below, 2, "the subscription and its resource group");
        assert_eq!(assignments_at(&all, None, None).len(), all.len());
    }
}
