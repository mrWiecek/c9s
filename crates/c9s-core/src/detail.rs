//! What the resource detail view shows, derived from the resource's full
//! document (a Resource Graph `resources` row: ARM's JSON plus
//! `subscriptionId` and `resourceGroup`).
//!
//! Everything here also works without the document (not loaded yet, or the
//! load failed), from the list row the user opened.

use crate::models::Resource;
use serde_json::Value;

/// Most property rows the Overview tab lists; the JSON tab has the rest.
pub const MAX_PROPERTY_ROWS: usize = 60;
/// How deep nested `properties` objects are flattened into dotted names.
const MAX_DEPTH: usize = 3;

/// The Overview row naming the resource's resource group (Enter opens it).
pub const RESOURCE_GROUP_ROW: &str = "Resource group";
/// The Overview row naming the resource's subscription (Enter opens it).
pub const SUBSCRIPTION_ROW: &str = "Subscription";

/// The line each Overview row is drawn on: a heading per section, a blank
/// line before every section but the first.
pub fn row_lines(sections: &[Section]) -> Vec<usize> {
    let mut lines = Vec::new();
    let mut line = 0;
    for (i, section) in sections.iter().enumerate() {
        if i > 0 {
            line += 1;
        }
        line += 1; // the heading
        for _ in &section.rows {
            lines.push(line);
            line += 1;
        }
    }
    lines
}

/// A titled group of label/value rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: &'static str,
    pub rows: Vec<(String, String)>,
}

/// The Overview tab: identity of the resource, SKU, managed identity, and
/// the scalar values in `properties` (e.g. `hardwareProfile.vmSize`).
pub fn overview(doc: Option<&Value>, resource: &Resource) -> Vec<Section> {
    let text = |key: &str| doc.and_then(|d| d.get(key)).and_then(scalar);
    let mut main = vec![
        (
            "Name".to_string(),
            text("name").unwrap_or(resource.name.clone()),
        ),
        (
            "Type".to_string(),
            text("type").unwrap_or(resource.resource_type.clone()),
        ),
    ];
    if let Some(kind) = text("kind") {
        main.push(("Kind".into(), kind));
    }
    main.push((
        "Location".into(),
        text("location").unwrap_or(resource.location.clone()),
    ));
    if let Some(zones) = doc.and_then(|d| d.get("zones")).and_then(Value::as_array) {
        let zones: Vec<String> = zones.iter().filter_map(scalar).collect();
        if !zones.is_empty() {
            main.push(("Zones".into(), zones.join(", ")));
        }
    }
    main.push((RESOURCE_GROUP_ROW.into(), resource.resource_group.clone()));
    main.push((SUBSCRIPTION_ROW.into(), resource.subscription_id.clone()));
    if let Some(state) = doc
        .and_then(|d| d.pointer("/properties/provisioningState"))
        .and_then(scalar)
    {
        main.push(("Provisioning state".into(), state));
    }
    if let Some(managed_by) = text("managedBy") {
        main.push(("Managed by".into(), managed_by));
    }
    main.push(("ID".into(), resource.id.clone()));

    let mut sections = vec![Section {
        title: "Resource",
        rows: main,
    }];
    for (title, key) in [("SKU", "sku"), ("Identity", "identity"), ("Plan", "plan")] {
        let mut rows = Vec::new();
        if let Some(value) = doc.and_then(|d| d.get(key)) {
            flatten("", value, 1, &mut rows);
        }
        if !rows.is_empty() {
            sections.push(Section { title, rows });
        }
    }

    if let Some(doc) = doc {
        sections.extend(type_sections(doc, &resource.resource_type));
    }

    let mut properties = Vec::new();
    if let Some(value) = doc.and_then(|d| d.get("properties")) {
        flatten("", value, 0, &mut properties);
    }
    properties.retain(|(name, _)| name != "provisioningState");
    if properties.len() > MAX_PROPERTY_ROWS {
        let more = properties.len() - MAX_PROPERTY_ROWS;
        properties.truncate(MAX_PROPERTY_ROWS);
        properties.push(("…".into(), format!("{more} more (see the JSON tab)")));
    }
    if !properties.is_empty() {
        sections.push(Section {
            title: "Properties",
            rows: properties,
        });
    }
    sections
}

