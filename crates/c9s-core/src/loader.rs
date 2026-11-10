//! Background loading of Azure data.
//!
//! The UI never awaits the network. It sends a [`LoadRequest`] to the
//! [`Loader`], which runs it on a tokio task and delivers a [`LoadResult`] over
//! a channel that the event loop polls alongside terminal input.
//!
//! Each request gets a generation number. Starting a new request of the same
//! [`LoadKind`] aborts the previous task, and a result that is no longer the
//! latest for its kind is discarded by [`Loader::accept`], so a slow answer
//! for a view the user already left can never overwrite a newer one.

use crate::models::{Resource, ResourceGroup, Subscription};
use crate::policy::{
    compliance_from_rows, subscription_compliance_from_rows, Compliance, PolicyAssignment,
    PolicyExemption, PolicyScope, PolicyState,
};
use crate::scope_tree::ScopeTree;
use crate::timing;
use c9s_azure::management_groups::ManagementGroupClient;
use c9s_azure::resource_graph::{self, GraphScope, ResourceGraphClient};
use c9s_azure::{AzureClient, AzureError};
use futures::future::BoxFuture;
use futures::{stream, FutureExt, StreamExt};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::warn;

/// Maximum concurrent ARM calls when loading many subscriptions at once.
pub const MAX_CONCURRENT_REQUESTS: usize = 16;

/// What to load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadRequest {
    Subscriptions,
    ResourceGroups {
        subscription_ids: Vec<String>,
    },
    Resources {
        subscription_ids: Vec<String>,
        /// Only resources in this resource group (requires exactly one subscription).
        resource_group: Option<String>,
    },
    /// The management group tree below the tenant root.
    ScopeTree,
    /// One resource's full document.
    ResourceDetail {
        resource_id: String,
        subscription_id: String,
    },
    /// The tenants the user can access, with names.
    Tenants,
    /// Contexts of other clouds' CLIs (gcloud configurations).
    CloudContexts,
    /// Every policy assignment in the tenant.
    PolicyAssignments,
    /// Every policy exemption in the tenant.
    PolicyExemptions,
    /// Every role assignment in the tenant (these subscriptions' if the
    /// tenant root isn't readable).
    RoleAssignments {
        subscription_ids: Vec<String>,
    },
    /// Every role definition, built-in and custom (as above).
    RoleDefinitions {
        subscription_ids: Vec<String>,
    },
    /// Names of these principal IDs, from Microsoft Graph.
    Principals {
        ids: Vec<String>,
    },
    /// The groups a principal belongs to, nested ones too (Microsoft Graph).
    PrincipalGroups {
        principal_id: String,
    },
    /// Every group in the directory (Microsoft Graph).
    Groups,
    /// Every app registration and enterprise application (Microsoft Graph).
    Applications,
    /// One application's owners, grants, assignments and APIs.
    AppDetail {
        app_id: String,
        application_id: Option<String>,
        service_principal_id: Option<String>,
    },
    /// A group's members, direct or (`transitive`) nested too.
    GroupMembers {
        group_id: String,
        transitive: bool,
    },
    /// Resources evaluated / non-compliant per subscription, tenant-wide:
    /// the compliance roll-up of the management group tree and `:compliance`.
    ComplianceRollup,
    /// Compliance counts per assignment for resources in a scope.
    PolicyCompliance {
        scope: PolicyScope,
    },
    /// Resources evaluated / non-compliant per subscription in a scope.
    ComplianceSummary {
        scope: PolicyScope,
    },
    /// Non-compliant policy states of one assignment, for resources in a scope.
    PolicyStates {
        scope: PolicyScope,
        assignment_id: String,
    },
    /// Every policy state of one resource (an empty subscription searches
    /// the whole tenant).
    ResourcePolicy {
        resource_id: String,
        subscription_id: String,
    },
    /// What a scope cost over a period (Cost Management).
    Cost {
        query: crate::cost::CostQuery,
    },
}

/// Which list a request fills. One request per kind is in flight at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LoadKind {
    Subscriptions,
    ResourceGroups,
    Resources,
    ScopeTree,
    ResourceDetail,
    Tenants,
    CloudContexts,
    PolicyAssignments,
    PolicyExemptions,
    RoleAssignments,
    RoleDefinitions,
    Principals,
    PrincipalGroups,
    Groups,
    GroupMembers,
    Applications,
    AppDetail,
    ComplianceRollup,
    PolicyCompliance,
    ComplianceSummary,
    PolicyStates,
    ResourcePolicy,
    Cost,
}

impl LoadKind {
    pub fn label(self) -> &'static str {
        match self {
            LoadKind::Subscriptions => "subscriptions",
            LoadKind::ResourceGroups => "resource groups",
            LoadKind::Resources => "resources",
            LoadKind::ScopeTree => "management group tree",
            LoadKind::ResourceDetail => "resource details",
            LoadKind::Tenants => "tenants",
            LoadKind::CloudContexts => "cloud contexts",
            LoadKind::PolicyAssignments => "policy assignments",
            LoadKind::PolicyExemptions => "policy exemptions",
            LoadKind::RoleAssignments => "role assignments",
            LoadKind::RoleDefinitions => "role definitions",
            LoadKind::Principals => "principal names",
            LoadKind::PrincipalGroups => "group memberships",
            LoadKind::Groups => "groups",
            LoadKind::Applications => "applications",
            LoadKind::AppDetail => "application details",
            LoadKind::GroupMembers => "group members",
            LoadKind::ComplianceRollup => "compliance per subscription",
            LoadKind::PolicyCompliance => "policy compliance",
            LoadKind::ComplianceSummary => "compliance summary",
            LoadKind::PolicyStates => "non-compliant resources",
            LoadKind::ResourcePolicy => "resource policy states",
            LoadKind::Cost => "cost",
        }
    }
}

impl LoadRequest {
    pub fn kind(&self) -> LoadKind {
        match self {
            LoadRequest::Subscriptions => LoadKind::Subscriptions,
            LoadRequest::ResourceGroups { .. } => LoadKind::ResourceGroups,
            LoadRequest::Resources { .. } => LoadKind::Resources,
            LoadRequest::ScopeTree => LoadKind::ScopeTree,
            LoadRequest::ResourceDetail { .. } => LoadKind::ResourceDetail,
            LoadRequest::Tenants => LoadKind::Tenants,
            LoadRequest::CloudContexts => LoadKind::CloudContexts,
            LoadRequest::PolicyAssignments => LoadKind::PolicyAssignments,
            LoadRequest::PolicyExemptions => LoadKind::PolicyExemptions,
            LoadRequest::RoleAssignments { .. } => LoadKind::RoleAssignments,
            LoadRequest::RoleDefinitions { .. } => LoadKind::RoleDefinitions,
            LoadRequest::Principals { .. } => LoadKind::Principals,
            LoadRequest::PrincipalGroups { .. } => LoadKind::PrincipalGroups,
            LoadRequest::Groups => LoadKind::Groups,
            LoadRequest::Applications => LoadKind::Applications,
            LoadRequest::AppDetail { .. } => LoadKind::AppDetail,
            LoadRequest::GroupMembers { .. } => LoadKind::GroupMembers,
            LoadRequest::ComplianceRollup => LoadKind::ComplianceRollup,
            LoadRequest::PolicyCompliance { .. } => LoadKind::PolicyCompliance,
            LoadRequest::ComplianceSummary { .. } => LoadKind::ComplianceSummary,
            LoadRequest::PolicyStates { .. } => LoadKind::PolicyStates,
            LoadRequest::ResourcePolicy { .. } => LoadKind::ResourcePolicy,
            LoadRequest::Cost { .. } => LoadKind::Cost,
        }
    }
}

/// The other clouds' contexts for `:ctx`: gcloud configurations and AWS
/// CLI profiles.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CloudContexts {
    pub gcp: Vec<c9s_gcp::Configuration>,
    pub aws: Vec<c9s_aws::Profile>,
}

/// gcloud's configurations and the AWS CLI's profiles. A missing CLI is
/// no error while the other cloud answers.
pub async fn cloud_contexts() -> Result<CloudContexts, String> {
    let aws = c9s_aws::profiles();
    match c9s_gcp::configurations("gcloud").await {
        Ok(gcp) => Ok(CloudContexts { gcp, aws }),
        Err(e) if !aws.is_empty() => {
            tracing::debug!("No gcloud configurations: {e}");
            Ok(CloudContexts { gcp: vec![], aws })
        }
        Err(e) => Err(e.to_string()),
    }
}

