//! Azure Cost Management: the cost of a scope over time.
//!
//! One call to the query API returns cost per day (or month), optionally
//! split by a dimension (resource group, resource type, service...):
//! `POST {endpoint}{scope}/providers/Microsoft.CostManagement/query`.
//! The scope is a subscription, a resource group or a management group.
//! Results are a table (`properties.columns`, `properties.rows`), paged by
//! `properties.nextLink` (POSTed again with the same body).
//!
//! The API is heavily throttled; a 429 names when to retry in one of its
//! `x-ms-ratelimit-microsoft.costmanagement-*-retry-after` headers.

use crate::resource_graph::TokenSource;
use crate::{timing, AzureError, Result};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

const API_VERSION: &str = "2023-11-01";
const MAX_RETRIES: u32 = 4;
/// Longest wait for one retry, whatever the server asks.
const MAX_WAIT: Duration = Duration::from_secs(60);
/// More pages than this is not a cost report a terminal can show.
const MAX_PAGES: usize = 50;

/// A cost query: one scope, one time range, one granularity, optionally
/// grouped by one dimension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostQuery {
    /// An ARM scope: `/subscriptions/{id}`, `/subscriptions/{id}/resourceGroups/{rg}`
    /// or `/providers/Microsoft.Management/managementGroups/{id}`.
    pub scope: String,
    /// Amortized (reservations and savings plans spread over their term)
    /// rather than actual cost.
    pub amortized: bool,
    /// First day, `YYYY-MM-DD`.
    pub from: String,
    /// Last day (included), `YYYY-MM-DD`.
    pub to: String,
    /// `"Daily"` or `"Monthly"`.
    pub granularity: &'static str,
    /// A dimension (`ResourceGroupName`, `ResourceType`, `ServiceName`,
    /// `SubscriptionId`...), or none for totals.
    pub group_by: Option<String>,
}

impl CostQuery {
    /// The request body.
    pub fn body(&self) -> Value {
        let mut dataset = json!({
            "granularity": self.granularity,
            "aggregation": {
                "totalCost": {"name": "Cost", "function": "Sum"},
                "totalCostUSD": {"name": "CostUSD", "function": "Sum"},
            },
        });
        if let Some(dimension) = &self.group_by {
            dataset["grouping"] = json!([{"type": "Dimension", "name": dimension}]);
        }
        json!({
            "type": if self.amortized { "AmortizedCost" } else { "ActualCost" },
            "timeframe": "Custom",
            "timePeriod": {
                "from": format!("{}T00:00:00Z", self.from),
                "to": format!("{}T23:59:59Z", self.to),
            },
            "dataset": dataset,
        })
    }
}

/// The answer: column names and rows, every page together.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CostTable {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

pub struct CostClient {
    http: reqwest::Client,
    endpoint: String,
    tokens: Arc<dyn TokenSource>,
}

impl CostClient {
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

    /// Run `query`, following `nextLink` until the table is complete.
    pub async fn query(&self, query: &CostQuery) -> Result<CostTable> {
        let _t = timing::timer("azure.cost_query");
        let scope = crate::client::arm_path(&query.scope)?;
        let mut url = format!(
            "{}{}/providers/Microsoft.CostManagement/query?api-version={}",
            self.endpoint,
            scope.trim_end_matches('/'),
            API_VERSION
        );
        let body = query.body();
        let mut table = CostTable::default();
        for _ in 0..MAX_PAGES {
            let page = self.post(&url, &body).await?;
            let properties = page.get("properties").unwrap_or(&Value::Null);
            if table.columns.is_empty() {
                table.columns = properties
                    .get("columns")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|c| c.get("name").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect();
            }
            table.rows.extend(
                properties
                    .get("rows")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|r| r.as_array().cloned()),
            );
            match properties.get("nextLink").and_then(Value::as_str) {
                Some(next) if !next.is_empty() => {
                    url = crate::client::next_page(next, &self.endpoint)?
                }
                _ => return Ok(table),
            }
        }
        warn!("Cost query stopped after {MAX_PAGES} pages");
        Ok(table)
    }

    async fn post(&self, url: &str, body: &Value) -> Result<Value> {
        let mut attempt = 0;
        loop {
            let token = self.tokens.token().await?;
            let response = self
                .http
                .post(url)
                .bearer_auth(token)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.to_string())
                .send()
                .await
                .map_err(|e| {
                    AzureError::ApiError(format!("Cost Management request failed: {e}"))
                })?;
            let status = response.status();
            let retry_after = retry_after(response.headers());
            let text = response
                .text()
                .await
                .map_err(|e| AzureError::ApiError(format!("Cost Management response: {e}")))?;
            if status.as_u16() == 429 && attempt < MAX_RETRIES {
                attempt += 1;
                let wait = retry_after
                    .unwrap_or(Duration::from_secs(2 << attempt))
                    .min(MAX_WAIT);
                warn!("Cost Management throttled; retrying in {:?}", wait);
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
                AzureError::ApiError(format!("Invalid Cost Management response: {e}"))
            });
        }
    }
}

