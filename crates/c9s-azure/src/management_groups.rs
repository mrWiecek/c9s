//! Management group hierarchy.
//!
//! One call to the descendants API returns every management group and
//! subscription below a group, each with its parent, paged by `nextLink`:
//! `GET {endpoint}/providers/Microsoft.Management/managementGroups/{group}/descendants`.
//! That is exactly what the scope tree needs (tenant root → management
//! groups → subscriptions). Reading it needs read access on the starting
//! group; platform engineers usually have it at the tenant root.

use crate::resource_graph::TokenSource;
use crate::{timing, AzureError, Result};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

const API_VERSION: &str = "2020-05-01";
const MAX_RETRIES: u32 = 4;

/// A tenant the signed-in identity can access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantInfo {
    pub id: String,
    pub display_name: String,
    pub default_domain: Option<String>,
}

const TENANTS_API_VERSION: &str = "2022-12-01";
const POLICY_API_VERSION: &str = "2023-04-01";
const ROLE_API_VERSION: &str = "2022-04-01";
/// Privileged Identity Management (role eligibility).
const PIM_API_VERSION: &str = "2020-10-01";

/// What a node in the hierarchy is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    ManagementGroup,
    Subscription,
}

/// A management group or subscription with its parent group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HierarchyNode {
    pub kind: NodeKind,
    /// Management group name (ID segment) or subscription ID (lowercase).
    pub name: String,
    pub display_name: String,
    /// Name of the parent management group.
    pub parent: Option<String>,
}

pub struct ManagementGroupClient {
    http: reqwest::Client,
    endpoint: String,
    tokens: Arc<dyn TokenSource>,
}

impl ManagementGroupClient {
    pub fn new(tokens: Arc<dyn TokenSource>) -> Self {
        Self::with_endpoint(crate::resource_graph::DEFAULT_ENDPOINT, tokens)
    }

    pub fn with_endpoint(endpoint: impl Into<String>, tokens: Arc<dyn TokenSource>) -> Self {
        Self {
            http: reqwest::Client::new(),
            endpoint: endpoint.into().trim_end_matches('/').to_string(),
            tokens,
        }
    }

    /// Display name of one management group (e.g. "Tenant Root Group").
    pub async fn display_name(&self, group: &str) -> Result<String> {
        let url = format!(
            "{}/providers/Microsoft.Management/managementGroups/{}?api-version={}",
            self.endpoint, group, API_VERSION
        );
        let value = self.get(&url).await?;
        Ok(value
            .pointer("/properties/displayName")
            .and_then(Value::as_str)
            .unwrap_or(group)
            .to_string())
    }

    /// Every management group and subscription below `group`.
    pub async fn descendants(&self, group: &str) -> Result<Vec<HierarchyNode>> {
        let _t = timing::timer("azure.management_groups");
        let mut url = format!(
            "{}/providers/Microsoft.Management/managementGroups/{}/descendants?api-version={}",
            self.endpoint, group, API_VERSION
        );
        let mut nodes = Vec::new();
        loop {
            let page = self.get(&url).await?;
            if let Some(items) = page.get("value").and_then(Value::as_array) {
                nodes.extend(items.iter().filter_map(node_from_item));
            }
            match page.get("nextLink").and_then(Value::as_str) {
                Some(next) if !next.is_empty() => {
                    url = crate::client::next_page(next, &self.endpoint)?
                }
                _ => break,
            }
        }
        Ok(nodes)
    }