/// The fetcher without the Azure CLI: it lists the other clouds'
/// contexts (`:ctx`) and says how to sign in for everything else.
pub struct ContextsOnly;

/// What Azure requests say without the Azure CLI.
pub const NOT_SIGNED_IN: &str = "the Azure CLI (az) was not found: install it, run `az login` and \
     restart, or pick a Google Cloud or AWS context with :ctx";

impl Fetcher for ContextsOnly {
    fn fetch(&self, request: LoadRequest, _sink: ChunkSink) -> BoxFuture<'static, FetchOutcome> {
        async move {
            let data = match request {
                LoadRequest::CloudContexts => cloud_contexts().await.map(LoadedData::CloudContexts),
                // No Azure tenants to list; the other clouds' contexts are.
                LoadRequest::Tenants => Ok(LoadedData::Tenants(vec![])),
                _ => Err(NOT_SIGNED_IN.to_string()),
            };
            FetchOutcome {
                data,
                failed_subscriptions: vec![],
                notice: None,
            }
        }
        .boxed()
    }
}

#[derive(Debug)]
pub enum LoadedData {
    Subscriptions(Vec<Subscription>),
    ResourceGroups(Vec<ResourceGroup>),
    Resources(Vec<Resource>),
    ScopeTree(ScopeTree),
    /// A resource's document (`Null` only as an empty placeholder).
    ResourceDetail(serde_json::Value),
    Tenants(Vec<crate::models::Tenant>),
    CloudContexts(CloudContexts),
    PolicyAssignments(Vec<PolicyAssignment>),
    PolicyExemptions(Vec<PolicyExemption>),
    RoleAssignments(Vec<crate::rbac::RoleAssignment>),
    RoleDefinitions(Vec<crate::rbac::RoleDefinition>),
    Principals(Vec<c9s_azure::directory::Principal>),
    /// A principal (lowercase ID) and its groups
    PrincipalGroups(String, Vec<c9s_azure::directory::Principal>),
    Groups(Vec<c9s_azure::directory::Group>),
    Applications(
        Vec<c9s_azure::apps::Application>,
        Vec<c9s_azure::apps::ServicePrincipal>,
    ),
    /// An application (its app ID) and its details
    AppDetail(String, Box<c9s_azure::apps::AppDetail>),
    /// A group (lowercase ID), direct or transitive, and its members
    GroupMembers(String, bool, Vec<c9s_azure::directory::Principal>),
    /// Per subscription (lowercase ID), tenant-wide
    ComplianceRollup(HashMap<String, Compliance>),
    PolicyCompliance(PolicyScope, HashMap<String, Compliance>),
    /// Per subscription (lowercase ID)
    ComplianceSummary(PolicyScope, HashMap<String, Compliance>),
    PolicyStates(Vec<PolicyState>),
    /// The resource ID and its policy states.
    ResourcePolicy(String, Vec<PolicyState>),
    Cost(Box<crate::cost::CostReport>),
}

/// Outcome of a fetch: the data (or an error), plus the subscriptions that
/// failed when several were loaded at once.
#[derive(Debug)]
pub struct FetchOutcome {
    /// Shown to the user in the status bar (e.g. "fell back to ARM").
    pub notice: Option<String>,
    pub data: Result<LoadedData, String>,
    pub failed_subscriptions: Vec<String>,
}

#[derive(Debug)]
pub struct LoadResult {
    /// `false` for a partial chunk streamed while the load continues; `true`
    /// for the final message of a request.
    pub done: bool,
    pub kind: LoadKind,
    pub generation: u64,
    pub outcome: FetchOutcome,
}

/// Source of data. [`AzureFetcher`] talks to ARM; tests use fakes.
/// Lets a fetcher deliver rows as they arrive (e.g. one Resource Graph page at
/// a time), so the first page is visible before the whole load finishes.
#[derive(Clone)]
pub struct ChunkSink {
    tx: mpsc::UnboundedSender<LoadResult>,
    kind: LoadKind,
    generation: u64,
}

impl ChunkSink {
    /// Deliver a chunk of rows. The final [`FetchOutcome`] then carries only
    /// rows not already sent through the sink.
    pub fn send(&self, data: LoadedData) {
        let _ = self.tx.send(LoadResult {
            done: false,
            kind: self.kind,
            generation: self.generation,
            outcome: FetchOutcome {
                data: Ok(data),
                failed_subscriptions: Vec::new(),
                notice: None,
            },
        });
    }

    /// A sink whose chunks go nowhere (for fetchers called outside a Loader).
    pub fn discard() -> Self {
        let (tx, _) = mpsc::unbounded_channel();
        Self {
            tx,
            kind: LoadKind::Subscriptions,
            generation: 0,
        }
    }
}

/// How the loader treats an incoming [`LoadResult`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accept {
    /// A newer request of the same kind exists; ignore it.
    Stale,
    /// A chunk of the current request; more will follow.
    Partial,
    /// The last message of the current request.
    Final,
}

pub trait Fetcher: Send + Sync + 'static {
    fn fetch(&self, request: LoadRequest, _sink: ChunkSink) -> BoxFuture<'static, FetchOutcome>;

    /// Start slow one-time setup (e.g. getting a token) in the background, so
    /// the first real request doesn't pay for it. Must not block.
    fn warm_up(&self) {}
}

pub type LoadReceiver = mpsc::UnboundedReceiver<LoadResult>;

pub struct Loader {
    fetcher: Option<Arc<dyn Fetcher>>,
    tx: mpsc::UnboundedSender<LoadResult>,
    rx: Option<LoadReceiver>,
    next_generation: u64,
    in_flight: HashMap<LoadKind, (u64, JoinHandle<()>)>,
}

impl Loader {
    /// `None` means offline: every request fails immediately with an error.
    pub fn new(fetcher: Option<Arc<dyn Fetcher>>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            fetcher,
            tx,
            rx: Some(rx),
            next_generation: 0,
            in_flight: HashMap::new(),
        }
    }

    /// Use `fetcher` from now on (e.g. after switching tenant). Aborts every
    /// request in flight; their results would belong to the old tenant.
    pub fn replace_fetcher(&mut self, fetcher: Option<Arc<dyn Fetcher>>) {
        for (_, (_, handle)) in self.in_flight.drain() {
            handle.abort();
        }
        self.fetcher = fetcher;
    }

    /// The receiving end of the result channel. Can be taken once.
    pub fn take_receiver(&mut self) -> Option<LoadReceiver> {
        self.rx.take()
    }

    /// Start `request` in the background, cancelling any in-flight request of
    /// the same kind. Returns the request's generation.
    pub fn request(&mut self, request: LoadRequest) -> Result<u64, String> {
        let fetcher = self
            .fetcher
            .clone()
            .ok_or_else(|| "Not connected to Azure (run `az login` and press r)".to_string())?;

        let kind = request.kind();
        self.next_generation += 1;
        let generation = self.next_generation;

        if let Some((_, previous)) = self.in_flight.remove(&kind) {
            previous.abort();
        }

        let tx = self.tx.clone();
        let sink = ChunkSink {
            tx: tx.clone(),
            kind,
            generation,
        };
        let handle = tokio::spawn(async move {
            let outcome = fetcher.fetch(request, sink).await;
            // The receiver may be gone during shutdown; nothing to do then.
            let _ = tx.send(LoadResult {
                done: true,
                kind,
                generation,
                outcome,
            });
        });
        self.in_flight.insert(kind, (generation, handle));
        Ok(generation)
    }

    /// Classify an incoming result. A final result of the current request
    /// also marks the request as finished.
    pub fn accept(&mut self, result: &LoadResult) -> Accept {
        match self.in_flight.get(&result.kind) {
            Some((generation, _)) if *generation == result.generation => {
                if result.done {
                    self.in_flight.remove(&result.kind);
                    Accept::Final
                } else {
                    Accept::Partial
                }
            }
            _ => Accept::Stale,
        }
    }

    /// Whether any request is still running.
    pub fn is_loading(&self) -> bool {
        !self.in_flight.is_empty()
    }

    pub fn is_loading_kind(&self, kind: LoadKind) -> bool {
        self.in_flight.contains_key(&kind)
    }
}

impl Drop for Loader {
    fn drop(&mut self) {
        for (_, (_, handle)) in self.in_flight.drain() {
            handle.abort();
        }
    }
}