/// The longest wait any `*retry-after` header asks for (Cost Management
/// sends one per quota: `x-ms-ratelimit-microsoft.costmanagement-qpu-retry-after`...).
fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .iter()
        .filter(|(name, _)| name.as_str().ends_with("retry-after"))
        .filter_map(|(_, v)| v.to_str().ok()?.trim().parse::<u64>().ok())
        .max()
        .map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{error, ok, serve_paths, Reply, StaticToken};
    use std::sync::Mutex;

    fn client(endpoint: &str) -> CostClient {
        CostClient::with_endpoint(endpoint, Arc::new(StaticToken("t".into())))
    }

    fn query() -> CostQuery {
        CostQuery {
            scope: "/subscriptions/s1".into(),
            amortized: false,
            from: "2026-10-12".into(),
            to: "2026-11-10".into(),
            granularity: "Daily",
            group_by: Some("ResourceGroupName".into()),
        }
    }

    #[test]
    fn the_body_asks_for_cost_and_usd_grouped() {
        let body = query().body();
        assert_eq!(body["type"], "ActualCost");
        assert_eq!(body["timePeriod"]["from"], "2026-10-12T00:00:00Z");
        assert_eq!(body["timePeriod"]["to"], "2026-11-10T23:59:59Z");
        assert_eq!(body["dataset"]["granularity"], "Daily");
        assert_eq!(
            body["dataset"]["aggregation"]["totalCostUSD"]["name"],
            "CostUSD"
        );
        assert_eq!(body["dataset"]["grouping"][0]["name"], "ResourceGroupName");

        let amortized = CostQuery {
            amortized: true,
            group_by: None,
            ..query()
        }
        .body();
        assert_eq!(amortized["type"], "AmortizedCost");
        assert!(amortized["dataset"].get("grouping").is_none());
    }

    #[tokio::test]
    async fn pages_are_joined_and_throttling_is_retried() {
        let me = Arc::new(Mutex::new(String::new()));
        let base = me.clone();
        let calls = Arc::new(Mutex::new(0));
        let seen = calls.clone();
        let (endpoint, requests) = serve_paths(move |path, _| {
            assert!(
                path.starts_with(
                    "/subscriptions/s1/providers/Microsoft.CostManagement/query?api-version=2023-11-01"
                ),
                "{path}"
            );
            let mut n = seen.lock().unwrap();
            *n += 1;
            if *n == 1 {
                return Reply {
                    status: 429,
                    headers: vec![(
                        "x-ms-ratelimit-microsoft.costmanagement-qpu-retry-after",
                        "0",
                    )],
                    body: String::new(),
                    delay: Default::default(),
                };
            }
            let columns = json!([{"name": "Cost"}, {"name": "CostUSD"}, {"name": "UsageDate"},
                {"name": "ResourceGroupName"}, {"name": "Currency"}]);
            if path.contains("skiptoken") {
                return ok(json!({"properties": {"columns": columns,
                    "rows": [[2.0, 2.2, 20261102, "rg-b", "EUR"]]}}));
            }
            let next = format!(
                "{}/subscriptions/s1/providers/Microsoft.CostManagement/query?api-version=2023-11-01&$skiptoken=x",
                base.lock().unwrap()
            );
            ok(json!({"properties": {"columns": columns, "nextLink": next,
                "rows": [[1.0, 1.1, 20261101, "rg-a", "EUR"]]}}))
        })
        .await;
        *me.lock().unwrap() = endpoint.clone();
        let table = client(&endpoint).query(&query()).await.unwrap();
        assert_eq!(table.columns[0], "Cost");
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[1][3], "rg-b");
        assert_eq!(*calls.lock().unwrap(), 3, "throttled once, then two pages");
        // Every page is asked with the same body.
        let bodies = requests.lock().unwrap();
        assert!(bodies.iter().all(|b| b["type"] == "ActualCost"));
    }

    #[tokio::test]
    async fn errors_carry_the_api_message() {
        let (endpoint, _) = serve_paths(|_, _| {
            error(
                400,
                "Cost Management supports only Enterprise Agreement, Web direct and Microsoft Customer Agreement offer types.",
            )
        })
        .await;
        let e = client(&endpoint).query(&query()).await.unwrap_err();
        assert!(e.to_string().contains("offer types"), "{e}");
    }

    #[tokio::test]
    async fn a_scope_that_is_not_an_arm_path_is_refused() {
        let e = client("http://127.0.0.1:9")
            .query(&CostQuery {
                scope: "https://evil.example/x".into(),
                ..query()
            })
            .await
            .unwrap_err();
        assert!(!e.to_string().is_empty());
    }
}