/// Extra sections for resource types where the generic property list hides
/// what matters (it shows arrays only as "[n items]").
fn type_sections(doc: &Value, resource_type: &str) -> Vec<Section> {
    let kind = resource_type.to_ascii_lowercase();
    if kind == "microsoft.authorization/roledefinitions" {
        return role_definition_sections(doc);
    }
    if kind == "microsoft.authorization/policydefinitions"
        || kind == "microsoft.authorization/policysetdefinitions"
    {
        return vec![policy_definition_section(doc)];
    }
    if resource_type.eq_ignore_ascii_case("microsoft.network/virtualnetworks") {
        let subnets = doc
            .pointer("/properties/subnets")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if subnets.is_empty() {
            return vec![];
        }
        return vec![Section {
            title: "Subnets",
            rows: subnets.iter().map(subnet_row).collect(),
        }];
    }
    vec![]
}

/// A role definition: what it is, then what it allows and denies (one
/// permission per row) and where it can be assigned.
fn role_definition_sections(doc: &Value) -> Vec<Section> {
    let role = crate::rbac::RoleDefinition::from_row(&serde_json::json!({
        "id": doc.get("id").cloned().unwrap_or_default(),
        "name": doc.get("name").cloned().unwrap_or_default(),
        "roleName": doc.pointer("/properties/roleName").cloned().unwrap_or_default(),
        "roleType": doc.pointer("/properties/type").cloned().unwrap_or_default(),
        "description": doc.pointer("/properties/description").cloned().unwrap_or_default(),
        "assignableScopes": doc.pointer("/properties/assignableScopes").cloned().unwrap_or_default(),
        "permissions": doc.pointer("/properties/permissions").cloned().unwrap_or_default(),
    }));
    let Some(role) = role else {
        return vec![];
    };
    let mut sections = vec![Section {
        title: "Role definition",
        rows: vec![
            ("Role".into(), role.role_name.clone()),
            ("Type".into(), role.kind_label().into()),
            ("Description".into(), role.description.clone()),
        ],
    }];
    for (title, items) in [
        ("Actions", &role.actions),
        ("Not actions", &role.not_actions),
        ("Data actions", &role.data_actions),
        ("Not data actions", &role.not_data_actions),
        ("Assignable scopes", &role.assignable_scopes),
    ] {
        if !items.is_empty() {
            sections.push(Section {
                title,
                rows: items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| ((i + 1).to_string(), item.clone()))
                    .collect(),
            });
        }
    }
    sections
}

/// A policy definition or initiative at a glance: what it is and does.
fn policy_definition_section(doc: &Value) -> Section {
    let get = |path: &str| doc.pointer(path).and_then(scalar);
    let mut rows = Vec::new();
    for (label, path) in [
        ("Display name", "/properties/displayName"),
        ("Type", "/properties/policyType"),
        ("Category", "/properties/metadata/category"),
        ("Version", "/properties/metadata/version"),
        ("Mode", "/properties/mode"),
    ] {
        if let Some(value) = get(path) {
            rows.push((label.to_string(), value));
        }
    }
    if let Some(effect) = get("/properties/policyRule/then/effect") {
        // Usually a parameter: show its default and allowed values.
        let param = effect
            .strip_prefix("[parameters('")
            .and_then(|p| p.strip_suffix("')]"))
            .map(str::to_string);
        let effect = match param {
            Some(p) => {
                let default = get(&format!("/properties/parameters/{p}/defaultValue"));
                let allowed = doc
                    .pointer(&format!("/properties/parameters/{p}/allowedValues"))
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(scalar).collect::<Vec<_>>().join(", "));
                let mut text = default.unwrap_or_else(|| format!("parameter {p}"));
                if let Some(allowed) = allowed.filter(|a| !a.is_empty()) {
                    text.push_str(&format!(" (allowed: {allowed})"));
                }
                text
            }
            None => effect,
        };
        rows.push(("Effect".into(), effect));
    }
    if let Some(policies) = doc
        .pointer("/properties/policyDefinitions")
        .and_then(Value::as_array)
    {
        rows.push(("Policies".into(), policies.len().to_string()));
    }
    if let Some(description) = get("/properties/description") {
        rows.push(("Description".into(), description));
    }
    Section {
        title: "Policy definition",
        rows,
    }
}