/// Fetches from Azure Resource Manager.
pub struct AzureFetcher {
    client: Arc<AzureClient>,
}

impl AzureFetcher {
    pub fn new(client: Arc<AzureClient>) -> Self {
        Self { client }
    }
}

impl Fetcher for AzureFetcher {
    fn fetch(&self, request: LoadRequest, _sink: ChunkSink) -> BoxFuture<'static, FetchOutcome> {
        let client = self.client.clone();
        async move {
            match request {
                LoadRequest::Subscriptions => {
                    let _t = timing::timer("azure.list_subscriptions");
                    FetchOutcome {
                        data: client
                            .list_subscriptions()
                            .await
                            .map(|subs| {
                                LoadedData::Subscriptions(subs.iter().map(Into::into).collect())
                            })
                            .map_err(|e| e.to_string()),
                        failed_subscriptions: Vec::new(),
                        notice: None,
                    }
                }
                LoadRequest::ResourceGroups { subscription_ids } => {
                    let _t = timing::timer("azure.load_resource_groups");
                    let (items, failed) = fetch_per_subscription(subscription_ids, move |sub| {
                        let client = client.clone();
                        async move {
                            let _t = timing::timer("azure.list_resource_groups");
                            client
                                .list_resource_groups(&sub)
                                .await
                                .map(|rgs| rgs.iter().map(ResourceGroup::from).collect::<Vec<_>>())
                        }
                    })
                    .await;
                    FetchOutcome {
                        data: items.map(LoadedData::ResourceGroups),
                        failed_subscriptions: failed,
                        notice: None,
                    }
                }
                LoadRequest::Resources {
                    subscription_ids,
                    resource_group,
                } => {
                    let _t = timing::timer("azure.load_resources");
                    let (items, failed) = fetch_per_subscription(subscription_ids, move |sub| {
                        let client = client.clone();
                        let resource_group = resource_group.clone();
                        async move {
                            let fetched = match &resource_group {
                                Some(rg) => {
                                    let _t = timing::timer("azure.list_resources_in_group");
                                    client.list_resources_in_group(&sub, rg).await
                                }
                                None => {
                                    let _t = timing::timer("azure.list_resources");
                                    client.list_resources(&sub).await
                                }
                            };
                            fetched.map(|res| res.iter().map(Resource::from).collect::<Vec<_>>())
                        }
                    })
                    .await;
                    FetchOutcome {
                        data: items.map(LoadedData::Resources),
                        failed_subscriptions: failed,
                        notice: None,
                    }
                }
                LoadRequest::ScopeTree => FetchOutcome {
                    data: Err("the management group tree is not available with \
                               C9S_BACKEND=arm"
                        .into()),
                    failed_subscriptions: Vec::new(),
                    notice: None,
                },
                LoadRequest::ResourceDetail { .. } => FetchOutcome {
                    data: Err("resource details need Resource Graph \
                               (not available with C9S_BACKEND=arm)"
                        .into()),
                    failed_subscriptions: Vec::new(),
                    notice: None,
                },
                LoadRequest::Tenants => FetchOutcome {
                    data: Err("tenant names are not available with C9S_BACKEND=arm".into()),
                    failed_subscriptions: Vec::new(),
                    notice: None,
                },
                LoadRequest::CloudContexts => FetchOutcome {
                    data: cloud_contexts().await.map(LoadedData::CloudContexts),
                    failed_subscriptions: Vec::new(),
                    notice: None,
                },
                LoadRequest::RoleAssignments { .. }
                | LoadRequest::RoleDefinitions { .. }
                | LoadRequest::Principals { .. }
                | LoadRequest::PrincipalGroups { .. }
                | LoadRequest::Groups
                | LoadRequest::GroupMembers { .. }
                | LoadRequest::Applications
                | LoadRequest::AppDetail { .. } => FetchOutcome {
                    data: Err("role assignments need Resource Graph \
                                   (not available with C9S_BACKEND=arm)"
                        .into()),
                    failed_subscriptions: Vec::new(),
                    notice: None,
                },
                LoadRequest::PolicyAssignments
                | LoadRequest::PolicyExemptions
                | LoadRequest::ComplianceRollup
                | LoadRequest::PolicyCompliance { .. }
                | LoadRequest::ComplianceSummary { .. }
                | LoadRequest::PolicyStates { .. }
                | LoadRequest::ResourcePolicy { .. } => FetchOutcome {
                    data: Err("policy needs Resource Graph \
                                   (not available with C9S_BACKEND=arm)"
                        .into()),
                    failed_subscriptions: Vec::new(),
                    notice: None,
                },
                LoadRequest::Cost { .. } => FetchOutcome {
                    data: Err("cost is not available with C9S_BACKEND=arm".into()),
                    failed_subscriptions: Vec::new(),
                    notice: None,
                },
            }
        }
        .boxed()
    }
}

/// Fetches lists from Azure Resource Graph: one paged query instead of one
/// ARM call per subscription. Pages are streamed to the UI as they arrive.
///
/// Scope: a single subscription is queried directly. Several subscriptions
/// ("all") are queried through the tenant root management group (its ID is
/// the tenant ID), and the rows are limited to the requested subscriptions.
/// If the management group is not readable, the query is retried scoped to
/// the subscription list. If Resource Graph fails before any rows were
/// delivered, the request falls back to ARM and a notice says so.
pub struct GraphFetcher {
    graph: Arc<ResourceGraphClient>,
    /// Used for subscriptions, and as a fallback when Resource Graph fails.
    arm: Arc<dyn Fetcher>,
    /// Tenant of the signed-in identity, resolved once. Shared through an
    /// `Arc`: cloning a `OnceCell` copies it, so a per-request clone would
    /// never keep the value (that bug made every `:rg all` re-fetch it).
    home_tenant: Arc<tokio::sync::OnceCell<Option<String>>>,
    /// Loads the management group tree, if configured.
    management_groups: Option<Arc<ManagementGroupClient>>,
    /// Names principals (Microsoft Graph), if configured.
    directory: Option<Arc<c9s_azure::directory::DirectoryClient>>,
    /// Reads cost (Cost Management), if configured.
    cost: Option<Arc<c9s_azure::cost::CostClient>>,
}

impl GraphFetcher {
    pub fn new(graph: Arc<ResourceGraphClient>, arm: Arc<dyn Fetcher>) -> Self {
        Self {
            graph,
            arm,
            home_tenant: Arc::new(tokio::sync::OnceCell::new()),
            management_groups: None,
            directory: None,
            cost: None,
        }
    }

    /// Also serve [`LoadRequest::Principals`] through this client.
    pub fn with_directory(mut self, client: Arc<c9s_azure::directory::DirectoryClient>) -> Self {
        self.directory = Some(client);
        self
    }

    /// Also serve [`LoadRequest::Cost`] through this client.
    pub fn with_cost(mut self, client: Arc<c9s_azure::cost::CostClient>) -> Self {
        self.cost = Some(client);
        self
    }

    /// Also serve [`LoadRequest::ScopeTree`] through this client.
    pub fn with_management_groups(mut self, client: Arc<ManagementGroupClient>) -> Self {
        self.management_groups = Some(client);
        self
    }

    /// The tree below the tenant root group (whose name is the tenant ID).
    async fn scope_tree(
        graph: Arc<ResourceGraphClient>,
        cell: Arc<tokio::sync::OnceCell<Option<String>>>,
        client: Option<Arc<ManagementGroupClient>>,
    ) -> Result<ScopeTree, String> {
        let client = client.ok_or("management groups are not configured")?;
        let tenant = Self::resolve_tenant(&graph, &cell)
            .await
            .ok_or("could not determine the tenant from the Azure token")?;
        let (nodes, name) =
            futures::join!(client.descendants(&tenant), client.display_name(&tenant));
        let nodes = nodes.map_err(|e| e.to_string())?;
        // The descendants are what matters; a missing name is cosmetic.
        let name = name.unwrap_or_else(|_| "Tenant Root Group".to_string());
        Ok(ScopeTree::from_nodes(&tenant, &name, &nodes))
    }
}

impl GraphFetcher {
    async fn resolve_tenant(
        graph: &ResourceGraphClient,
        cell: &tokio::sync::OnceCell<Option<String>>,
    ) -> Option<String> {
        cell.get_or_init(|| async { graph.home_tenant_id().await.ok() })
            .await
            .clone()
    }
}