    /// The tenants the signed-in identity can access, with their names
    /// (`GET /tenants`, paged by `nextLink`).
    pub async fn tenants(&self) -> Result<Vec<TenantInfo>> {
        let _t = timing::timer("azure.tenants");
        let mut url = format!(
            "{}/tenants?api-version={}",
            self.endpoint, TENANTS_API_VERSION
        );
        let mut tenants = Vec::new();
        loop {
            let page = self.get(&url).await?;
            if let Some(items) = page.get("value").and_then(Value::as_array) {
                tenants.extend(items.iter().filter_map(|item| {
                    let id = item.get("tenantId").and_then(Value::as_str)?.to_string();
                    let text = |key: &str| {
                        item.get(key)
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                    };
                    Some(TenantInfo {
                        display_name: text("displayName").unwrap_or_else(|| id.clone()),
                        default_domain: text("defaultDomain"),
                        id,
                    })
                }));
            }
            match page.get("nextLink").and_then(Value::as_str) {
                Some(next) if !next.is_empty() => {
                    url = crate::client::next_page(next, &self.endpoint)?
                }
                _ => break,
            }
        }
        Ok(tenants)
    }

    /// A policy definition or initiative (set definition) by its ARM ID,
    /// built-in or custom: `GET {endpoint}{id}`. Resource Graph may not
    /// list built-in definitions, which ARM always serves.
    pub async fn policy_object(&self, id: &str) -> Result<Value> {
        let _t = timing::timer("azure.policy_definition");
        let lower = id.to_ascii_lowercase();
        if !lower.contains("/providers/microsoft.authorization/policydefinitions/")
            && !lower.contains("/providers/microsoft.authorization/policysetdefinitions/")
        {
            return Err(AzureError::InvalidConfig(format!(
                "not a policy definition ID: {id}"
            )));
        }
        let id = crate::client::arm_path(id)?;
        let url = format!("{}{}?api-version={}", self.endpoint, id, POLICY_API_VERSION);
        self.get(&url).await
    }