/// One subnet: name → "prefix · delegated to … · NSG … · route table …".
fn subnet_row(subnet: &Value) -> (String, String) {
    let name = subnet
        .get("name")
        .and_then(scalar)
        .unwrap_or_else(|| "(unnamed)".into());
    let props = subnet.get("properties");
    let get = |path: &str| props.and_then(|p| p.pointer(path));
    // The last segment of a referenced resource's ID is its name.
    let ref_name = |path: &str| {
        get(path)
            .and_then(Value::as_str)
            .and_then(|id| id.rsplit('/').next())
            .map(str::to_string)
    };

    let mut parts = Vec::new();
    let prefixes: Vec<String> = match get("/addressPrefix").and_then(scalar) {
        Some(prefix) => vec![prefix],
        None => get("/addressPrefixes")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(scalar).collect())
            .unwrap_or_default(),
    };
    if !prefixes.is_empty() {
        parts.push(prefixes.join(", "));
    }
    let delegations: Vec<String> = get("/delegations")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|d| d.pointer("/properties/serviceName").and_then(scalar))
                .collect()
        })
        .unwrap_or_default();
    parts.push(if delegations.is_empty() {
        "no delegation".into()
    } else {
        format!("delegated to {}", delegations.join(", "))
    });
    for (label, path) in [
        ("NSG", "/networkSecurityGroup/id"),
        ("route table", "/routeTable/id"),
        ("NAT gateway", "/natGateway/id"),
    ] {
        if let Some(name) = ref_name(path) {
            parts.push(format!("{label} {name}"));
        }
    }
    (name, parts.join(" · "))
}

/// Tags, sorted by key: from the document when loaded, else the list row.
pub fn tags(doc: Option<&Value>, resource: &Resource) -> Vec<(String, String)> {
    let mut tags: Vec<(String, String)> = match doc.and_then(|d| d.get("tags")) {
        Some(Value::Object(map)) => map
            .iter()
            .map(|(k, v)| (k.clone(), scalar(v).unwrap_or_default()))
            .collect(),
        _ => resource
            .tags
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    };
    tags.sort_by_cached_key(|(k, _)| k.to_lowercase());
    tags
}

/// The JSON tab's text: the document pretty-printed, or a placeholder.
pub fn json_text(doc: Option<&Value>) -> String {
    match doc {
        Some(doc) => serde_json::to_string_pretty(doc).unwrap_or_default(),
        None => "(not loaded yet)".into(),
    }
}

/// Azure Portal link for an ARM ID (resource, resource group, subscription
/// or management group).
pub fn portal_url(arm_id: &str, tenant_id: Option<&str>) -> String {
    let tenant = tenant_id
        .filter(|t| !t.is_empty())
        .map(|t| format!("@{t}"))
        .unwrap_or_default();
    format!(
        "https://portal.azure.com/#{}/resource{}",
        encode_path(&tenant),
        encode_path(arm_id)
    )
}