impl Fetcher for GraphFetcher {
    fn warm_up(&self) {
        // Getting the first token through the Azure CLI takes ~0.8 s. Do it
        // now, while the user looks at the subscription list.
        let graph = self.graph.clone();
        let cell = self.home_tenant.clone();
        tokio::spawn(async move {
            let _t = timing::timer("azure.warm_up");
            let _ = Self::resolve_tenant(&graph, &cell).await;
        });
    }

    fn fetch(&self, request: LoadRequest, sink: ChunkSink) -> BoxFuture<'static, FetchOutcome> {
        // Subscriptions come from the Azure CLI (with tenant information).
        if request == LoadRequest::Subscriptions {
            return self.arm.fetch(request, sink);
        }
        if let LoadRequest::Cost { query } = &request {
            let client = self.cost.clone();
            let query = query.clone();
            return async move {
                let data = match client {
                    None => Err("cost is not configured".to_string()),
                    Some(client) => match client.query(&query.api()).await {
                        Ok(table) => crate::cost::CostReport::from_table(query, &table)
                            .map(|r| LoadedData::Cost(Box::new(r))),
                        Err(e) => Err(cost_error(e)),
                    },
                };
                FetchOutcome {
                    data,
                    failed_subscriptions: vec![],
                    notice: None,
                }
            }
            .boxed();
        }
        if let LoadRequest::ResourceDetail {
            resource_id,
            subscription_id,
        } = &request
        {
            let graph = self.graph.clone();
            let cell = self.home_tenant.clone();
            let query = resource_graph::resource_detail_query(resource_id);
            // Policy objects can sit on management groups (no subscription):
            // look for them across the tenant.
            let policy = resource_graph::is_policy_id(resource_id) || subscription_id.is_empty();
            let subscription_id = subscription_id.clone();
            let arm = self.management_groups.clone();
            let id = resource_id.clone();
            return async move {
                let _t = timing::timer("azure.resource_detail");
                let scope = if policy {
                    match Self::resolve_tenant(&graph, &cell).await {
                        Some(tenant) => GraphScope::ManagementGroup(tenant),
                        None => GraphScope::Subscriptions(vec![subscription_id]),
                    }
                } else {
                    GraphScope::Subscriptions(vec![subscription_id])
                };
                let mut first = None;
                let result = graph
                    .query_pages(&query, &scope, |rows| {
                        if first.is_none() {
                            first = rows.into_iter().next();
                        }
                    })
                    .await;
                let data = match (result, first) {
                    (Ok(_), Some(doc)) => Ok(LoadedData::ResourceDetail(doc)),
                    (Ok(_), None) => Err("not found in Resource Graph (it can lag \
                                          behind ARM for a few minutes)"
                        .to_string()),
                    (Err(e), _) => Err(e.to_string()),
                };
                // Built-in policy definitions may be missing from Resource
                // Graph; ARM serves every definition by ID.
                let data = match (data, arm) {
                    (Err(e), Some(arm)) if is_definition_id(&id) => arm
                        .policy_object(&id)
                        .await
                        .map(LoadedData::ResourceDetail)
                        .map_err(|arm_error| format!("{e}; ARM: {arm_error}")),
                    // Role definitions likewise (built-in ones, or an ID
                    // written at another scope than Resource Graph's).
                    (Err(e), Some(arm)) if is_role_definition_id(&id) => arm
                        .role_definition(&id)
                        .await
                        .map(LoadedData::ResourceDetail)
                        .map_err(|arm_error| format!("{e}; ARM: {arm_error}")),
                    (data, _) => data,
                };
                FetchOutcome {
                    data,
                    failed_subscriptions: vec![],
                    notice: None,
                }
            }
            .boxed();
        }
        match &request {
            LoadRequest::Applications | LoadRequest::AppDetail { .. } => {
                let directory = self.directory.clone();
                let request = request.clone();
                return async move {
                    let data = match (directory, request) {
                        (None, _) => Err("Microsoft Graph is not configured".to_string()),
                        (Some(d), LoadRequest::Applications) => d
                            .applications()
                            .await
                            .map(|(apps, sps)| LoadedData::Applications(apps, sps))
                            .map_err(|e| e.to_string()),
                        (
                            Some(d),
                            LoadRequest::AppDetail {
                                app_id,
                                application_id,
                                service_principal_id,
                            },
                        ) => d
                            .app_detail(application_id.as_deref(), service_principal_id.as_deref())
                            .await
                            .map(|detail| LoadedData::AppDetail(app_id, Box::new(detail)))
                            .map_err(|e| e.to_string()),
                        (Some(_), _) => unreachable!("matched above"),
                    };
                    FetchOutcome {
                        data,
                        failed_subscriptions: vec![],
                        notice: None,
                    }
                }
                .boxed();
            }
            LoadRequest::Groups | LoadRequest::GroupMembers { .. } => {
                let directory = self.directory.clone();
                let request = request.clone();
                return async move {
                    let data = match (directory, request) {
                        (None, _) => Err("Microsoft Graph is not configured".to_string()),
                        (Some(d), LoadRequest::Groups) => d
                            .groups()
                            .await
                            .map(LoadedData::Groups)
                            .map_err(|e| e.to_string()),
                        (
                            Some(d),
                            LoadRequest::GroupMembers {
                                group_id,
                                transitive,
                            },
                        ) => d
                            .members(&group_id, transitive)
                            .await
                            .map(|m| {
                                LoadedData::GroupMembers(
                                    group_id.to_ascii_lowercase(),
                                    transitive,
                                    m,
                                )
                            })
                            .map_err(|e| e.to_string()),
                        (Some(_), _) => unreachable!("matched above"),
                    };
                    FetchOutcome {
                        data,
                        failed_subscriptions: vec![],
                        notice: None,
                    }
                }
                .boxed();
            }
            LoadRequest::PrincipalGroups { principal_id } => {
                let directory = self.directory.clone();
                let id = principal_id.to_ascii_lowercase();
                return async move {
                    let data = match directory {
                        Some(directory) => directory
                            .groups_of(&id)
                            .await
                            .map(|groups| LoadedData::PrincipalGroups(id, groups))
                            .map_err(|e| e.to_string()),
                        None => Ok(LoadedData::PrincipalGroups(id, vec![])),
                    };
                    FetchOutcome {
                        data,
                        failed_subscriptions: vec![],
                        notice: None,
                    }
                }
                .boxed();
            }
            LoadRequest::Principals { ids } => {
                let directory = self.directory.clone();
                let ids = ids.clone();
                return async move {
                    let data = match directory {
                        Some(directory) => directory
                            .principals(&ids)
                            .await
                            .map(LoadedData::Principals)
                            .map_err(|e| e.to_string()),
                        // No directory client (tests, offline): no names.
                        None => Ok(LoadedData::Principals(vec![])),
                    };
                    FetchOutcome {
                        data,
                        failed_subscriptions: vec![],
                        notice: None,
                    }
                }
                .boxed();
            }
            LoadRequest::RoleAssignments { subscription_ids }
            | LoadRequest::RoleDefinitions { subscription_ids } => {
                let graph = self.graph.clone();
                let cell = self.home_tenant.clone();
                let arm = self.management_groups.clone();
                let ids = subscription_ids.clone();
                let assignments = matches!(request, LoadRequest::RoleAssignments { .. });
                let pim = self.management_groups.clone();
                let pim_subscriptions = subscription_ids.clone();
                return async move {
                    let _t = timing::timer(if assignments {
                        "azure.role_assignments"
                    } else {
                        "azure.role_definitions"
                    });
                    let query = if assignments {
                        resource_graph::role_assignments_query()
                    } else {
                        resource_graph::role_definitions_query()
                    };
                    let tenant = Self::resolve_tenant(&graph, &cell).await;
                    let first_subscription = ids.first().cloned();
                    let (rows, notice) =
                        tenant_or_subscriptions(&graph, &query, tenant.clone(), ids).await;
                    // Resource Graph may not list the built-in roles: then
                    // ask ARM for every role usable at the tenant root (or
                    // a subscription), so assignments show role names.
                    let rows = match (assignments, arm) {
                        (false, Some(arm)) if !has_built_in_roles(&rows) => {
                            let scope = match (&tenant, &first_subscription) {
                                (Some(t), _) => {
                                    format!("/providers/Microsoft.Management/managementGroups/{t}")
                                }
                                (None, Some(s)) => format!("/subscriptions/{s}"),
                                (None, None) => String::new(),
                            };
                            let mut from_arm = arm.role_definitions(&scope).await;
                            if from_arm.is_err() {
                                if let (Some(_), Some(s)) = (&tenant, &first_subscription) {
                                    // No access at the root: a subscription
                                    // lists the same built-in roles.
                                    from_arm =
                                        arm.role_definitions(&format!("/subscriptions/{s}")).await;
                                }
                            }
                            merge_role_rows(rows, from_arm)
                        }
                        _ => rows,
                    };
                    // PIM: eligible assignments beside the active ones.
                    let mut notice = notice;
                    let mut eligible = Vec::new();
                    if assignments && rows.is_ok() {
                        if let Some(arm) = &pim {
                            let (found, problem) =
                                role_eligibilities(arm, tenant.clone(), pim_subscriptions).await;
                            eligible = found;
                            if let Some(problem) = problem {
                                notice = Some(match notice {
                                    Some(n) => format!("{n}; {problem}"),
                                    None => problem,
                                });
                            }
                        }
                    }
                    let data = rows.map(|rows| {
                        if assignments {
                            LoadedData::RoleAssignments(
                                rows.iter()
                                    .filter_map(crate::rbac::RoleAssignment::from_row)
                                    .chain(eligible)
                                    .collect(),
                            )
                        } else {
                            LoadedData::RoleDefinitions(
                                rows.iter()
                                    .filter_map(crate::rbac::RoleDefinition::from_row)
                                    .collect(),
                            )
                        }
                    });
                    FetchOutcome {
                        data,
                        failed_subscriptions: vec![],
                        notice,
                    }
                }
                .boxed();
            }
            LoadRequest::PolicyExemptions => {
                let graph = self.graph.clone();
                let cell = self.home_tenant.clone();
                return async move {
                    let _t = timing::timer("azure.policy_exemptions");
                    let data = async {
                        let tenant = Self::resolve_tenant(&graph, &cell)
                            .await
                            .ok_or("could not determine the tenant from the Azure token")?;
                        let rows = query_all(
                            &graph,
                            &resource_graph::policy_exemptions_query(),
                            &GraphScope::ManagementGroup(tenant),
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                        Ok::<_, String>(LoadedData::PolicyExemptions(
                            rows.iter().filter_map(PolicyExemption::from_row).collect(),
                        ))
                    }
                    .await;
                    FetchOutcome {
                        data,
                        failed_subscriptions: vec![],
                        notice: None,
                    }
                }
                .boxed();
            }
            LoadRequest::ComplianceRollup => {
                let graph = self.graph.clone();
                let cell = self.home_tenant.clone();
                return async move {
                    let _t = timing::timer("azure.compliance_rollup");
                    let data = async {
                        let tenant = Self::resolve_tenant(&graph, &cell)
                            .await
                            .ok_or("could not determine the tenant from the Azure token")?;
                        let rows = query_all(
                            &graph,
                            &resource_graph::compliance_summary_query(),
                            &GraphScope::ManagementGroup(tenant),
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                        Ok::<_, String>(LoadedData::ComplianceRollup(
                            subscription_compliance_from_rows(&rows),
                        ))
                    }
                    .await;
                    FetchOutcome {
                        data,
                        failed_subscriptions: vec![],
                        notice: None,
                    }
                }
                .boxed();
            }
            LoadRequest::PolicyAssignments => {
                let graph = self.graph.clone();
                let cell = self.home_tenant.clone();
                return async move {
                    let _t = timing::timer("azure.policy_assignments");
                    let data = async {
                        let tenant = Self::resolve_tenant(&graph, &cell)
                            .await
                            .ok_or("could not determine the tenant from the Azure token")?;
                        let rows = query_all(
                            &graph,
                            &resource_graph::policy_assignments_query(),
                            &GraphScope::ManagementGroup(tenant),
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                        Ok::<_, String>(LoadedData::PolicyAssignments(
                            rows.iter().filter_map(PolicyAssignment::from_row).collect(),
                        ))
                    }
                    .await;
                    FetchOutcome {
                        data,
                        failed_subscriptions: vec![],
                        notice: None,
                    }
                }
                .boxed();
            }
            LoadRequest::PolicyCompliance { scope } | LoadRequest::ComplianceSummary { scope } => {
                let graph = self.graph.clone();
                let scope = scope.clone();
                let summary = matches!(request, LoadRequest::ComplianceSummary { .. });
                return async move {
                    let _t = timing::timer(if summary {
                        "azure.compliance_summary"
                    } else {
                        "azure.policy_compliance"
                    });
                    let graph_scope = match &scope {
                        PolicyScope::Group(g) => GraphScope::ManagementGroup(g.clone()),
                        PolicyScope::Subscription(s) => GraphScope::Subscriptions(vec![s.clone()]),
                    };
                    let query = if summary {
                        resource_graph::compliance_summary_query()
                    } else {
                        resource_graph::policy_compliance_query()
                    };
                    let data = query_all(&graph, &query, &graph_scope)
                        .await
                        .map(|rows| {
                            if summary {
                                LoadedData::ComplianceSummary(
                                    scope,
                                    subscription_compliance_from_rows(&rows),
                                )
                            } else {
                                LoadedData::PolicyCompliance(scope, compliance_from_rows(&rows))
                            }
                        })
                        .map_err(|e| e.to_string());
                    FetchOutcome {
                        data,
                        failed_subscriptions: vec![],
                        notice: None,
                    }
                }
                .boxed();
            }
            LoadRequest::PolicyStates {
                scope,
                assignment_id,
            } => {
                let graph = self.graph.clone();
                let graph_scope = match scope {
                    PolicyScope::Group(g) => GraphScope::ManagementGroup(g.clone()),
                    PolicyScope::Subscription(s) => GraphScope::Subscriptions(vec![s.clone()]),
                };
                let query = resource_graph::non_compliant_states_query(assignment_id);
                return async move {
                    let _t = timing::timer("azure.policy_states");
                    let data = query_all(&graph, &query, &graph_scope)
                        .await
                        .map(|rows| {
                            LoadedData::PolicyStates(
                                rows.iter().filter_map(PolicyState::from_row).collect(),
                            )
                        })
                        .map_err(|e| e.to_string());
                    FetchOutcome {
                        data,
                        failed_subscriptions: vec![],
                        notice: None,
                    }
                }
                .boxed();
            }
            LoadRequest::ResourcePolicy {
                resource_id,
                subscription_id,
            } => {
                let graph = self.graph.clone();
                let cell = self.home_tenant.clone();
                let arm = self.management_groups.clone();
                let query = resource_graph::resource_policy_states_query(resource_id);
                let resource_id = resource_id.clone();
                let subscription_id = subscription_id.clone();
                return async move {
                    let _t = timing::timer("azure.resource_policy");
                    let data = async {
                        let scope = if subscription_id.is_empty() {
                            let tenant = Self::resolve_tenant(&graph, &cell)
                                .await
                                .ok_or("could not determine the tenant from the Azure token")?;
                            GraphScope::ManagementGroup(tenant)
                        } else {
                            GraphScope::Subscriptions(vec![subscription_id])
                        };
                        let rows = query_all(&graph, &query, &scope)
                            .await
                            .map_err(|e| e.to_string())?;
                        let mut states: Vec<PolicyState> =
                            rows.iter().filter_map(PolicyState::from_row).collect();
                        // Names are a nicety: the states show without them.
                        let tenant = Self::resolve_tenant(&graph, &cell).await;
                        name_definitions(&graph, tenant, arm, &mut states).await;
                        Ok::<_, String>(LoadedData::ResourcePolicy(resource_id, states))
                    }
                    .await;
                    FetchOutcome {
                        data,
                        failed_subscriptions: vec![],
                        notice: None,
                    }
                }
                .boxed();
            }
            _ => {}
        }
        if request == LoadRequest::CloudContexts {
            return async move {
                FetchOutcome {
                    data: cloud_contexts().await.map(LoadedData::CloudContexts),
                    failed_subscriptions: vec![],
                    notice: None,
                }
            }
            .boxed();
        }
        if request == LoadRequest::Tenants {
            let client = self.management_groups.clone();
            return async move {
                let data = match client {
                    Some(client) => client
                        .tenants()
                        .await
                        .map(|tenants| {
                            LoadedData::Tenants(
                                tenants
                                    .into_iter()
                                    .map(|t| crate::models::Tenant {
                                        id: t.id,
                                        name: t.display_name,
                                        domain: t.default_domain,
                                    })
                                    .collect(),
                            )
                        })
                        .map_err(|e| e.to_string()),
                    None => Err("management groups are not configured".into()),
                };
                FetchOutcome {
                    data,
                    failed_subscriptions: vec![],
                    notice: None,
                }
            }
            .boxed();
        }
        if request == LoadRequest::ScopeTree {
            let tree = Self::scope_tree(
                self.graph.clone(),
                self.home_tenant.clone(),
                self.management_groups.clone(),
            );
            return async move {
                FetchOutcome {
                    data: tree.await.map(LoadedData::ScopeTree),
                    failed_subscriptions: vec![],
                    notice: None,
                }
            }
            .boxed();
        }
        let graph = self.graph.clone();
        let arm = self.arm.clone();
        let home_tenant = self.home_tenant.clone();
        async move {
            let (query, subscription_ids, kind) = match &request {
                LoadRequest::ResourceGroups { subscription_ids } => (
                    resource_graph::resource_groups_query(),
                    subscription_ids.clone(),
                    LoadKind::ResourceGroups,
                ),
                LoadRequest::Resources {
                    subscription_ids,
                    resource_group,
                } => (
                    resource_graph::resources_query(resource_group.as_deref()),
                    subscription_ids.clone(),
                    LoadKind::Resources,
                ),
                LoadRequest::Subscriptions
                | LoadRequest::ScopeTree
                | LoadRequest::Tenants
                | LoadRequest::CloudContexts
                | LoadRequest::PolicyAssignments
                | LoadRequest::PolicyExemptions
                | LoadRequest::RoleAssignments { .. }
                | LoadRequest::RoleDefinitions { .. }
                | LoadRequest::Principals { .. }
                | LoadRequest::PrincipalGroups { .. }
                | LoadRequest::Groups
                | LoadRequest::GroupMembers { .. }
                | LoadRequest::Applications
                | LoadRequest::AppDetail { .. }
                | LoadRequest::ComplianceRollup
                | LoadRequest::PolicyCompliance { .. }
                | LoadRequest::ComplianceSummary { .. }
                | LoadRequest::PolicyStates { .. }
                | LoadRequest::ResourcePolicy { .. }
                | LoadRequest::Cost { .. }
                | LoadRequest::ResourceDetail { .. } => {
                    unreachable!("handled above")
                }
            };
            if subscription_ids.is_empty() {
                return FetchOutcome {
                    data: Ok(empty(kind)),
                    failed_subscriptions: vec![],
                    notice: None,
                };
            }

            let tenant = if subscription_ids.len() > 1 {
                Self::resolve_tenant(&graph, &home_tenant).await
            } else {
                None
            };

            let _t = timing::timer("azure.graph_query");
            let mut delivered = 0usize;
            let mut result = Err(AzureError::InvalidConfig("no scope".into()));
            if let Some(tenant) = tenant {
                result = query_into_sink(
                    &graph,
                    &query,
                    &GraphScope::ManagementGroup(tenant),
                    &subscription_ids,
                    kind,
                    &sink,
                    &mut delivered,
                )
                .await;
                if let Err(e) = &result {
                    warn!("Resource Graph at tenant root failed: {}", e);
                }
            }
            if result.is_err() && delivered == 0 {
                result = query_into_sink(
                    &graph,
                    &query,
                    &GraphScope::Subscriptions(subscription_ids.clone()),
                    &subscription_ids,
                    kind,
                    &sink,
                    &mut delivered,
                )
                .await;
            }

            match result {
                Ok(_) => FetchOutcome {
                    // Everything was streamed through the sink already.
                    data: Ok(empty(kind)),
                    failed_subscriptions: vec![],
                    notice: None,
                },
                Err(e) if delivered == 0 => {
                    warn!("Resource Graph failed, falling back to ARM: {}", e);
                    let mut outcome = arm.fetch(request, sink).await;
                    outcome.notice = Some(format!("Resource Graph unavailable ({e}); used ARM"));
                    outcome
                }
                Err(e) => FetchOutcome {
                    data: Err(format!("Resource Graph failed after {delivered} rows: {e}")),
                    failed_subscriptions: vec![],
                    notice: None,
                },
            }
        }
        .boxed()
    }
}

fn empty(kind: LoadKind) -> LoadedData {
    match kind {
        LoadKind::Subscriptions => LoadedData::Subscriptions(vec![]),
        LoadKind::ResourceGroups => LoadedData::ResourceGroups(vec![]),
        LoadKind::Resources => LoadedData::Resources(vec![]),
        LoadKind::ScopeTree => LoadedData::ScopeTree(ScopeTree::default()),
        LoadKind::ResourceDetail => LoadedData::ResourceDetail(serde_json::Value::Null),
        LoadKind::Tenants => LoadedData::Tenants(vec![]),
        LoadKind::CloudContexts => LoadedData::CloudContexts(CloudContexts::default()),
        LoadKind::PolicyAssignments => LoadedData::PolicyAssignments(vec![]),
        LoadKind::PolicyExemptions => LoadedData::PolicyExemptions(vec![]),
        LoadKind::RoleAssignments => LoadedData::RoleAssignments(vec![]),
        LoadKind::RoleDefinitions => LoadedData::RoleDefinitions(vec![]),
        LoadKind::Principals => LoadedData::Principals(vec![]),
        LoadKind::PrincipalGroups => LoadedData::PrincipalGroups(String::new(), vec![]),
        LoadKind::Groups => LoadedData::Groups(vec![]),
        LoadKind::Applications => LoadedData::Applications(vec![], vec![]),
        LoadKind::AppDetail => LoadedData::AppDetail(String::new(), Box::default()),
        LoadKind::GroupMembers => LoadedData::GroupMembers(String::new(), false, vec![]),
        LoadKind::ComplianceRollup => LoadedData::ComplianceRollup(HashMap::new()),
        LoadKind::PolicyCompliance => {
            LoadedData::PolicyCompliance(PolicyScope::Group(String::new()), HashMap::new())
        }
        LoadKind::ComplianceSummary => {
            LoadedData::ComplianceSummary(PolicyScope::Group(String::new()), HashMap::new())
        }
        LoadKind::PolicyStates => LoadedData::PolicyStates(vec![]),
        LoadKind::ResourcePolicy => LoadedData::ResourcePolicy(String::new(), vec![]),
        LoadKind::Cost => unreachable!("cost is loaded by its own fetch"),
    }
}

fn is_role_definition_id(id: &str) -> bool {
    id.to_ascii_lowercase()
        .contains("/providers/microsoft.authorization/roledefinitions/")
}

/// Whether Resource Graph's role definitions include the built-in roles.
fn has_built_in_roles(rows: &Result<Vec<serde_json::Value>, String>) -> bool {
    rows.as_ref().is_ok_and(|rows| {
        rows.iter().any(|r| {
            r.get("roleType")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|t| t.eq_ignore_ascii_case("BuiltInRole"))
        })
    })
}

/// Resource Graph's role definitions plus ARM's, each role (by GUID) once.
/// Either source alone is enough; both failing reports the first error.
/// Cost Management's error, with what to do about the usual ones.
fn cost_error(e: AzureError) -> String {
    match &e {
        AzureError::Http {
            status: 401 | 403, ..
        } => format!("{e}\n\nReading cost needs Cost Management Reader (or Reader) on the scope."),
        AzureError::Http { status: 429, .. } => format!(
            "{e}\n\nCost Management limits how often cost can be read; try again in a minute."
        ),
        _ => e.to_string(),
    }
}

/// PIM eligible role assignments: from the tenant root group (at and below
/// it), else from each subscription. A note when some or all of them
/// couldn't be read; the active assignments show either way.
async fn role_eligibilities(
    arm: &ManagementGroupClient,
    tenant: Option<String>,
    subscriptions: Vec<String>,
) -> (Vec<crate::rbac::RoleAssignment>, Option<String>) {
    let now = chrono::Utc::now().timestamp();
    let parse = |items: &[serde_json::Value]| -> Vec<crate::rbac::RoleAssignment> {
        items
            .iter()
            .filter_map(|i| crate::rbac::RoleAssignment::from_eligibility(i, now))
            .collect()
    };
    if let Some(tenant) = &tenant {
        let root = format!("/providers/Microsoft.Management/managementGroups/{tenant}");
        match arm.role_eligibilities(&root).await {
            Ok(items) => return (parse(&items), None),
            Err(e) => warn!("PIM eligibility at the tenant root: {e}; trying subscriptions"),
        }
    }
    if subscriptions.is_empty() {
        return (
            vec![],
            Some("PIM eligible assignments unavailable (no access at the tenant root)".into()),
        );
    }
    let total = subscriptions.len();
    let results: Vec<Result<Vec<serde_json::Value>, AzureError>> = stream::iter(subscriptions)
        .map(|s| async move { arm.role_eligibilities(&format!("/subscriptions/{s}")).await })
        .buffer_unordered(8)
        .collect()
        .await;
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    let mut failed = 0;
    let mut first_error = None;
    for result in results {
        match result {
            Ok(items) => {
                for a in parse(&items) {
                    if seen.insert(a.id.to_ascii_lowercase()) {
                        found.push(a);
                    }
                }
            }
            Err(e) => {
                failed += 1;
                first_error.get_or_insert(e.to_string());
            }
        }
    }
    let note = first_error.map(|e| {
        if failed == total {
            format!("PIM eligible assignments unavailable: {e}")
        } else {
            format!("PIM eligible assignments missing for {failed} of {total} subscriptions: {e}")
        }
    });
    (found, note)
}

fn merge_role_rows(
    graph: Result<Vec<serde_json::Value>, String>,
    arm: c9s_azure::Result<Vec<serde_json::Value>>,
) -> Result<Vec<serde_json::Value>, String> {
    let guid = |r: &serde_json::Value| {
        r.get("id")
            .and_then(serde_json::Value::as_str)
            .map(crate::rbac::role_guid)
    };
    match (graph, arm) {
        (Ok(mut rows), Ok(more)) => {
            let known: HashSet<String> = rows.iter().filter_map(guid).collect();
            rows.extend(
                more.into_iter()
                    .filter(|r| guid(r).is_some_and(|g| !known.contains(&g))),
            );
            Ok(rows)
        }
        (Ok(rows), Err(e)) => {
            warn!("Built-in role names from ARM unavailable: {}", e);
            Ok(rows)
        }
        (Err(_), Ok(rows)) => Ok(rows),
        (Err(e), Err(_)) => Err(e),
    }
}

/// Run a query across the tenant (its root management group), or, when that
/// isn't readable (no management group access), across `subscription_ids`.
/// The notice says when the narrower scope was used.
async fn tenant_or_subscriptions(
    graph: &ResourceGraphClient,
    query: &str,
    tenant: Option<String>,
    subscription_ids: Vec<String>,
) -> (Result<Vec<serde_json::Value>, String>, Option<String>) {
    let mut tenant_error = None;
    if let Some(tenant) = tenant {
        match query_all(graph, query, &GraphScope::ManagementGroup(tenant)).await {
            Ok(rows) => return (Ok(rows), None),
            Err(e) => {
                warn!("Resource Graph at the tenant root failed: {}", e);
                tenant_error = Some(e.to_string());
            }
        }
    }
    if subscription_ids.is_empty() {
        let e = tenant_error.unwrap_or_else(|| "could not determine the tenant".into());
        return (Err(e), None);
    }
    let rows = query_all(graph, query, &GraphScope::Subscriptions(subscription_ids))
        .await
        .map_err(|e| e.to_string());
    let notice = (rows.is_ok() && tenant_error.is_some()).then(|| {
        "No access at the tenant root: showing what your subscriptions grant \
         (management group assignments may be missing)"
            .to_string()
    });
    (rows, notice)
}

/// Most definitions whose names are fetched one by one from ARM when
/// Resource Graph doesn't know them.
const MAX_ARM_NAME_LOOKUPS: usize = 60;

fn is_definition_id(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    id.contains("/providers/microsoft.authorization/policydefinitions/")
        || id.contains("/providers/microsoft.authorization/policysetdefinitions/")
}

/// Fill in the display names of the states' policy definitions and
/// initiatives: from Resource Graph across the tenant, then from ARM for
/// those it doesn't list (built-ins, possibly). Failures leave names empty.
async fn name_definitions(
    graph: &ResourceGraphClient,
    tenant: Option<String>,
    arm: Option<Arc<ManagementGroupClient>>,
    states: &mut [PolicyState],
) {
    // Lowercase ID → the ID as Azure reported it.
    let originals: HashMap<String, String> = states
        .iter()
        .flat_map(|s| [&s.definition_id, &s.set_definition_id])
        .filter(|id| !id.is_empty())
        .map(|id| (id.to_ascii_lowercase(), id.clone()))
        .collect();
    let mut ids: Vec<String> = originals.keys().cloned().collect();
    ids.sort();
    if ids.is_empty() {
        return;
    }
    let mut names: HashMap<String, String> = HashMap::new();
    if let Some(tenant) = tenant {
        let query = resource_graph::policy_definition_names_query(&ids);
        match query_all(graph, &query, &GraphScope::ManagementGroup(tenant)).await {
            Ok(rows) => names.extend(rows.iter().filter_map(|row| {
                let id = row.get("id")?.as_str()?.to_ascii_lowercase();
                let name = row.get("displayName")?.as_str()?.to_string();
                (!name.is_empty()).then_some((id, name))
            })),
            Err(e) => warn!("Looking up policy definition names failed: {}", e),
        }
    }
    if let Some(arm) = arm {
        let missing: Vec<String> = ids
            .iter()
            .filter(|id| !names.contains_key(*id))
            .take(MAX_ARM_NAME_LOOKUPS)
            .cloned()
            .collect();
        let originals = &originals;
        let found: Vec<(String, String)> = stream::iter(missing)
            .map(|id| {
                let arm = arm.clone();
                async move {
                    let doc = arm.policy_object(&originals[&id]).await.ok()?;
                    let name = doc
                        .pointer("/properties/displayName")?
                        .as_str()?
                        .to_string();
                    Some((id, name))
                }
            })
            .buffer_unordered(8)
            .filter_map(|found| async move { found })
            .collect()
            .await;
        names.extend(found);
    }
    for s in states {
        let name = |id: &str| {
            names
                .get(&id.to_ascii_lowercase())
                .cloned()
                .unwrap_or_default()
        };
        s.definition_name = name(&s.definition_id);
        s.initiative_name = name(&s.set_definition_id);
    }
}

/// Run a Resource Graph query and collect every row (for small results,
/// such as policy assignments or per-assignment counts).
async fn query_all(
    graph: &ResourceGraphClient,
    query: &str,
    scope: &GraphScope,
) -> c9s_azure::Result<Vec<serde_json::Value>> {
    let mut all = Vec::new();
    graph
        .query_pages(query, scope, |rows| all.extend(rows))
        .await?;
    Ok(all)
}

/// Run a Resource Graph query and stream each page into `sink`, keeping only
/// rows from the requested subscriptions. `delivered` counts rows sent.
async fn query_into_sink(
    graph: &ResourceGraphClient,
    query: &str,
    scope: &GraphScope,
    subscription_ids: &[String],
    kind: LoadKind,
    sink: &ChunkSink,
    delivered: &mut usize,
) -> c9s_azure::Result<usize> {
    let wanted: HashSet<String> = subscription_ids
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();
    let wanted_sub = |sub: &str| wanted.contains(&sub.to_ascii_lowercase());
    let started = std::time::Instant::now();
    let mut first_page = true;

    graph
        .query_pages(query, scope, |rows| {
            if first_page {
                first_page = false;
                timing::record("azure.graph_first_page", started.elapsed());
            }
            let chunk = match kind {
                LoadKind::ResourceGroups => {
                    let items: Vec<ResourceGroup> = rows
                        .iter()
                        .filter_map(resource_graph::resource_group_from_row)
                        .filter(|rg| wanted_sub(&rg.subscription_id))
                        .map(|rg| ResourceGroup::from(&rg))
                        .collect();
                    *delivered += items.len();
                    LoadedData::ResourceGroups(items)
                }
                _ => {
                    let items: Vec<Resource> = rows
                        .iter()
                        .filter_map(resource_graph::resource_from_row)
                        .filter(|r| wanted_sub(&r.subscription_id))
                        .map(|r| Resource::from(&r))
                        .collect();
                    *delivered += items.len();
                    LoadedData::Resources(items)
                }
            };
            sink.send(chunk);
        })
        .await
}

/// Run `fetch_one` for every subscription, up to [`MAX_CONCURRENT_REQUESTS`]
/// at a time, and concatenate the results.
///
/// Returns an error only if every subscription failed; otherwise returns the
/// combined items plus the IDs of the subscriptions that failed.
pub async fn fetch_per_subscription<T, F, Fut, E>(
    subscription_ids: Vec<String>,
    fetch_one: F,
) -> (Result<Vec<T>, String>, Vec<String>)
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<T>, E>>,
    E: std::fmt::Display,
{
    let total = subscription_ids.len();
    let results: Vec<(String, Result<Vec<T>, E>)> = stream::iter(subscription_ids)
        .map(|sub| {
            let fut = fetch_one(sub.clone());
            async move { (sub, fut.await) }
        })
        .buffer_unordered(MAX_CONCURRENT_REQUESTS)
        .collect()
        .await;

    let mut items = Vec::new();
    let mut failed = Vec::new();
    let mut first_error = None;
    for (sub, result) in results {
        match result {
            Ok(mut chunk) => items.append(&mut chunk),
            Err(e) => {
                warn!("Loading subscription {} failed: {}", sub, e);
                first_error.get_or_insert_with(|| e.to_string());
                failed.push(sub);
            }
        }
    }

    if total > 0 && failed.len() == total {
        (
            Err(first_error.unwrap_or_else(|| "unknown error".into())),
            failed,
        )
    } else {
        (Ok(items), failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_errors_say_what_to_do() {
        let denied = cost_error(AzureError::Http {
            status: 403,
            message: "does not have authorization".into(),
        });
        assert!(denied.contains("Cost Management Reader"), "{denied}");
        let throttled = cost_error(AzureError::Http {
            status: 429,
            message: "Too many requests".into(),
        });
        assert!(throttled.contains("try again in a minute"), "{throttled}");
    }

    #[tokio::test]
    async fn pim_eligibility_falls_back_to_subscriptions() {
        use c9s_azure::testing::{error, ok, serve_paths, StaticToken};
        use serde_json::json;
        let item = |id: &str, scope: &str| {
            json!({"id": id, "properties": {"scope": scope, "principalId": "p1",
                "principalType": "User", "memberType": "Direct",
                "roleDefinitionId": "/providers/Microsoft.Authorization/roleDefinitions/r1"}})
        };
        let (endpoint, _) = serve_paths(move |path, _| {
            if path.starts_with("/providers/Microsoft.Management") {
                return error(403, "no access at the root");
            }
            if path.starts_with("/subscriptions/s1/") {
                // The MG-level one comes back from both subscriptions.
                return ok(json!({"value": [item("e-mg", "/providers/Microsoft.Management/managementGroups/m"), item("e-s1", "/subscriptions/s1")]}));
            }
            if path.starts_with("/subscriptions/s2/") {
                return ok(json!({"value": [item("e-mg", "/providers/Microsoft.Management/managementGroups/m")]}));
            }
            error(403, "denied")
        })
        .await;
        let arm = ManagementGroupClient::with_endpoint(endpoint, Arc::new(StaticToken("t".into())));
        let (found, note) = role_eligibilities(
            &arm,
            Some("t".into()),
            vec!["s1".into(), "s2".into(), "s3".into()],
        )
        .await;
        let mut ids: Vec<&str> = found.iter().map(|a| a.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, ["e-mg", "e-s1"], "each once");
        assert!(found.iter().all(|a| a.eligible));
        let note = note.unwrap();
        assert!(note.contains("missing for 1 of 3 subscriptions"), "{note}");
    }
    use std::time::Duration;

    /// Returns `n` resource groups after `delay`, where `n` is the number of
    /// subscriptions requested (so tests can tell requests apart).
    struct FakeFetcher {
        delay: Duration,
    }

    impl Fetcher for FakeFetcher {
        fn fetch(
            &self,
            request: LoadRequest,
            _sink: ChunkSink,
        ) -> BoxFuture<'static, FetchOutcome> {
            let delay = self.delay;
            async move {
                tokio::time::sleep(delay).await;
                let n = match &request {
                    LoadRequest::ResourceGroups { subscription_ids } => subscription_ids.len(),
                    _ => 0,
                };
                FetchOutcome {
                    data: Ok(LoadedData::ResourceGroups(
                        (0..n)
                            .map(|i| ResourceGroup {
                                id: format!("rg-{i}"),
                                name: format!("rg-{i}"),
                                location: "westeurope".into(),
                                subscription_id: "s".into(),
                                tags: HashMap::new(),
                            })
                            .collect(),
                    )),
                    failed_subscriptions: vec![],
                    notice: None,
                }
            }
            .boxed()
        }
    }

    fn rg_request(n: usize) -> LoadRequest {
        LoadRequest::ResourceGroups {
            subscription_ids: (0..n).map(|i| format!("sub-{i}")).collect(),
        }
    }

    #[tokio::test]
    async fn request_returns_immediately_and_result_arrives_later() {
        let mut loader = Loader::new(Some(Arc::new(FakeFetcher {
            delay: Duration::from_millis(50),
        })));
        let mut rx = loader.take_receiver().unwrap();

        let started = std::time::Instant::now();
        loader.request(rg_request(3)).unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(20),
            "request must not block"
        );
        assert!(loader.is_loading_kind(LoadKind::ResourceGroups));

        let result = rx.recv().await.unwrap();
        assert_eq!(loader.accept(&result), Accept::Final);
        assert!(!loader.is_loading());
        match result.outcome.data {
            Ok(LoadedData::ResourceGroups(rgs)) => assert_eq!(rgs.len(), 3),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn newer_request_supersedes_older_one() {
        let mut loader = Loader::new(Some(Arc::new(FakeFetcher {
            delay: Duration::from_millis(30),
        })));
        let mut rx = loader.take_receiver().unwrap();

        let first = loader.request(rg_request(1)).unwrap();
        let second = loader.request(rg_request(2)).unwrap();
        assert!(second > first);

        // The first task was aborted, so only the second result arrives.
        let result = rx.recv().await.unwrap();
        assert_eq!(result.generation, second);
        assert_eq!(loader.accept(&result), Accept::Final);

        // A late result with an old generation is rejected.
        let stale = LoadResult {
            done: true,
            kind: LoadKind::ResourceGroups,
            generation: first,
            outcome: FetchOutcome {
                data: Ok(LoadedData::ResourceGroups(vec![])),
                failed_subscriptions: vec![],
                notice: None,
            },
        };
        assert_eq!(loader.accept(&stale), Accept::Stale);
    }

    #[tokio::test]
    async fn different_kinds_load_concurrently() {
        let mut loader = Loader::new(Some(Arc::new(FakeFetcher {
            delay: Duration::from_millis(10),
        })));
        loader.request(LoadRequest::Subscriptions).unwrap();
        loader.request(rg_request(1)).unwrap();
        assert!(loader.is_loading_kind(LoadKind::Subscriptions));
        assert!(loader.is_loading_kind(LoadKind::ResourceGroups));
    }

    #[test]
    fn offline_loader_rejects_requests() {
        let mut loader = Loader::new(None);
        assert!(loader.request(LoadRequest::Subscriptions).is_err());
        assert!(!loader.is_loading());
    }

    #[tokio::test]
    async fn fetch_per_subscription_runs_concurrently_and_reports_partial_failures() {
        let subs: Vec<String> = (0..64).map(|i| format!("sub-{i}")).collect();
        let started = std::time::Instant::now();
        let (items, failed) = fetch_per_subscription(subs, |sub| async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if sub == "sub-7" {
                Err("forbidden")
            } else {
                Ok(vec![sub])
            }
        })
        .await;
        // 64 calls x 20ms sequentially would be 1.28s; 16 at a time is ~80ms.
        assert!(started.elapsed() < Duration::from_millis(600));
        assert_eq!(items.unwrap().len(), 63);
        assert_eq!(failed, vec!["sub-7".to_string()]);
    }

    #[tokio::test]
    async fn fetch_per_subscription_errors_only_when_everything_failed() {
        let (items, failed) = fetch_per_subscription(vec!["a".into(), "b".into()], |_| async {
            Err::<Vec<()>, _>("boom")
        })
        .await;
        assert_eq!(items.unwrap_err(), "boom");
        assert_eq!(failed.len(), 2);

        let (items, _) = fetch_per_subscription(Vec::<String>::new(), |_| async {
            Ok::<Vec<()>, &str>(vec![])
        })
        .await;
        assert!(items.unwrap().is_empty());
    }
}