    /// Every role definition (built-in and custom) usable at `scope` (an
    /// ARM ID: a management group or subscription), from ARM, as rows like
    /// `resource_graph::role_definitions_query` returns. Resource Graph may
    /// leave out the built-in roles.
    pub async fn role_definitions(&self, scope: &str) -> Result<Vec<Value>> {
        let _t = timing::timer("azure.role_definitions_arm");
        let mut url = format!(
            "{}{}/providers/Microsoft.Authorization/roleDefinitions?api-version={}",
            self.endpoint,
            scope.trim_end_matches('/'),
            ROLE_API_VERSION
        );
        let mut rows = Vec::new();
        loop {
            let page = self.get(&url).await?;
            for item in page
                .get("value")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let p = item.get("properties");
                let get = |key: &str| p.and_then(|p| p.get(key)).cloned().unwrap_or_default();
                rows.push(serde_json::json!({
                    "id": item.get("id").cloned().unwrap_or_default(),
                    "name": item.get("name").cloned().unwrap_or_default(),
                    "roleName": get("roleName"),
                    "roleType": get("type"),
                    "description": get("description"),
                    "assignableScopes": get("assignableScopes"),
                    "permissions": get("permissions"),
                }));
            }
            match page.get("nextLink").and_then(Value::as_str) {
                Some(next) if !next.is_empty() => {
                    url = crate::client::next_page(next, &self.endpoint)?
                }
                _ => return Ok(rows),
            }
        }
    }

    /// PIM: role eligibility instances at, above and below `scope` (an ARM
    /// ID: the tenant root group, a management group or a subscription),
    /// as ARM returns them (`properties.principalId`, `roleDefinitionId`,
    /// `scope`, `endDateTime`, `memberType`...). Reader can read them.
    pub async fn role_eligibilities(&self, scope: &str) -> Result<Vec<Value>> {
        let _t = timing::timer("azure.role_eligibilities");
        let mut url = format!(
            "{}{}/providers/Microsoft.Authorization/roleEligibilityScheduleInstances?api-version={}",
            self.endpoint,
            scope.trim_end_matches('/'),
            PIM_API_VERSION
        );
        let mut items = Vec::new();
        loop {
            let page = self.get(&url).await?;
            items.extend(
                page.get("value")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
            match page.get("nextLink").and_then(Value::as_str) {
                Some(next) if !next.is_empty() => {
                    url = crate::client::next_page(next, &self.endpoint)?
                }
                _ => return Ok(items),
            }
        }
    }

    /// One role definition by any of its IDs (it can be written at any
    /// scope: `/subscriptions/…/providers/Microsoft.Authorization/roleDefinitions/<guid>`).
    pub async fn role_definition(&self, id: &str) -> Result<Value> {
        let _t = timing::timer("azure.role_definition");
        if !id
            .to_ascii_lowercase()
            .contains("/providers/microsoft.authorization/roledefinitions/")
        {
            return Err(AzureError::InvalidConfig(format!(
                "not a role definition ID: {id}"
            )));
        }
        let id = crate::client::arm_path(id)?;
        let url = format!("{}{}?api-version={}", self.endpoint, id, ROLE_API_VERSION);
        self.get(&url).await
    }

    async fn get(&self, url: &str) -> Result<Value> {
        let mut attempt = 0;
        loop {
            let token = self.tokens.token().await?;
            let response = self
                .http
                .get(url)
                .bearer_auth(token)
                .send()
                .await
                .map_err(|e| {
                    AzureError::ApiError(format!("Management groups request failed: {e}"))
                })?;
            let status = response.status();
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(Duration::from_secs);
            let text = response
                .text()
                .await
                .map_err(|e| AzureError::ApiError(format!("Management groups response: {e}")))?;
            if status.as_u16() == 429 && attempt < MAX_RETRIES {
                attempt += 1;
                let wait = retry_after.unwrap_or(Duration::from_secs(1 << attempt));
                warn!("Management groups API throttled; retrying in {:?}", wait);
                tokio::time::sleep(wait).await;
                continue;
            }
            if !status.is_success() {
                let message = serde_json::from_str::<Value>(&text)
                    .ok()
                    .and_then(|v| {
                        v.pointer("/error/message")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| text.chars().take(300).collect());
                return Err(AzureError::Http {
                    status: status.as_u16(),
                    message,
                });
            }
            return serde_json::from_str(&text).map_err(|e| {
                AzureError::ApiError(format!("Invalid management groups response: {e}"))
            });
        }
    }
}

/// Last segment of an ARM ID: ".../managementGroups/mg-1" -> "mg-1".
fn last_segment(id: &str) -> Option<&str> {
    id.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
}

fn node_from_item(item: &Value) -> Option<HierarchyNode> {
    let type_ = item.get("type").and_then(Value::as_str)?;
    let kind = if type_.eq_ignore_ascii_case("Microsoft.Management/managementGroups") {
        NodeKind::ManagementGroup
    } else if type_.to_ascii_lowercase().ends_with("/subscriptions") {
        // "/subscriptions" (descendants API), or the long form
        // "Microsoft.Management/managementGroups/subscriptions".
        NodeKind::Subscription
    } else {
        return None;
    };
    let mut name = item.get("name").and_then(Value::as_str)?.to_string();
    if kind == NodeKind::Subscription {
        name = name.to_ascii_lowercase();
    }
    let display_name = item
        .pointer("/properties/displayName")
        .and_then(Value::as_str)
        .unwrap_or(&name)
        .to_string();
    let parent = item
        .pointer("/properties/parent/id")
        .and_then(Value::as_str)
        .and_then(last_segment)
        .map(str::to_string);
    Some(HierarchyNode {
        kind,
        name,
        display_name,
        parent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{error, ok, serve_paths, StaticToken};
    use serde_json::json;

    fn group(name: &str, display: &str, parent: &str) -> Value {
        json!({
            "id": format!("/providers/Microsoft.Management/managementGroups/{name}"),
            "type": "Microsoft.Management/managementGroups",
            "name": name,
            "properties": {
                "displayName": display,
                "parent": {"id": format!("/providers/Microsoft.Management/managementGroups/{parent}")}
            }
        })
    }

    fn subscription(id: &str, display: &str, parent: &str) -> Value {
        json!({
            "id": format!("/subscriptions/{id}"),
            "type": "/subscriptions",
            "name": id,
            "properties": {
                "displayName": display,
                "parent": {"id": format!("/providers/Microsoft.Management/managementGroups/{parent}")}
            }
        })
    }

    fn client(endpoint: &str) -> ManagementGroupClient {
        ManagementGroupClient::with_endpoint(endpoint, Arc::new(StaticToken("t".into())))
    }

    #[tokio::test]
    async fn policy_definitions_are_read_from_arm_by_id() {
        let id = "/providers/Microsoft.Authorization/policyDefinitions/abc";
        let (endpoint, _) = serve_paths(move |path, _| {
            assert!(
                path.starts_with(&format!("{id}?api-version=2023-04-01")),
                "{path}"
            );
            ok(json!({"id": id, "properties": {"displayName": "Require a tag"}}))
        })
        .await;
        let client = client(&endpoint);
        let doc = client.policy_object(id).await.unwrap();
        assert_eq!(doc["properties"]["displayName"], "Require a tag");
        assert!(
            client.policy_object("/subscriptions/s").await.is_err(),
            "only policy definitions and initiatives"
        );
    }

    #[tokio::test]
    async fn pim_eligibilities_are_listed_across_pages() {
        use std::sync::{Arc, Mutex};
        let me = Arc::new(Mutex::new(String::new()));
        let base = me.clone();
        let (endpoint, _) = serve_paths(move |path, _| {
            assert!(
                path.starts_with(
                    "/providers/Microsoft.Management/managementGroups/t1/providers/Microsoft.Authorization/roleEligibilityScheduleInstances?api-version=2020-10-01"
                ),
                "{path}"
            );
            if path.contains("skiptoken") {
                return ok(json!({"value": [{"id": "e2", "properties": {}}]}));
            }
            let next = format!(
                "{}/providers/Microsoft.Management/managementGroups/t1/providers/Microsoft.Authorization/roleEligibilityScheduleInstances?api-version=2020-10-01&$skiptoken=2",
                base.lock().unwrap()
            );
            ok(json!({"value": [{"id": "e1", "properties": {}}], "nextLink": next}))
        })
        .await;
        *me.lock().unwrap() = endpoint.clone();
        let items = client(&endpoint)
            .role_eligibilities("/providers/Microsoft.Management/managementGroups/t1")
            .await
            .unwrap();
        let ids: Vec<&str> = items.iter().filter_map(|i| i["id"].as_str()).collect();
        assert_eq!(ids, ["e1", "e2"]);
    }

    #[tokio::test]
    async fn role_definitions_come_from_arm_in_resource_graph_shape() {
        let (endpoint, _) = serve_paths(|path, _| {
            assert!(
                path.starts_with("/providers/Microsoft.Management/managementGroups/t/providers/Microsoft.Authorization/roleDefinitions?api-version=2022-04-01")
                    || path.starts_with("/subscriptions/s/providers/Microsoft.Authorization/roleDefinitions/abc?"),
                "{path}"
            );
            if path.contains("/roleDefinitions/abc") {
                return ok(json!({"id": "x", "properties": {"roleName": "Reader"}}));
            }
            ok(json!({"value": [{
                "id": "/providers/Microsoft.Authorization/roleDefinitions/abc", "name": "abc",
                "properties": {"roleName": "Reader", "type": "BuiltInRole",
                    "assignableScopes": ["/"], "permissions": [{"actions": ["*/read"]}]}
            }]}))
        })
        .await;
        let client = client(&endpoint);
        let rows = client
            .role_definitions("/providers/Microsoft.Management/managementGroups/t")
            .await
            .unwrap();
        assert_eq!(rows[0]["roleName"], "Reader");
        assert_eq!(rows[0]["roleType"], "BuiltInRole");
        assert_eq!(rows[0]["permissions"][0]["actions"][0], "*/read");
        let one = client
            .role_definition(
                "/subscriptions/s/providers/Microsoft.Authorization/roleDefinitions/abc",
            )
            .await
            .unwrap();
        assert_eq!(one["properties"]["roleName"], "Reader");
        assert!(client.role_definition("/subscriptions/s").await.is_err());
    }

    #[tokio::test]
    async fn tenants_are_listed_with_names_and_domains() {
        let (endpoint, _) = serve_paths(|path, _| {
            assert!(path.starts_with("/tenants?api-version="), "{path}");
            ok(json!({"value": [
                {"tenantId": "t-1", "displayName": "Contoso", "defaultDomain": "contoso.onmicrosoft.com"},
                {"tenantId": "t-2"},
                {"displayName": "no id: skipped"}
            ]}))
        })
        .await;
        let tenants = client(&endpoint).tenants().await.unwrap();
        assert_eq!(
            tenants,
            [
                TenantInfo {
                    id: "t-1".into(),
                    display_name: "Contoso".into(),
                    default_domain: Some("contoso.onmicrosoft.com".into()),
                },
                TenantInfo {
                    id: "t-2".into(),
                    display_name: "t-2".into(),
                    default_domain: None,
                },
            ]
        );
    }

    #[tokio::test]
    async fn descendants_follow_next_link_and_parse_parents() {
        // nextLink is absolute, so the reply needs the server's own address,
        // which is only known once it is listening.
        let base = Arc::new(std::sync::Mutex::new(String::new()));
        let base_in_server = base.clone();
        let (endpoint, _) = serve_paths(move |path, _| {
            if path.contains("page=2") {
                return ok(json!({"value": [subscription("SUB-B", "Prod B", "platform")]}));
            }
            assert!(
                path.starts_with(
                    "/providers/Microsoft.Management/managementGroups/tenant-1/descendants"
                ),
                "{path}"
            );
            let next = format!(
                "{}/providers/Microsoft.Management/managementGroups/tenant-1/descendants?page=2",
                base_in_server.lock().unwrap()
            );
            ok(json!({
                "value": [
                    group("platform", "Platform", "tenant-1"),
                    subscription("sub-a", "Prod A", "platform"),
                    {"type": "something/else", "name": "x"}
                ],
                "nextLink": next
            }))
        })
        .await;
        *base.lock().unwrap() = endpoint.clone();

        let nodes = client(&endpoint).descendants("tenant-1").await.unwrap();
        assert_eq!(
            nodes,
            vec![
                HierarchyNode {
                    kind: NodeKind::ManagementGroup,
                    name: "platform".into(),
                    display_name: "Platform".into(),
                    parent: Some("tenant-1".into()),
                },
                HierarchyNode {
                    kind: NodeKind::Subscription,
                    name: "sub-a".into(),
                    display_name: "Prod A".into(),
                    parent: Some("platform".into()),
                },
                HierarchyNode {
                    kind: NodeKind::Subscription,
                    name: "sub-b".into(),
                    display_name: "Prod B".into(),
                    parent: Some("platform".into()),
                },
            ],
            "unknown types skipped, subscription IDs lowercased, second page followed"
        );
    }

    #[tokio::test]
    async fn display_name_and_errors() {
        let (endpoint, _) = serve_paths(|path, _| {
            if path.contains("/managementGroups/tenant-1?") {
                ok(json!({"name": "tenant-1", "properties": {"displayName": "Tenant Root Group"}}))
            } else {
                error(403, "no access")
            }
        })
        .await;
        let c = client(&endpoint);
        assert_eq!(
            c.display_name("tenant-1").await.unwrap(),
            "Tenant Root Group"
        );
        match c.descendants("tenant-1").await.unwrap_err() {
            AzureError::Http { status, message } => {
                assert_eq!(status, 403);
                assert_eq!(message, "no access");
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