/// Google Cloud console link for a project (`projects/ID`), folder
/// (`folders/N`), organization (`organizations/N`) or Cloud Asset resource
/// name (`//service.googleapis.com/…`) in `project`. Common resources open
/// on their own page; others on the Asset Inventory search for their name.
pub fn gcp_console_url(id: &str, project: Option<&str>) -> String {
    const CONSOLE: &str = "https://console.cloud.google.com";
    if let Some(p) = id.strip_prefix("projects/") {
        return format!("{CONSOLE}/home/dashboard?project={}", encode(p));
    }
    if let Some(f) = id.strip_prefix("folders/") {
        return format!("{CONSOLE}/cloud-resource-manager?folder={}", encode(f));
    }
    if let Some(o) = id.strip_prefix("organizations/") {
        return format!(
            "{CONSOLE}/cloud-resource-manager?organizationId={}",
            encode(o)
        );
    }
    let project = project
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .or_else(|| c9s_gcp::project_of(id).map(str::to_string))
        .unwrap_or_default();
    let q = format!("project={}", encode(&project));
    let (service, path) = id
        .strip_prefix("//")
        .and_then(|rest| rest.split_once('/'))
        .unwrap_or(("", ""));
    let parts: Vec<&str> = path.split('/').collect();
    // `segment(key)`: the part after `key` in the resource path.
    let segment = |key: &str| parts.windows(2).find(|w| w[0] == key).map(|w| encode(w[1]));
    let page = match service {
        "compute.googleapis.com" => match (segment("zones"), segment("instances")) {
            (Some(zone), Some(vm)) => Some(format!(
                "compute/instancesDetail/zones/{zone}/instances/{vm}"
            )),
            _ => None,
        },
        "storage.googleapis.com" => parts
            .first()
            .filter(|b| !b.is_empty())
            .map(|bucket| format!("storage/browser/{}", encode(bucket))),
        "container.googleapis.com" => match (segment("locations"), segment("clusters")) {
            (Some(loc), Some(c)) => Some(format!("kubernetes/clusters/details/{loc}/{c}/details")),
            _ => None,
        },
        "cloudsql.googleapis.com" | "sqladmin.googleapis.com" => {
            segment("instances").map(|i| format!("sql/instances/{i}/overview"))
        }
        "run.googleapis.com" => match (segment("locations"), segment("services")) {
            (Some(loc), Some(svc)) => Some(format!("run/detail/{loc}/{svc}")),
            _ => None,
        },
        "cloudfunctions.googleapis.com" => match (segment("locations"), segment("functions")) {
            (Some(loc), Some(f)) => Some(format!("functions/details/{loc}/{f}")),
            _ => None,
        },
        "pubsub.googleapis.com" => {
            segment("topics").map(|t| format!("cloudpubsub/topic/detail/{t}"))
        }
        "bigquery.googleapis.com" if segment("tables").is_none() => segment("datasets")
            .map(|d| format!("bigquery?p={}&d={d}&page=dataset", encode(&project))),
        _ => None,
    };
    match page {
        Some(page) if page.contains('?') => format!("{CONSOLE}/{page}&{q}"),
        Some(page) => format!("{CONSOLE}/{page}?{q}"),
        None => format!(
            "{CONSOLE}/iam-admin/asset-inventory/resources?{q}&query={}",
            encode(&format!("name:\"{id}\""))
        ),
    }
}

/// AWS console link: a resource by its ARN (the console's ARN redirect),
/// an organizational unit or root in AWS Organizations, anything else (an
/// account ID) the console home.
pub fn aws_console_url(id: &str) -> String {
    const CONSOLE: &str = "https://console.aws.amazon.com";
    if id.starts_with("arn:") {
        format!("{CONSOLE}/go/view?arn={}", encode(id))
    } else if id.starts_with("ou-") || id.starts_with("r-") {
        format!("{CONSOLE}/organizations/v2/home/accounts")
    } else {
        format!("{CONSOLE}/console/home")
    }
}

/// Percent-encode what a URL path can't carry as is (spaces, quotes,
/// non-ASCII, `%`...), keeping `/` and the characters ARM IDs use.
fn encode_path(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' => (b as char).to_string(),
            b'-' | b'.' | b'_' | b'~' | b'/' | b'@' | b'!' | b'$' | b'\'' | b'(' | b')' | b'*'
            | b'+' | b',' | b';' | b'=' | b':' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Percent-encode a URL query value or path segment.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The Entra admin page of a directory object in the Azure Portal: a group,
/// a user, or a service principal (enterprise application, with its app
/// ID). `None` for kinds without one (e.g. devices).
pub fn entra_portal_url(
    kind: &str,
    id: &str,
    app_id: &str,
    tenant_id: Option<&str>,
) -> Option<String> {
    let tenant = tenant_id
        .filter(|t| !t.is_empty())
        .map(|t| format!("@{t}"))
        .unwrap_or_default();
    let blade = match kind {
        "group" => format!("Microsoft_AAD_IAM/GroupDetailsMenuBlade/~/Overview/groupId/{id}"),
        "user" => {
            format!("Microsoft_AAD_UsersAndTenants/UserProfileMenuBlade/~/overview/userId/{id}")
        }
        "application" => {
            format!("Microsoft_AAD_RegisteredApps/ApplicationMenuBlade/~/Overview/appId/{app_id}")
        }
        "servicePrincipal" => {
            format!("Microsoft_AAD_IAM/ManagedAppMenuBlade/~/Overview/objectId/{id}/appId/{app_id}")
        }
        _ => return None,
    };
    Some(format!(
        "https://portal.azure.com/#{}/view/{}",
        encode_path(&tenant),
        encode_path(&blade)
    ))
}

/// Text for a JSON scalar; `None` for objects, arrays and null.
fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Collect scalar leaves of `value` as `a.b.c` rows. Arrays become a count
/// (or the values, when they are a few scalars).
fn flatten(prefix: &str, value: &Value, depth: usize, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let name = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                match child {
                    Value::Object(_) if depth < MAX_DEPTH => flatten(&name, child, depth + 1, out),
                    Value::Object(_) => out.push((name, "{…}".into())),
                    _ => flatten(&name, child, depth + 1, out),
                }
            }
        }
        Value::Array(items) => {
            let scalars: Vec<String> = items.iter().filter_map(scalar).collect();
            let text = if !items.is_empty() && scalars.len() == items.len() && items.len() <= 5 {
                scalars.join(", ")
            } else {
                format!("[{} items]", items.len())
            };
            out.push((prefix.to_string(), text));
        }
        Value::Null => {}
        other => {
            if let Some(text) = scalar(other) {
                out.push((prefix.to_string(), text));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn aws_console_links() {
        assert_eq!(
            aws_console_url("arn:aws:ec2:eu-west-1:1:instance/i-0abc"),
            "https://console.aws.amazon.com/go/view?arn=arn%3Aaws%3Aec2%3Aeu-west-1%3A1%3Ainstance%2Fi-0abc"
        );
        assert!(aws_console_url("ou-ab12-34cd5678").contains("/organizations/"));
        assert_eq!(
            aws_console_url("111122223333"),
            "https://console.aws.amazon.com/console/home"
        );
    }

    #[test]
    fn gcp_console_links() {
        const C: &str = "https://console.cloud.google.com";
        let url = |id: &str, p: Option<&str>| gcp_console_url(id, p);
        assert_eq!(
            url("projects/billing-prod", None),
            format!("{C}/home/dashboard?project=billing-prod")
        );
        assert_eq!(
            url("folders/12", None),
            format!("{C}/cloud-resource-manager?folder=12")
        );
        assert_eq!(
            url(
                "//compute.googleapis.com/projects/p1/zones/europe-west1-b/instances/vm-1",
                None
            ),
            format!("{C}/compute/instancesDetail/zones/europe-west1-b/instances/vm-1?project=p1")
        );
        assert_eq!(
            url("//storage.googleapis.com/logs-eu", Some("p1")),
            format!("{C}/storage/browser/logs-eu?project=p1"),
            "buckets carry no project: the row's"
        );
        assert_eq!(
            url(
                "//container.googleapis.com/projects/p1/locations/europe-west1/clusters/gke",
                None
            ),
            format!("{C}/kubernetes/clusters/details/europe-west1/gke/details?project=p1")
        );
        assert_eq!(
            url("//bigquery.googleapis.com/projects/p1/datasets/sales", None),
            format!("{C}/bigquery?p=p1&d=sales&page=dataset&project=p1")
        );
        let other = url("//dns.googleapis.com/projects/p1/managedZones/z", None);
        assert!(
            other.starts_with(&format!(
                "{C}/iam-admin/asset-inventory/resources?project=p1&query=name%3A%22%2F%2Fdns"
            )),
            "{other}"
        );
    }

    fn resource() -> Resource {
        Resource {
            id: "/subscriptions/s1/resourceGroups/rg/providers/Microsoft.Compute/virtualMachines/vm1"
                .into(),
            name: "vm1".into(),
            resource_type: "microsoft.compute/virtualmachines".into(),
            location: "westeurope".into(),
            resource_group: "rg".into(),
            subscription_id: "s1".into(),
            tags: HashMap::from([("env".to_string(), "dev".to_string())]),
        }
    }

    fn vm() -> Value {
        json!({
            "name": "vm1",
            "type": "microsoft.compute/virtualmachines",
            "location": "westeurope",
            "zones": ["1", "2"],
            "sku": null,
            "identity": {"type": "SystemAssigned", "principalId": "p-1"},
            "tags": {"Owner": "team-a", "env": "prod"},
            "properties": {
                "provisioningState": "Succeeded",
                "hardwareProfile": {"vmSize": "Standard_D2s_v5"},
                "storageProfile": {"osDisk": {"osType": "Linux", "managedDisk": {"id": "/x"}}},
                "networkProfile": {"networkInterfaces": [{"id": "/nic"}]},
                "priority": "Regular",
                "a": {"b": {"c": {"d": {"e": 1}}}},
                "licenseType": null
            }
        })
    }

    fn rows<'a>(sections: &'a [Section], title: &str) -> &'a [(String, String)] {
        &sections.iter().find(|s| s.title == title).unwrap().rows
    }

    fn get<'a>(rows: &'a [(String, String)], label: &str) -> Option<&'a str> {
        rows.iter()
            .find(|(l, _)| l == label)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn overview_without_a_document_uses_the_list_row() {
        let sections = overview(None, &resource());
        assert_eq!(sections.len(), 1);
        let main = rows(&sections, "Resource");
        assert_eq!(get(main, "Name"), Some("vm1"));
        assert_eq!(get(main, "Resource group"), Some("rg"));
        assert!(get(main, "Provisioning state").is_none());
    }

    #[test]
    fn overview_flattens_properties_identity_and_zones() {
        let doc = vm();
        let sections = overview(Some(&doc), &resource());
        let main = rows(&sections, "Resource");
        assert_eq!(get(main, "Zones"), Some("1, 2"));
        assert_eq!(get(main, "Provisioning state"), Some("Succeeded"));

        let identity = rows(&sections, "Identity");
        assert_eq!(get(identity, "type"), Some("SystemAssigned"));
        assert!(
            sections.iter().all(|s| s.title != "SKU"),
            "null sku is skipped"
        );

        let props = rows(&sections, "Properties");
        assert_eq!(
            get(props, "hardwareProfile.vmSize"),
            Some("Standard_D2s_v5")
        );
        assert_eq!(get(props, "storageProfile.osDisk.osType"), Some("Linux"));
        assert_eq!(
            get(props, "storageProfile.osDisk.managedDisk.id"),
            Some("/x")
        );
        assert_eq!(
            get(props, "a.b.c.d"),
            Some("{…}"),
            "deeper objects are summarized"
        );
        assert_eq!(
            get(props, "networkProfile.networkInterfaces"),
            Some("[1 items]")
        );
        assert!(get(props, "provisioningState").is_none(), "shown above");
        assert!(get(props, "licenseType").is_none(), "nulls are skipped");
    }

    #[test]
    fn vnets_list_subnets_with_delegations_and_references() {
        let doc = json!({
            "type": "microsoft.network/virtualnetworks",
            "properties": {
                "addressSpace": {"addressPrefixes": ["10.0.0.0/16"]},
                "subnets": [
                    {"name": "app", "properties": {
                        "addressPrefix": "10.0.1.0/24",
                        "delegations": [{"name": "d", "properties": {"serviceName": "Microsoft.Web/serverFarms"}}],
                        "networkSecurityGroup": {"id": "/subscriptions/s/resourceGroups/rg/providers/Microsoft.Network/networkSecurityGroups/nsg-app"}
                    }},
                    {"name": "db", "properties": {
                        "addressPrefixes": ["10.0.2.0/24", "fd00::/64"],
                        "delegations": [],
                        "routeTable": {"id": "/x/routeTables/rt-db"},
                        "natGateway": {"id": "/x/natGateways/nat-1"}
                    }}
                ]
            }
        });
        let vnet = Resource {
            resource_type: "Microsoft.Network/virtualNetworks".into(),
            ..resource()
        };
        let sections = overview(Some(&doc), &vnet);
        assert_eq!(
            rows(&sections, "Subnets"),
            [
                (
                    "app".to_string(),
                    "10.0.1.0/24 · delegated to Microsoft.Web/serverFarms · NSG nsg-app"
                        .to_string()
                ),
                (
                    "db".to_string(),
                    "10.0.2.0/24, fd00::/64 · no delegation · route table rt-db · NAT gateway nat-1"
                        .to_string()
                ),
            ]
        );
        // Other types get no Subnets section.
        assert!(overview(Some(&doc), &resource())
            .iter()
            .all(|s| s.title != "Subnets"));
    }

    #[test]
    fn many_properties_are_capped_with_a_pointer_to_json() {
        let props: serde_json::Map<String, Value> =
            (0..100).map(|i| (format!("p{i:03}"), json!(i))).collect();
        let doc = json!({ "properties": props });
        let sections = overview(Some(&doc), &resource());
        let props = rows(&sections, "Properties");
        assert_eq!(props.len(), MAX_PROPERTY_ROWS + 1);
        assert_eq!(props.last().unwrap().1, "40 more (see the JSON tab)");
    }

    #[test]
    fn tags_prefer_the_document_and_sort_case_insensitively() {
        let doc = vm();
        assert_eq!(
            tags(Some(&doc), &resource()),
            [
                ("env".to_string(), "prod".to_string()),
                ("Owner".to_string(), "team-a".to_string())
            ]
        );
        assert_eq!(
            tags(None, &resource()),
            [("env".to_string(), "dev".to_string())]
        );
    }

    #[test]
    fn portal_urls() {
        assert_eq!(
            portal_url("/subscriptions/s1", Some("t1")),
            "https://portal.azure.com/#@t1/resource/subscriptions/s1"
        );
        assert_eq!(
            portal_url("/subscriptions/s1", None),
            "https://portal.azure.com/#/resource/subscriptions/s1"
        );
    }

    #[test]
    fn policy_definitions_show_effect_and_category() {
        let doc = serde_json::json!({
            "name": "abc", "type": "Microsoft.Authorization/policyDefinitions",
            "properties": {
                "displayName": "Key vaults should have purge protection",
                "policyType": "BuiltIn", "mode": "Indexed",
                "metadata": {"category": "Key Vault", "version": "2.1.0"},
                "parameters": {"effect": {"defaultValue": "Audit",
                    "allowedValues": ["Audit", "Deny", "Disabled"]}},
                "policyRule": {"then": {"effect": "[parameters('effect')]"}}
            }
        });
        let section = super::policy_definition_section(&doc);
        let get = |label: &str| {
            section
                .rows
                .iter()
                .find(|(l, _)| l == label)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("Type"), Some("BuiltIn"));
        assert_eq!(get("Category"), Some("Key Vault"));
        assert_eq!(
            get("Effect"),
            Some("Audit (allowed: Audit, Deny, Disabled)")
        );
    }

    #[test]
    fn directory_objects_open_on_their_entra_page() {
        let url = |kind: &str| super::entra_portal_url(kind, "id-1", "app-1", Some("t-1"));
        assert_eq!(
            url("group").unwrap(),
            "https://portal.azure.com/#@t-1/view/Microsoft_AAD_IAM/GroupDetailsMenuBlade/~/Overview/groupId/id-1"
        );
        assert!(url("user")
            .unwrap()
            .contains("UserProfileMenuBlade/~/overview/userId/id-1"));
        assert!(url("servicePrincipal")
            .unwrap()
            .ends_with("objectId/id-1/appId/app-1"));
        assert_eq!(url("device"), None);
    }
}
