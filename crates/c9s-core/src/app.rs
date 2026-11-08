use crate::config::{self, Config, StartView};
use crate::provider::Provider;
use crate::{
    cache::{Cache, WriteJob},
    loader::{
        Accept, AzureFetcher, Fetcher, GraphFetcher, LoadKind, LoadReceiver, LoadRequest,
        LoadResult, LoadedData, Loader,
    },
    state::{AppState, DataSource, Freshness, ViewMode},
    timing,
};
use anyhow::Context;
use c9s_azure::management_groups::ManagementGroupClient;
use c9s_azure::resource_graph::{
    CachingTokenSource, CliTenantTokenSource, CredentialTokenSource, TokenSource,
};
use c9s_azure::{AuthProvider, AzureAuth, AzureClient, ResourceGraphClient};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};

/// Cache key of the management group tree.
const SCOPE_TREE_KEY: &str = "scope_tree";

/// Main application coordinator.
///
/// Owns the UI state, the local cache and the background [`Loader`]. Nothing
/// here awaits the network on the UI path: [`App::request`] starts a load and
/// returns immediately, and the event loop hands results back via
/// [`App::apply`].
pub struct App {
    state: AppState,
    cache: Cache,
    loader: Loader,
    /// The in-flight request of each kind
    pending: HashMap<LoadKind, Pending>,
    /// The latest request for the resource list: what `resources` holds
    resources_request: Option<LoadRequest>,
    /// Builds the fetcher for a tenant (`None`: the CLI's default login),
    /// for `:ctx`. Absent offline and in tests that don't switch.
    fetcher_factory: Option<FetcherFactory>,
}

/// Builds a fetcher that talks to Azure as a given tenant.
/// Builds the fetcher for a context: its cloud, and the context within it
/// (an Azure tenant, `None` for the CLI's default login; a gcloud
/// configuration, `None` for gcloud's active one).
pub type FetcherFactory =
    Arc<dyn Fn(Provider, Option<&str>) -> Option<Arc<dyn Fetcher>> + Send + Sync>;

/// A request in flight, as the App tracks it.
struct Pending {
    generation: u64,
    request: LoadRequest,
    /// Rows were on screen when the request started (cached or older data).
    /// Streamed chunks then collect in `staging` and replace the list in one
    /// swap at the end, instead of the list shrinking to the first page.
    staged: bool,
    staging: Option<LoadedData>,
}

impl App {
    /// Create the application: open the cache, show cached subscriptions right
    /// away, and start fetching fresh subscriptions in the background.
    pub async fn new() -> anyhow::Result<Self> {
        Self::start(Config::load(Config::path(None))).await
    }

    /// Like [`App::new`], with an already loaded config (`--config`).
    pub async fn start(loaded: config::Loaded) -> anyhow::Result<Self> {
        let _t = timing::timer("startup.app_init");
        info!("Initializing c9s application");

        let cache_dir = Self::cache_dir()?;
        std::fs::create_dir_all(&cache_dir).context("Failed to create cache directory")?;
        // The cache holds the tenant's inventory, role assignments and
        // directory names: readable by this user only.
        crate::paths::make_private(&cache_dir, true);
        let db = cache_dir.join("c9s.db");
        let cache = Cache::new(&db)?;
        for file in ["c9s.db", "c9s.db-wal", "c9s.db-shm"] {
            crate::paths::make_private(&cache_dir.join(file), false);
        }

        let auth = match AzureAuth::new_auto().await {
            Ok(auth) => Some(Arc::new(auth)),
            Err(e) => {
                warn!("Azure authentication failed: {}", e);
                None
            }
        };
        let fetcher = auth.clone().map(|auth| Self::azure_fetcher(auth, None));
        if let Some(fetcher) = &fetcher {
            fetcher.warm_up();
        }
        let mut app = Self::with_fetcher(cache, fetcher);
        // No Azure CLI at all: someone using c9s for Google Cloud or AWS.
        let azure = auth.is_some() && on_path("az");
        let aws_role = loaded.config.aws_role.clone();
        // Every cloud's fetcher: Google Cloud and AWS work without an
        // Azure login.
        app.set_fetcher_factory(Arc::new(
            move |provider: Provider, context: Option<&str>| {
                let fetcher: Arc<dyn Fetcher> = match provider {
                    Provider::Azure => Self::azure_fetcher(auth.clone()?, context),
                    Provider::Gcp => {
                        Arc::new(crate::gcp::GcpFetcher::new(context.map(str::to_string)))
                    }
                    Provider::Aws => {
                        Arc::new(crate::aws::AwsFetcher::new(context?, aws_role.as_deref()))
                    }
                };
                fetcher.warm_up();
                Some(fetcher)
            },
        ));
        if !azure {
            app.start_without_azure(loaded);
            return Ok(app);
        }
        app.apply_config(loaded);
        Ok(app)
    }

    /// Start without the Azure CLI: on `:ctx`, so a Google Cloud or AWS
    /// context can be picked; Azure requests say how to sign in.
    pub fn start_without_azure(&mut self, loaded: config::Loaded) {
        self.state.config = loaded.config;
        self.loader
            .replace_fetcher(Some(Arc::new(crate::loader::ContextsOnly)));
        self.state.view = ViewMode::TenantList;
        self.state.list_state.reset();
        self.request(LoadRequest::CloudContexts);
        self.state.notice = Some(match loaded.problem {
            Some(problem) => format!("Config {problem}"),
            None => "No Azure CLI (az) found: pick a Google Cloud or AWS context, \
                     or install the Azure CLI, run az login and restart"
                .into(),
        });
    }

    /// Start with `loaded`: the first view, and the background loads. Says
    /// in the status bar if the config file couldn't be used.
    pub fn apply_config(&mut self, loaded: config::Loaded) {
        self.state.config = loaded.config;
        if let Some(tenant) = self.state.config.tenant.clone() {
            if let Some(factory) = &self.fetcher_factory {
                self.loader
                    .replace_fetcher(factory(Provider::Azure, Some(&tenant)));
            }
            self.state.tenant = Some(tenant);
        }
        self.load_cached_subscriptions();
        self.request(LoadRequest::Subscriptions);
        match self.state.config.start {
            StartView::Mg => self.open_scope_tree(),
            StartView::Sub => {
                // The tree still loads (for :mg and the palette), but the
                // subscription list is the start view.
                self.state.view = ViewMode::SubscriptionList;
                self.request(LoadRequest::ScopeTree);
            }
        }
        if let Some(problem) = loaded.problem {
            self.state.notice = Some(format!("Config {problem}"));
        }
    }

    /// Resource Graph for lists (with ARM fallback), unless `C9S_BACKEND=arm`.
    ///
    /// With a `tenant`, Azure is asked as that tenant: tokens come from
    /// `az account get-access-token --tenant`. The ARM fallback keeps the
    /// CLI's default login.
    fn azure_fetcher(auth: Arc<AzureAuth>, tenant: Option<&str>) -> Arc<dyn Fetcher> {
        let arm: Arc<dyn Fetcher> =
            Arc::new(AzureFetcher::new(Arc::new(AzureClient::new(auth.clone()))));
        if crate::paths::env_var("C9S_BACKEND").is_some_and(|v| v.eq_ignore_ascii_case("arm")) {
            info!("Using ARM backend (C9S_BACKEND=arm)");
            return arm;
        }
        let tokens: Arc<dyn TokenSource> = match tenant {
            Some(tenant) => Arc::new(CachingTokenSource::new(CliTenantTokenSource::new(tenant))),
            None => Arc::new(CachingTokenSource::new(CredentialTokenSource(
                auth.credential(),
            ))),
        };
        let management_groups = Arc::new(ManagementGroupClient::new(tokens.clone()));
        // Principal names come from Microsoft Graph: same login, its audience.
        let graph_tokens: Arc<dyn TokenSource> = match tenant {
            Some(tenant) => Arc::new(CachingTokenSource::new(CliTenantTokenSource::for_resource(
                tenant,
                c9s_azure::directory::GRAPH_RESOURCE,
            ))),
            None => Arc::new(CachingTokenSource::new(
                c9s_azure::resource_graph::ScopedCredentialTokenSource {
                    credential: auth.credential(),
                    scope: format!("{}.default", c9s_azure::directory::GRAPH_RESOURCE),
                },
            )),
        };
        let directory = Arc::new(c9s_azure::directory::DirectoryClient::new(graph_tokens));
        Arc::new(
            GraphFetcher::new(Arc::new(ResourceGraphClient::new(tokens)), arm)
                .with_management_groups(management_groups)
                .with_directory(directory),
        )
    }

    /// Create an application with no Azure connection, backed by `cache`.
    /// Every load request fails with a "not connected" error.
    pub fn offline(cache: Cache) -> Self {
        Self::with_fetcher(cache, None)
    }

    /// Create an application that loads data through `fetcher`.
    pub fn with_fetcher(cache: Cache, fetcher: Option<Arc<dyn Fetcher>>) -> Self {
        Self {
            state: AppState::new(),
            cache,
            loader: Loader::new(fetcher),
            pending: HashMap::new(),
            resources_request: None,
            fetcher_factory: None,
        }
    }

    /// How to build a fetcher for another tenant (enables `:ctx`).
    pub fn set_fetcher_factory(&mut self, factory: FetcherFactory) {
        self.fetcher_factory = Some(factory);
    }

    /// Work in `tenant` from now on (`None`: the CLI's default login, all
    /// tenants' subscriptions listed). Azure is asked as that tenant;
    /// every list that belonged to the previous tenant is dropped and the
    /// start view reloads (its management group tree is cached per tenant).
    pub fn switch_tenant(&mut self, tenant: Option<String>) {
        info!("Switching tenant to {:?}", tenant);
        if let Some(factory) = &self.fetcher_factory {
            self.loader
                .replace_fetcher(factory(Provider::Azure, tenant.as_deref()));
        }
        // Coming from another cloud: its accounts aren't Azure's.
        if self.state.provider != Provider::Azure {
            self.state.provider = Provider::Azure;
            self.state.gcp_configuration = None;
            self.state.aws_profile = None;
            self.state.subscriptions.clear();
            self.load_cached_subscriptions();
            self.request(LoadRequest::Subscriptions);
        }
        self.pending.clear();
        let state = &mut self.state;
        state.tenant = tenant;
        state.resource_groups.clear();
        state.resources.clear();
        state.scope_tree = None;
        state.tree_expanded.clear();
        state.scope_group = None;
        state.selected_subscription = None;
        state.selected_resource_detail = None;
        state.resource_document = None;
        state.resource_query = Default::default();
        state.cross_subscription_mode = false;
        state.clear_navigation();
        state.filter.clear();
        state.freshness.clear();
        state.compliance_rollup = None;
        state.set_loading(false);
        match state.config.start {
            StartView::Sub => {
                state.view = ViewMode::SubscriptionList;
                state.list_state.reset();
                self.request(LoadRequest::ScopeTree);
            }
            StartView::Mg => self.open_scope_tree(),
        }
        if let Some(name) = self.state.tenant_label() {
            self.state.notice = Some(format!("Switched to tenant {name}"));
        } else {
            self.state.notice =
                Some("Switched to all tenants (the Azure CLI's default login)".into());
        }
    }

    /// Work in a Google Cloud context from now on: a gcloud configuration
    /// (`None`: gcloud's active one). The lists of the previous context are
    /// dropped and the projects load.
    pub fn switch_to_gcp(&mut self, configuration: Option<String>) {
        info!("Switching to GCP configuration {:?}", configuration);
        let notice = format!(
            "Switched to Google Cloud ({})",
            configuration
                .as_deref()
                .unwrap_or("gcloud's active configuration")
        );
        self.switch_cloud(Provider::Gcp, configuration, notice);
    }

    /// Work in an AWS context from now on: an AWS CLI profile (its one
    /// account). The lists of the previous context are dropped and the
    /// account loads.
    pub fn switch_to_aws(&mut self, profile: String) {
        info!("Switching to AWS profile {}", profile);
        let notice = format!("Switched to AWS (profile {profile})");
        self.switch_cloud(Provider::Aws, Some(profile), notice);
    }

    fn switch_cloud(&mut self, provider: Provider, context: Option<String>, notice: String) {
        if let Some(factory) = &self.fetcher_factory {
            self.loader
                .replace_fetcher(factory(provider, context.as_deref()));
        }
        self.pending.clear();
        let state = &mut self.state;
        state.provider = provider;
        state.gcp_configuration = None;
        state.aws_profile = None;
        match provider {
            Provider::Gcp => state.gcp_configuration = context,
            Provider::Aws => state.aws_profile = context,
            Provider::Azure => {}
        }
        state.tenant = None;
        state.subscriptions.clear();
        state.resource_groups.clear();
        state.resources.clear();
        state.scope_tree = None;
        state.tree_expanded.clear();
        state.scope_group = None;
        state.selected_subscription = None;
        state.selected_resource_detail = None;
        state.resource_document = None;
        state.resource_query = Default::default();
        state.cross_subscription_mode = false;
        state.clear_navigation();
        state.filter.clear();
        state.freshness.clear();
        state.compliance_rollup = None;
        state.set_loading(false);
        state.view = ViewMode::SubscriptionList;
        state.list_state.reset();
        self.request(LoadRequest::Subscriptions);
        self.state.notice = Some(notice);
    }

    /// Cache key of the policy assignments for the current tenant.
    fn policy_key(&self) -> String {
        match &self.state.tenant {
            Some(tenant) => format!("policy_assignments:{}", tenant.to_ascii_lowercase()),
            None => "policy_assignments".into(),
        }
    }

    /// Cache key of a tenant-wide document (`prefix:<tenant>`).
    fn tenant_key(&self, prefix: &str) -> String {
        match &self.state.tenant {
            Some(tenant) => format!("{prefix}:{}", tenant.to_ascii_lowercase()),
            None => prefix.to_string(),
        }
    }

    /// The enabled subscriptions of the current tenant: the fallback scope
    /// for tenant-wide queries without management group access.
    fn tenant_subscription_ids(&self) -> Vec<String> {
        self.state
            .subscriptions
            .iter()
            .filter(|s| self.state.in_tenant(s) && !s.state.eq_ignore_ascii_case("Disabled"))
            .map(|s| s.id.clone())
            .collect()
    }

    /// Load role assignments and definitions (cached first) unless loading.
    pub fn load_rbac(&mut self) {
        let subscription_ids = self.tenant_subscription_ids();
        if !self.pending.contains_key(&LoadKind::RoleAssignments) {
            self.request(LoadRequest::RoleAssignments {
                subscription_ids: subscription_ids.clone(),
            });
        }
        if !self.pending.contains_key(&LoadKind::RoleDefinitions) {
            self.request(LoadRequest::RoleDefinitions { subscription_ids });
        }
    }

    /// Name the principals of the loaded role assignments that have no name
    /// yet (Microsoft Graph), unless a lookup is running.
    pub fn load_principal_names(&mut self) {
        if self.pending.contains_key(&LoadKind::Principals) {
            return;
        }
        let mut ids: Vec<String> = self
            .state
            .role_assignments
            .iter()
            .map(|a| a.principal_id.to_ascii_lowercase())
            .filter(|id| !id.is_empty() && !self.state.principals.contains_key(id))
            .collect();
        ids.sort();
        ids.dedup();
        if !ids.is_empty() {
            self.request(LoadRequest::Principals { ids });
        }
    }

    /// `:ra`: the role assignments that matter at `scope` (`None`: every one
    /// in the tenant), optionally only those of one role (its GUID).
    pub fn open_role_assignments(
        &mut self,
        scope: Option<crate::policy::PolicyScope>,
        role: Option<String>,
    ) {
        let state = &mut self.state;
        state.ra_scope = scope;
        state.ra_role = role;
        state.ra_principal = None;
        state.view = ViewMode::RoleAssignments;
        state.list_state.reset();
        self.load_rbac();
        self.load_principal_names();
    }

    /// One principal's access across the tenant: its own role assignments
    /// and its groups' (nested groups too, from Microsoft Graph).
    pub fn open_access(&mut self, principal_id: &str) {
        let id = principal_id.to_ascii_lowercase();
        let state = &mut self.state;
        state.ra_scope = None;
        state.ra_role = None;
        state.ra_principal = Some(id.clone());
        state.view = ViewMode::RoleAssignments;
        state.list_state.reset();
        self.load_rbac();
        self.load_principal_names();
        if !self.state.principal_groups.contains_key(&id) {
            self.request(LoadRequest::PrincipalGroups { principal_id: id });
        }
    }

    /// `:app`: every application (cached first).
    pub fn open_apps(&mut self) {
        self.state.view = ViewMode::AppList;
        self.state.list_state.reset();
        self.request(LoadRequest::Applications);
    }

    /// One application's tabs; its details load fresh.
    pub fn open_app(&mut self, row: crate::apps::AppRow) {
        let request = LoadRequest::AppDetail {
            app_id: row.app_id.clone(),
            application_id: row.app.as_ref().map(|a| a.id.clone()),
            service_principal_id: row.sp.as_ref().map(|s| s.id.clone()),
        };
        let state = &mut self.state;
        if state
            .app_detail
            .as_ref()
            .is_some_and(|(id, _)| *id != row.app_id)
        {
            state.app_detail = None;
        }
        state.app_open = Some(row);
        state.app_tab = Default::default();
        state.detail_scroll = 0;
        state.clear_detail_search();
        state.app_field = crate::app_view::default_field(&crate::app_view::overview_fields(state));
        state.view = ViewMode::AppDetail;
        self.request(request);
    }

    /// Reload the open application's details, keeping its tab.
    pub fn refresh_app(&mut self) {
        let Some(row) = self.state.app_open.clone() else {
            return;
        };
        self.request(LoadRequest::AppDetail {
            app_id: row.app_id.clone(),
            application_id: row.app.as_ref().map(|a| a.id.clone()),
            service_principal_id: row.sp.as_ref().map(|s| s.id.clone()),
        });
    }

    /// `:group`: every group in the directory (cached first).
    pub fn open_groups(&mut self) {
        self.state.view = ViewMode::GroupList;
        self.state.list_state.reset();
        self.request(LoadRequest::Groups);
    }

    /// A group's members, direct or nested too; loaded fresh each time.
    pub fn open_group_members(&mut self, group_id: &str, transitive: bool) {
        let id = group_id.to_ascii_lowercase();
        let state = &mut self.state;
        state.members_group = Some(id.clone());
        state.members_transitive = transitive;
        state.view = ViewMode::GroupMembers;
        state.list_state.reset();
        state.group_members.remove(&(id.clone(), transitive));
        self.request(LoadRequest::GroupMembers {
            group_id: id,
            transitive,
        });
    }

    /// The groups a user, group or app belongs to, nested ones too.
    pub fn open_member_of(&mut self, principal_id: &str) {
        let id = principal_id.to_ascii_lowercase();
        let state = &mut self.state;
        state.member_of = Some(id.clone());
        state.view = ViewMode::MemberOf;
        state.list_state.reset();
        state.principal_groups.remove(&id);
        self.request(LoadRequest::PrincipalGroups { principal_id: id });
    }

    /// `:role`: every role definition, custom first.
    pub fn open_role_definitions(&mut self) {
        self.state.view = ViewMode::RoleDefinitions;
        self.state.list_state.reset();
        self.load_rbac();
    }

    /// Cache key of the policy exemptions for the current tenant.
    fn exemptions_key(&self) -> String {
        match &self.state.tenant {
            Some(tenant) => format!("policy_exemptions:{}", tenant.to_ascii_lowercase()),
            None => "policy_exemptions".into(),
        }
    }

    /// Load the tenant's policy exemptions (cached first) unless loading.
    pub fn load_exemptions(&mut self) {
        if !self.pending.contains_key(&LoadKind::PolicyExemptions) {
            self.request(LoadRequest::PolicyExemptions);
        }
    }

    /// `:exemption`: the exemptions at or below `scope` (`None`: the whole
    /// tenant), soonest to expire first.
    pub fn open_exemptions(&mut self, scope: Option<crate::policy::PolicyScope>) {
        let state = &mut self.state;
        state.exemption_scope = scope;
        state.view = ViewMode::ExemptionList;
        state.list_state.reset();
        self.request(LoadRequest::PolicyExemptions);
        // Names of the assignments they exempt from.
        if self.state.policy_assignments.is_empty() {
            self.request(LoadRequest::PolicyAssignments);
        }
    }

    /// Cache key of the compliance roll-up for the current tenant.
    fn rollup_key(&self) -> String {
        match &self.state.tenant {
            Some(tenant) => format!("compliance_rollup:{}", tenant.to_ascii_lowercase()),
            None => "compliance_rollup".into(),
        }
    }

    /// Load the tenant's compliance per subscription (cached first), for the
    /// tree's compliance column and `:compliance`. Needs Resource Graph
    /// access at the tenant root, like `:policy all`.
    pub fn load_compliance_rollup(&mut self) {
        if !self.pending.contains_key(&LoadKind::ComplianceRollup) {
            self.request(LoadRequest::ComplianceRollup);
        }
    }

    /// Show the policy assignments that apply at `scope` (cached first),
    /// and load fresh assignments and compliance counts in the background.
    pub fn open_policy(&mut self, scope: crate::policy::PolicyScope) {
        let state = &mut self.state;
        state.view = ViewMode::PolicyAssignments;
        state.list_state.reset();
        state.policy_scope = Some(scope.clone());
        if state
            .policy_compliance
            .as_ref()
            .is_some_and(|(s, _)| *s != scope)
        {
            state.policy_compliance = None;
        }
        if state
            .compliance_summary
            .as_ref()
            .is_some_and(|(s, _)| *s != scope)
        {
            state.compliance_summary = None;
        }
        self.request(LoadRequest::PolicyAssignments);
        self.request(LoadRequest::PolicyCompliance {
            scope: scope.clone(),
        });
        self.request(LoadRequest::ComplianceSummary { scope });
    }

    /// Show the resources that are non-compliant with `assignment` in the
    /// policy view's scope, loaded fresh (policy states change often and are
    /// not cached).
    pub fn open_non_compliant(&mut self, assignment: crate::policy::PolicyAssignment) {
        let Some(scope) = self.state.policy_scope.clone() else {
            return;
        };
        let state = &mut self.state;
        let same = state
            .policy_states_assignment
            .as_ref()
            .is_some_and(|a| a.id.eq_ignore_ascii_case(&assignment.id));
        if !same {
            state.policy_states.clear();
        }
        let assignment_id = assignment.id.clone();
        state.policy_states_assignment = Some(assignment);
        state.view = ViewMode::PolicyNonCompliant;
        state.list_state.reset();
        self.request(LoadRequest::PolicyStates {
            scope,
            assignment_id,
        });
    }

    /// Load the policy states of the resource in the detail view (for its
    /// Policy tab), unless they are loaded already or `force`.
    pub fn load_resource_policy(&mut self, force: bool) {
        let Some(resource) = &self.state.selected_resource_detail else {
            return;
        };
        let loaded = self
            .state
            .resource_policy
            .as_ref()
            .is_some_and(|(id, _)| id.eq_ignore_ascii_case(&resource.id));
        if loaded && !force {
            return;
        }
        if !loaded {
            self.state.resource_policy = None;
        }
        let request = LoadRequest::ResourcePolicy {
            resource_id: resource.id.clone(),
            subscription_id: resource.subscription_id.clone(),
        };
        self.request(request);
        // Exempt states link to their exemption.
        if self.state.policy_exemptions.is_empty() {
            self.load_exemptions();
        }
    }

    /// Cache key of the management group tree for the current tenant.
    fn scope_tree_key(&self) -> String {
        if self.state.provider == Provider::Gcp {
            let configuration = self.state.gcp_configuration.as_deref().unwrap_or("active");
            return format!("{SCOPE_TREE_KEY}:gcp:{configuration}");
        }
        match &self.state.tenant {
            Some(tenant) => format!("{SCOPE_TREE_KEY}:{}", tenant.to_ascii_lowercase()),
            None => SCOPE_TREE_KEY.to_string(),
        }
    }

    /// Switch to the management group tree (the start view) and load it:
    /// the cached tree at once, the fresh one in the background. If it can't
    /// be loaded and nothing is cached, [`App::apply`] falls back to the
    /// subscription list.
    pub fn open_scope_tree(&mut self) {
        self.state.view = ViewMode::ManagementGroupTree;
        self.state.list_state.reset();
        self.request(LoadRequest::ScopeTree);
        // The tree's compliance column.
        self.load_compliance_rollup();
    }

    /// Show `resource` in the detail view now and load its full document
    /// (cached first, then from Azure).
    pub fn open_resource_detail(&mut self, resource: crate::models::Resource) {
        let request = LoadRequest::ResourceDetail {
            resource_id: resource.id.clone(),
            subscription_id: resource.subscription_id.clone(),
        };
        let state = &mut self.state;
        state.selected_resource_detail = Some(resource);
        state.resource_document = None;
        state.freshness.remove(&ViewMode::ResourceDetail);
        state.detail_tab = Default::default();
        state.detail_scroll = 0;
        state.detail_cursor = 0;
        state.clear_detail_search();
        state.view = ViewMode::ResourceDetail;
        state.list_state.reset();
        self.request(request);
    }

    /// The latest request for the resource list (what the list shows).
    pub fn resources_request(&self) -> Option<&LoadRequest> {
        self.resources_request.as_ref()
    }

    /// Reload the open resource's document.
    pub fn refresh_resource_detail(&mut self) {
        if let Some(resource) = &self.state.selected_resource_detail {
            let request = LoadRequest::ResourceDetail {
                resource_id: resource.id.clone(),
                subscription_id: resource.subscription_id.clone(),
            };
            self.request(request);
        }
    }

    /// The tree couldn't be shown: go to the subscription list and say why.
    fn fall_back_to_subscriptions(&mut self, reason: &str) {
        if self.state.view == ViewMode::ManagementGroupTree {
            self.state.view = ViewMode::SubscriptionList;
            self.state.list_state.reset();
        }
        if self.state.provider != Provider::Azure {
            let terms = self.state.provider.terms();
            self.state.notice = Some(format!(
                "{} unavailable: {reason}. Showing {}",
                terms.tree,
                terms.accounts.to_lowercase()
            ));
            return;
        }
        // Azure's 403 message is long (it names the user and the scope); for a
        // user without management group access, say so plainly instead.
        let forbidden = reason.starts_with("HTTP 403") || reason.contains("AuthorizationFailed");
        self.state.notice = Some(if forbidden {
            "No read access to management groups: showing subscriptions \
             (:sub, :rg, :res work; :mg and :policy all need management group access)"
                .to_string()
        } else {
            format!("Management group tree unavailable ({reason}); showing subscriptions")
        });
    }

    /// Show cached subscriptions immediately (before Azure answers).
    fn load_cached_subscriptions(&mut self) {
        let _t = timing::timer("cache.read_subscriptions");
        match self.cache.repository().cached_subscriptions() {
            Ok(cached) if !cached.items.is_empty() => {
                info!("Showing {} cached subscriptions", cached.items.len());
                self.state.subscriptions = cached.items;
                self.mark(LoadKind::Subscriptions, DataSource::Cache, cached.oldest);
            }
            Ok(_) => {}
            Err(e) => warn!("Failed to read cached subscriptions: {}", e),
        }
    }

    /// Fill an empty list from the cache for this request's scope, so the
    /// view shows data instantly while Azure is asked in the background.
    fn show_cached(&mut self, request: &LoadRequest) {
        let tree_key = self.scope_tree_key();
        let repo = self.cache.repository();
        let result = match request {
            LoadRequest::Subscriptions
            | LoadRequest::Tenants
            | LoadRequest::CloudContexts
            | LoadRequest::PrincipalGroups { .. }
            | LoadRequest::GroupMembers { .. }
            | LoadRequest::AppDetail { .. }
            | LoadRequest::PolicyStates { .. }
            | LoadRequest::ResourcePolicy { .. } => return,
            LoadRequest::Applications => {
                repo.document(&self.tenant_key("applications")).map(|doc| {
                    match doc.and_then(|(json, at)| {
                        serde_json::from_str::<(Vec<_>, Vec<_>)>(&json)
                            .ok()
                            .map(|v| (v, at))
                    }) {
                        Some(((apps, sps), at)) => (LoadedData::Applications(apps, sps), Some(at)),
                        None => (LoadedData::Applications(vec![], vec![]), None),
                    }
                })
            }
            LoadRequest::Groups => repo.document(&self.tenant_key("groups")).map(|doc| {
                match doc.and_then(|(json, at)| Some((serde_json::from_str(&json).ok()?, at))) {
                    Some((items, at)) => (LoadedData::Groups(items), Some(at)),
                    None => (LoadedData::Groups(vec![]), None),
                }
            }),
            LoadRequest::Principals { .. } => {
                repo.document(&self.tenant_key("principals")).map(|doc| {
                    match doc.and_then(|(json, at)| Some((serde_json::from_str(&json).ok()?, at))) {
                        Some((items, at)) => (LoadedData::Principals(items), Some(at)),
                        None => (LoadedData::Principals(vec![]), None),
                    }
                })
            }
            LoadRequest::RoleAssignments { .. } => repo
                .document(&self.tenant_key("role_assignments"))
                .map(|doc| {
                    match doc.and_then(|(json, at)| Some((serde_json::from_str(&json).ok()?, at))) {
                        Some((items, at)) => (LoadedData::RoleAssignments(items), Some(at)),
                        None => (LoadedData::RoleAssignments(vec![]), None),
                    }
                }),
            LoadRequest::RoleDefinitions { .. } => repo
                .document(&self.tenant_key("role_definitions"))
                .map(|doc| {
                    match doc.and_then(|(json, at)| Some((serde_json::from_str(&json).ok()?, at))) {
                        Some((items, at)) => (LoadedData::RoleDefinitions(items), Some(at)),
                        None => (LoadedData::RoleDefinitions(vec![]), None),
                    }
                }),
            LoadRequest::PolicyExemptions => repo.document(&self.exemptions_key()).map(|doc| {
                match doc.and_then(|(json, at)| Some((serde_json::from_str(&json).ok()?, at))) {
                    Some((items, at)) => (LoadedData::PolicyExemptions(items), Some(at)),
                    None => (LoadedData::PolicyExemptions(vec![]), None),
                }
            }),
            LoadRequest::ComplianceRollup => repo.document(&self.rollup_key()).map(|doc| match doc
                .and_then(|(json, at)| Some((serde_json::from_str(&json).ok()?, at)))
            {
                Some((counts, at)) => (LoadedData::ComplianceRollup(counts), Some(at)),
                None => (LoadedData::ComplianceRollup(Default::default()), None),
            }),
            LoadRequest::PolicyAssignments => {
                repo.document(&self.policy_key()).map(|doc| {
                    match doc.and_then(|(json, at)| Some((serde_json::from_str(&json).ok()?, at))) {
                        Some((items, at)) => (LoadedData::PolicyAssignments(items), Some(at)),
                        None => (LoadedData::PolicyAssignments(vec![]), None),
                    }
                })
            }
            LoadRequest::PolicyCompliance { scope } => repo
                .document(&format!("policy_compliance:{}", scope.arm_id()))
                .map(|doc| {
                    match doc.and_then(|(json, at)| Some((serde_json::from_str(&json).ok()?, at))) {
                        Some((counts, at)) => (
                            LoadedData::PolicyCompliance(scope.clone(), counts),
                            Some(at),
                        ),
                        None => (
                            LoadedData::PolicyCompliance(scope.clone(), Default::default()),
                            None,
                        ),
                    }
                }),
            LoadRequest::ComplianceSummary { scope } => repo
                .document(&format!("compliance_summary:{}", scope.arm_id()))
                .map(|doc| {
                    match doc.and_then(|(json, at)| Some((serde_json::from_str(&json).ok()?, at))) {
                        Some((counts, at)) => (
                            LoadedData::ComplianceSummary(scope.clone(), counts),
                            Some(at),
                        ),
                        None => (
                            LoadedData::ComplianceSummary(scope.clone(), Default::default()),
                            None,
                        ),
                    }
                }),
            LoadRequest::ScopeTree => repo.document(&tree_key).map(|doc| match doc {
                Some((json, cached_at)) => match serde_json::from_str(&json) {
                    Ok(tree) => (LoadedData::ScopeTree(tree), Some(cached_at)),
                    Err(e) => {
                        warn!("Ignoring unreadable cached tree: {}", e);
                        (LoadedData::ScopeTree(Default::default()), None)
                    }
                },
                None => (LoadedData::ScopeTree(Default::default()), None),
            }),
            LoadRequest::ResourceDetail { resource_id, .. } => repo
                .cached_resource_detail(resource_id)
                .map(|cached| match cached {
                    Some((doc, cached_at)) => (LoadedData::ResourceDetail(doc), Some(cached_at)),
                    None => (LoadedData::ResourceDetail(serde_json::Value::Null), None),
                }),
            LoadRequest::ResourceGroups { subscription_ids } => {
                let _t = timing::timer("cache.read_resource_groups");
                repo.cached_resource_groups(subscription_ids).map(|c| {
                    let oldest = c.oldest;
                    (LoadedData::ResourceGroups(c.items), oldest)
                })
            }
            LoadRequest::Resources {
                subscription_ids,
                resource_group,
            } => {
                let _t = timing::timer("cache.read_resources");
                repo.cached_resources(subscription_ids, resource_group.as_deref())
                    .map(|c| {
                        let oldest = c.oldest;
                        (LoadedData::Resources(c.items), oldest)
                    })
            }
        };
        match result {
            Ok((data, Some(oldest))) => {
                self.put(data, false);
                self.mark(request.kind(), DataSource::Cache, Some(oldest));
            }
            Ok((_, None)) => {}
            Err(e) => warn!("Failed to read cache: {}", e),
        }
    }

    /// Record where a list's data came from and when.
    fn mark(&mut self, kind: LoadKind, source: DataSource, as_of: Option<i64>) {
        let as_of = as_of.unwrap_or_else(|| chrono::Utc::now().timestamp());
        self.state
            .freshness
            .insert(view_of(kind), Freshness { source, as_of });
        self.state.set_data_source(source);
    }

    fn list_len(&self, kind: LoadKind) -> usize {
        match kind {
            LoadKind::Subscriptions => self.state.subscriptions.len(),
            LoadKind::ResourceGroups => self.state.resource_groups.len(),
            LoadKind::Resources => self.state.resources.len(),
            LoadKind::ScopeTree => usize::from(self.state.scope_tree.is_some()),
            LoadKind::ResourceDetail => usize::from(self.state.resource_document.is_some()),
            LoadKind::Tenants => self.state.tenants.len(),
            LoadKind::CloudContexts => {
                self.state.gcp_configurations.len() + self.state.aws_profiles.len()
            }
            LoadKind::PolicyAssignments => self.state.policy_assignments.len(),
            LoadKind::PolicyExemptions => self.state.policy_exemptions.len(),
            LoadKind::RoleAssignments => self.state.role_assignments.len(),
            LoadKind::RoleDefinitions => self.state.role_definitions.len(),
            LoadKind::Principals => self.state.principals.len(),
            LoadKind::PrincipalGroups => self.state.principal_groups.len(),
            LoadKind::Groups => self.state.groups.len(),
            LoadKind::Applications => {
                self.state.applications.len() + self.state.service_principals.len()
            }
            LoadKind::AppDetail => usize::from(self.state.app_detail.is_some()),
            LoadKind::GroupMembers => self.state.group_members.len(),
            LoadKind::ComplianceRollup => usize::from(self.state.compliance_rollup.is_some()),
            LoadKind::PolicyCompliance => usize::from(self.state.policy_compliance.is_some()),
            LoadKind::ComplianceSummary => usize::from(self.state.compliance_summary.is_some()),
            LoadKind::PolicyStates => self.state.policy_states.len(),
            LoadKind::ResourcePolicy => usize::from(self.state.resource_policy.is_some()),
        }
    }

    /// Start loading `request` in the background. Never blocks.
    ///
    /// If the target list is empty, it is first filled from the cache, so the
    /// view shows (possibly stale) data immediately. Errors (e.g. not
    /// connected) are shown in the UI; there is nothing for the caller to
    /// handle.
    pub fn request(&mut self, request: LoadRequest) {
        self.state.notice = None;
        let kind = request.kind();
        if kind == LoadKind::Resources {
            self.resources_request = Some(request.clone());
        }
        if self.list_len(kind) == 0 {
            self.show_cached(&request);
        }
        let staged = self.list_len(kind) > 0;
        match self.loader.request(request.clone()) {
            Ok(generation) => {
                self.pending.insert(
                    kind,
                    Pending {
                        generation,
                        request,
                        staged,
                        staging: None,
                    },
                );
            }
            Err(e) if staged => self.state.notice = Some(format!("{e}; showing cached data")),
            Err(e) if kind == LoadKind::ScopeTree => self.fall_back_to_subscriptions(&e),
            // Exemptions loaded for the Policy tab are optional too.
            Err(e)
                if kind == LoadKind::PolicyExemptions
                    && self.state.view != ViewMode::ExemptionList =>
            {
                warn!("Policy exemptions unavailable: {}", e)
            }
            // The tree's compliance column is optional: no popup for it.
            Err(e)
                if kind == LoadKind::ComplianceRollup
                    && self.state.view != ViewMode::ComplianceRanking =>
            {
                warn!("Compliance roll-up unavailable: {}", e)
            }
            Err(e) => self.state.set_error(Some(e)),
        }
        self.state.set_loading(self.loader.is_loading());
    }

    /// The channel on which load results arrive. The event loop takes it once
    /// and passes each result to [`App::apply`].
    pub fn take_load_receiver(&mut self) -> Option<LoadReceiver> {
        self.loader.take_receiver()
    }

    /// Whether a load of `kind` is in flight.
    pub fn is_loading(&self, kind: LoadKind) -> bool {
        self.loader.is_loading_kind(kind)
    }

    /// Apply a load result: a streamed chunk or the final message of a
    /// request. Stale results (a newer request of the same kind exists) are
    /// ignored. A successful final result replaces the cached scope.
    pub fn apply(&mut self, result: LoadResult) {
        let kind = result.kind;
        let accept = self.loader.accept(&result);
        if accept == Accept::Stale {
            return;
        }
        let Some(pending) = self
            .pending
            .get_mut(&kind)
            .filter(|p| p.generation == result.generation)
        else {
            return;
        };
        let outcome = result.outcome;

        if accept == Accept::Partial {
            if let Ok(data) = outcome.data {
                if pending.staged {
                    merge(&mut pending.staging, data);
                } else {
                    self.put(data, true);
                }
            }
            return;
        }

        let pending = self.pending.remove(&kind).expect("checked above");
        self.state.set_loading(self.loader.is_loading());
        if let Some(notice) = outcome.notice {
            self.state.notice = Some(notice);
        }

        match outcome.data {
            Ok(data) => {
                if pending.staged {
                    let mut all = pending.staging;
                    merge(&mut all, data);
                    self.swap_in(all.expect("merge always sets a value"));
                } else {
                    self.put(data, true);
                }
                self.write_scope(&pending.request, &outcome.failed_subscriptions);
                self.mark(kind, DataSource::Azure, None);
                if kind == LoadKind::RoleAssignments {
                    // New principals may have appeared: name them.
                    self.load_principal_names();
                }
                let failed = outcome.failed_subscriptions.len();
                if failed > 0 {
                    self.state.notice = Some(format!(
                        "{} subscription(s) failed to load {}",
                        failed,
                        kind.label()
                    ));
                }
            }
            Err(e) => {
                warn!("Loading {} failed: {}", kind.label(), e);
                // Cached rows are still on screen: a non-blocking notice is
                // enough. Otherwise the list is empty or incomplete: error.
                if kind == LoadKind::ScopeTree && !pending.staged {
                    self.fall_back_to_subscriptions(&e);
                } else if kind == LoadKind::AppDetail && !pending.staged {
                    // The overview and credentials still show.
                    self.state.notice = Some(format!("Application details unavailable: {e}"));
                } else if kind == LoadKind::PrincipalGroups && self.state.view == ViewMode::MemberOf
                {
                    self.state
                        .set_error(Some(format!("Failed to load group memberships: {e}")));
                } else if kind == LoadKind::PrincipalGroups {
                    // Direct assignments still show.
                    self.state.notice = Some(format!(
                        "Group memberships unavailable (Microsoft Graph): {e}; \
                         showing direct assignments only"
                    ));
                } else if kind == LoadKind::Principals {
                    // The assignments still show the principal IDs.
                    self.state.notice = Some(format!(
                        "Principal names unavailable (Microsoft Graph): {e}"
                    ));
                } else if kind == LoadKind::CloudContexts {
                    // No gcloud (or no login): no Google Cloud contexts.
                    warn!("Google Cloud contexts unavailable: {}", e);
                } else if kind == LoadKind::RoleDefinitions
                    && self.state.view != ViewMode::RoleDefinitions
                {
                    // The assignments show without role names.
                    self.state.notice = Some(format!("Role names unavailable: {e}"));
                } else if kind == LoadKind::RoleAssignments
                    && self.state.view != ViewMode::RoleAssignments
                {
                    warn!("Role assignments unavailable: {}", e);
                } else if kind == LoadKind::PolicyExemptions
                    && self.state.view != ViewMode::ExemptionList
                {
                    warn!("Policy exemptions unavailable: {}", e);
                } else if kind == LoadKind::ComplianceRollup
                    && self.state.view != ViewMode::ComplianceRanking
                {
                    // Only the tree's compliance column is missing: users
                    // without tenant-wide access already got a notice.
                    warn!("Compliance roll-up unavailable: {}", e);
                } else if kind == LoadKind::ComplianceSummary && !pending.staged {
                    // The assignments are still useful without the totals.
                    self.state.notice = Some(format!("Compliance summary unavailable: {e}"));
                } else if kind == LoadKind::ResourcePolicy && !pending.staged {
                    self.state.notice = Some(format!("Policy states unavailable: {e}"));
                } else if kind == LoadKind::ResourceDetail && !pending.staged {
                    // The view still shows what the list knew: no popup.
                    self.state.notice = Some(format!("Details unavailable: {e}"));
                } else if pending.staged {
                    let age = self
                        .state
                        .freshness
                        .get(&view_of(kind))
                        .map(|f| format!(" from {}", crate::state::format_age(f.as_of)))
                        .unwrap_or_default();
                    self.state.notice = Some(format!(
                        "Azure refresh failed ({e}); showing cached {}{age}",
                        kind.label()
                    ));
                } else if kind == LoadKind::Subscriptions && self.state.provider == Provider::Azure
                {
                    // Not signed in to Azure, perhaps on purpose: the other
                    // clouds are one command away.
                    self.state.set_error(Some(format!(
                        "Failed to load {}: {e}\n\nOther clouds: :ctx lists your gcloud \
                         configurations and AWS profiles.",
                        kind.label()
                    )));
                } else {
                    self.state
                        .set_error(Some(format!("Failed to load {}: {}", kind.label(), e)));
                }
            }
        }
    }

    /// Put rows into their list, appending or replacing.
    fn put(&mut self, data: LoadedData, append: bool) {
        fn put<T>(list: &mut Vec<T>, items: Vec<T>, append: bool) {
            if append {
                list.extend(items);
            } else {
                *list = items;
            }
        }
        match data {
            LoadedData::Subscriptions(items) => put(&mut self.state.subscriptions, items, append),
            LoadedData::ResourceGroups(items) => {
                put(&mut self.state.resource_groups, items, append)
            }
            LoadedData::Resources(items) => put(&mut self.state.resources, items, append),
            LoadedData::ScopeTree(tree) => self.state.set_scope_tree(tree),
            LoadedData::Tenants(tenants) => self.state.tenants = tenants,
            LoadedData::CloudContexts(contexts) => {
                self.state.gcp_configurations = contexts.gcp;
                self.state.aws_profiles = contexts.aws;
            }
            LoadedData::PolicyAssignments(items) => self.state.policy_assignments = items,
            LoadedData::PolicyExemptions(items) => self.state.policy_exemptions = items,
            LoadedData::RoleAssignments(items) => self.state.role_assignments = items,
            LoadedData::RoleDefinitions(items) => self.state.role_definitions = items,
            LoadedData::Groups(groups) => self.state.groups = groups,
            LoadedData::Applications(apps, sps) => {
                self.state.applications = apps;
                self.state.service_principals = sps;
            }
            LoadedData::AppDetail(app_id, detail) => {
                if !app_id.is_empty() {
                    self.state.app_detail = Some((app_id, *detail));
                }
            }
            LoadedData::GroupMembers(id, transitive, members) => {
                if !id.is_empty() {
                    for m in &members {
                        self.state
                            .principals
                            .insert(m.id.to_ascii_lowercase(), m.clone());
                    }
                    self.state.group_members.insert((id, transitive), members);
                }
            }
            LoadedData::PrincipalGroups(id, groups) => {
                if !id.is_empty() {
                    // Their names are known now too.
                    for g in &groups {
                        self.state
                            .principals
                            .insert(g.id.to_ascii_lowercase(), g.clone());
                    }
                    self.state.principal_groups.insert(id, groups);
                }
            }
            // Names add up: each lookup asks only for the missing ones.
            LoadedData::Principals(items) => self
                .state
                .principals
                .extend(items.into_iter().map(|p| (p.id.to_ascii_lowercase(), p))),
            LoadedData::ComplianceRollup(counts) => self.state.compliance_rollup = Some(counts),
            LoadedData::PolicyCompliance(scope, counts) => {
                self.state.policy_compliance = Some((scope, counts))
            }
            LoadedData::ComplianceSummary(scope, counts) => {
                self.state.compliance_summary = Some((scope, counts))
            }
            LoadedData::ResourceDetail(doc) => {
                self.state.resource_document = Some(doc).filter(|d| !d.is_null())
            }
            LoadedData::PolicyStates(items) => self.state.policy_states = items,
            LoadedData::ResourcePolicy(id, states) => {
                if !id.is_empty() {
                    self.state.resource_policy = Some((id, states))
                }
            }
        }
    }

    /// Replace a list with fresh rows, keeping the selection on the same item
    /// when the list is on screen.
    fn swap_in(&mut self, data: LoadedData) {
        let selected = self.state.selected_id();
        self.put(data, false);
        if let Some(id) = selected {
            self.state.select_id(&id);
        }
    }

    /// Replace the cache for the request's scope with the list now in state.
    /// Subscriptions that failed to load keep their cached rows. The write
    /// runs on the cache's background thread.
    fn write_scope(&self, request: &LoadRequest, failed: &[String]) {
        let scope = |ids: &[String]| -> Vec<String> {
            ids.iter()
                .filter(|id| !failed.iter().any(|f| f.eq_ignore_ascii_case(id)))
                .cloned()
                .collect()
        };
        let _t = timing::timer("cache.queue_write");
        let job = match request {
            // Tenant names are small and refreshed on each :ctx; policy
            // states change often and are always loaded fresh.
            LoadRequest::Tenants
            | LoadRequest::CloudContexts
            | LoadRequest::PrincipalGroups { .. }
            | LoadRequest::GroupMembers { .. }
            | LoadRequest::PolicyStates { .. }
            | LoadRequest::ResourcePolicy { .. } => return,
            LoadRequest::AppDetail { .. } => return,
            LoadRequest::Applications => match serde_json::to_string(&(
                &self.state.applications,
                &self.state.service_principals,
            )) {
                Ok(value) => WriteJob::Document {
                    key: self.tenant_key("applications"),
                    value,
                },
                Err(_) => return,
            },
            LoadRequest::Groups => match serde_json::to_string(&self.state.groups) {
                Ok(value) => WriteJob::Document {
                    key: self.tenant_key("groups"),
                    value,
                },
                Err(_) => return,
            },
            LoadRequest::Principals { .. } => {
                let all: Vec<&c9s_azure::directory::Principal> =
                    self.state.principals.values().collect();
                match serde_json::to_string(&all) {
                    Ok(value) => WriteJob::Document {
                        key: self.tenant_key("principals"),
                        value,
                    },
                    Err(_) => return,
                }
            }
            LoadRequest::RoleAssignments { .. } => {
                match serde_json::to_string(&self.state.role_assignments) {
                    Ok(value) => WriteJob::Document {
                        key: self.tenant_key("role_assignments"),
                        value,
                    },
                    Err(_) => return,
                }
            }
            LoadRequest::RoleDefinitions { .. } => {
                match serde_json::to_string(&self.state.role_definitions) {
                    Ok(value) => WriteJob::Document {
                        key: self.tenant_key("role_definitions"),
                        value,
                    },
                    Err(_) => return,
                }
            }
            LoadRequest::PolicyExemptions => {
                match serde_json::to_string(&self.state.policy_exemptions) {
                    Ok(value) => WriteJob::Document {
                        key: self.exemptions_key(),
                        value,
                    },
                    Err(_) => return,
                }
            }
            LoadRequest::ComplianceRollup => match &self.state.compliance_rollup {
                Some(counts) => match serde_json::to_string(counts) {
                    Ok(value) => WriteJob::Document {
                        key: self.rollup_key(),
                        value,
                    },
                    Err(_) => return,
                },
                None => return,
            },
            LoadRequest::PolicyAssignments => {
                match serde_json::to_string(&self.state.policy_assignments) {
                    Ok(value) => WriteJob::Document {
                        key: self.policy_key(),
                        value,
                    },
                    Err(_) => return,
                }
            }
            LoadRequest::PolicyCompliance { scope } => match &self.state.policy_compliance {
                Some((s, counts)) if s == scope => match serde_json::to_string(counts) {
                    Ok(value) => WriteJob::Document {
                        key: format!("policy_compliance:{}", scope.arm_id()),
                        value,
                    },
                    Err(_) => return,
                },
                _ => return,
            },
            LoadRequest::ComplianceSummary { scope } => match &self.state.compliance_summary {
                Some((s, counts)) if s == scope => match serde_json::to_string(counts) {
                    Ok(value) => WriteJob::Document {
                        key: format!("compliance_summary:{}", scope.arm_id()),
                        value,
                    },
                    Err(_) => return,
                },
                _ => return,
            },
            // The subscriptions cache is Azure's: another cloud's accounts
            // must not show at the next start.
            LoadRequest::Subscriptions if self.state.provider != Provider::Azure => return,
            LoadRequest::Subscriptions => WriteJob::Subscriptions(self.state.subscriptions.clone()),
            LoadRequest::ResourceGroups { subscription_ids } => WriteJob::ResourceGroups {
                subscription_ids: scope(subscription_ids),
                items: self.state.resource_groups.clone(),
            },
            LoadRequest::Resources {
                subscription_ids,
                resource_group,
            } => WriteJob::Resources {
                subscription_ids: scope(subscription_ids),
                resource_group: resource_group.clone(),
                items: self.state.resources.clone(),
            },
            LoadRequest::ResourceDetail { resource_id, .. } => {
                match &self.state.resource_document {
                    Some(doc) => WriteJob::ResourceDetail {
                        resource_id: resource_id.clone(),
                        document: doc.clone(),
                    },
                    None => return,
                }
            }
            LoadRequest::ScopeTree => match &self.state.scope_tree {
                Some(tree) => match serde_json::to_string(tree) {
                    Ok(value) => WriteJob::Document {
                        key: self.scope_tree_key(),
                        value,
                    },
                    Err(e) => {
                        warn!("Could not serialize the tree: {}", e);
                        return;
                    }
                },
                None => return,
            },
        };
        self.cache.write(job);
    }

    /// Get the application state
    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Get mutable application state
    pub fn state_mut(&mut self) -> &mut AppState {
        &mut self.state
    }

    /// Get the cache
    pub fn cache(&self) -> &Cache {
        &self.cache
    }

    /// Determine the cache directory (platform-specific local data dir)
    fn cache_dir() -> anyhow::Result<PathBuf> {
        let dirs = crate::paths::project_dirs().context("Could not determine data directory")?;
        Ok(dirs.data_local_dir().to_path_buf())
    }
}

/// The list view that shows a kind of data.
fn view_of(kind: LoadKind) -> ViewMode {
    match kind {
        LoadKind::Subscriptions => ViewMode::SubscriptionList,
        LoadKind::ResourceGroups => ViewMode::ResourceGroupList,
        LoadKind::Resources => ViewMode::ResourceList,
        LoadKind::ScopeTree => ViewMode::ManagementGroupTree,
        LoadKind::ResourceDetail | LoadKind::ResourcePolicy => ViewMode::ResourceDetail,
        LoadKind::PolicyStates => ViewMode::PolicyNonCompliant,
        LoadKind::Tenants | LoadKind::CloudContexts => ViewMode::TenantList,
        LoadKind::PolicyAssignments | LoadKind::PolicyCompliance | LoadKind::ComplianceSummary => {
            ViewMode::PolicyAssignments
        }
        LoadKind::ComplianceRollup => ViewMode::ComplianceRanking,
        LoadKind::PolicyExemptions => ViewMode::ExemptionList,
        LoadKind::RoleAssignments => ViewMode::RoleAssignments,
        LoadKind::RoleDefinitions => ViewMode::RoleDefinitions,
        LoadKind::Principals | LoadKind::PrincipalGroups => ViewMode::RoleAssignments,
        LoadKind::Groups => ViewMode::GroupList,
        LoadKind::Applications => ViewMode::AppList,
        LoadKind::AppDetail => ViewMode::AppDetail,
        LoadKind::GroupMembers => ViewMode::GroupMembers,
    }
}

/// Append `data` to the accumulated chunks.
fn merge(acc: &mut Option<LoadedData>, data: LoadedData) {
    match (acc.as_mut(), data) {
        (None, data) => *acc = Some(data),
        (Some(LoadedData::Subscriptions(a)), LoadedData::Subscriptions(b)) => a.extend(b),
        (Some(LoadedData::ResourceGroups(a)), LoadedData::ResourceGroups(b)) => a.extend(b),
        (Some(LoadedData::Resources(a)), LoadedData::Resources(b)) => a.extend(b),
        // A tree arrives whole; the newest replaces the rest.
        (Some(_), other) => *acc = Some(other),
    }
}

/// Whether `program` is on the PATH (`.exe` or `.cmd` on Windows).
fn on_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let names: Vec<String> = if cfg!(windows) {
        ["exe", "cmd", "bat"]
            .iter()
            .map(|ext| format!("{program}.{ext}"))
            .collect()
    } else {
        vec![program.to_string()]
    };
    std::env::split_paths(&path).any(|dir| names.iter().any(|n| dir.join(n).is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::{ChunkSink, FetchOutcome};
    use crate::models::{ResourceGroup, Subscription};
    use futures::future::BoxFuture;
    use futures::FutureExt;
    use std::path::Path;
    use std::sync::Mutex;

    /// Answers every request with a scripted outcome: `chunks` are streamed
    /// through the sink, then `last` is the final result.
    struct Script {
        chunks: Vec<LoadedData>,
        last: Result<LoadedData, String>,
        failed: Vec<String>,
    }

    struct Fake(Mutex<Vec<Script>>);

    impl Fake {
        fn new(scripts: Vec<Script>) -> Self {
            Self(Mutex::new(scripts))
        }
    }

    impl Fetcher for Fake {
        fn fetch(
            &self,
            _request: LoadRequest,
            sink: ChunkSink,
        ) -> BoxFuture<'static, FetchOutcome> {
            let script = self.0.lock().unwrap().remove(0);
            async move {
                for chunk in script.chunks {
                    sink.send(chunk);
                }
                FetchOutcome {
                    data: script.last,
                    failed_subscriptions: script.failed,
                    notice: None,
                }
            }
            .boxed()
        }
    }

    fn done(last: Result<LoadedData, String>) -> Script {
        Script {
            chunks: vec![],
            last,
            failed: vec![],
        }
    }

    fn sub(name: &str) -> Subscription {
        Subscription {
            id: format!("id-{name}"),
            name: name.into(),
            state: "Enabled".into(),
            tenant_id: "t".into(),
        }
    }

    fn rg(sub: &str, name: &str) -> ResourceGroup {
        ResourceGroup {
            id: format!("/subscriptions/{sub}/resourceGroups/{name}"),
            name: name.into(),
            location: "westeurope".into(),
            subscription_id: sub.into(),
            tags: Default::default(),
        }
    }

    fn rgs(items: Vec<ResourceGroup>) -> LoadedData {
        LoadedData::ResourceGroups(items)
    }

    fn names(app: &App) -> Vec<String> {
        app.state()
            .resource_groups
            .iter()
            .map(|r| r.name.clone())
            .collect()
    }

    /// App over a cache that already holds subscriptions `id-a`, `id-b` and
    /// these resource groups.
    fn app_with_cache(scripts: Vec<Script>, cached_rgs: &[ResourceGroup]) -> App {
        let cache = Cache::new(Path::new(":memory:")).unwrap();
        let repo = cache.repository();
        repo.replace_subscriptions(&[sub("a"), sub("b")]).unwrap();
        repo.replace_resource_groups(&["id-a".into(), "id-b".into()], cached_rgs)
            .unwrap();
        App::with_fetcher(cache, Some(Arc::new(Fake::new(scripts))))
    }

    fn rg_request(subs: &[&str]) -> LoadRequest {
        LoadRequest::ResourceGroups {
            subscription_ids: subs.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Request, then apply results until the load finishes.
    async fn load(app: &mut App, rx: &mut LoadReceiver, request: LoadRequest) {
        app.request(request);
        while app.state().loading {
            let result = rx.recv().await.unwrap();
            app.apply(result);
        }
    }

    #[tokio::test]
    async fn successful_load_updates_state_and_cache() {
        let mut app = app_with_cache(
            vec![done(Ok(LoadedData::Subscriptions(vec![
                sub("a"),
                sub("c"),
            ])))],
            &[],
        );
        let mut rx = app.take_load_receiver().unwrap();
        load(&mut app, &mut rx, LoadRequest::Subscriptions).await;

        assert_eq!(app.state().subscriptions.len(), 2);
        assert_eq!(app.state().data_source, DataSource::Azure);
        let cached = app.cache().repository().get_subscriptions().unwrap();
        let ids: Vec<_> = cached.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            ["id-a", "id-c"],
            "removed subscription dropped from cache"
        );
    }

    #[tokio::test]
    async fn cached_rows_show_immediately_then_azure_replaces_them() {
        let mut app = app_with_cache(
            vec![done(Ok(rgs(vec![rg("id-a", "rg-new")])))],
            &[rg("id-a", "rg-old"), rg("id-b", "rg-b")],
        );
        let mut rx = app.take_load_receiver().unwrap();

        app.request(rg_request(&["id-a"]));
        // Before Azure answers: cached rows for exactly this scope.
        assert_eq!(names(&app), ["rg-old"]);
        let fresh = app.state().freshness[&ViewMode::ResourceGroupList];
        assert_eq!(fresh.source, DataSource::Cache);

        app.apply(rx.recv().await.unwrap());
        assert_eq!(names(&app), ["rg-new"]);
        assert_eq!(
            app.state().freshness[&ViewMode::ResourceGroupList].source,
            DataSource::Azure
        );
        // Cache scope replaced: "rg-old" (deleted in Azure) is gone, other
        // subscriptions untouched.
        let repo = app.cache().repository();
        let a = repo.cached_resource_groups(&["id-a".into()]).unwrap().items;
        assert_eq!(
            a.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["rg-new"]
        );
        assert_eq!(
            repo.cached_resource_groups(&["id-b".into()])
                .unwrap()
                .items
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn with_cached_rows_on_screen_pages_are_staged_and_swapped_once() {
        let mut app = app_with_cache(
            vec![Script {
                chunks: vec![rgs(vec![rg("id-a", "rg-1")]), rgs(vec![rg("id-a", "rg-2")])],
                last: Ok(rgs(vec![rg("id-a", "rg-3")])),
                failed: vec![],
            }],
            &[rg("id-a", "rg-1"), rg("id-a", "rg-2"), rg("id-a", "rg-3")],
        );
        app.state_mut().view = ViewMode::ResourceGroupList;
        let mut rx = app.take_load_receiver().unwrap();

        app.request(rg_request(&["id-a"]));
        app.state_mut().list_state.selected = 2; // user is on rg-3

        app.apply(rx.recv().await.unwrap()); // first page
        assert_eq!(
            names(&app),
            ["rg-1", "rg-2", "rg-3"],
            "no shrink to one page"
        );
        app.apply(rx.recv().await.unwrap()); // second page
        app.apply(rx.recv().await.unwrap()); // final
        assert_eq!(names(&app), ["rg-1", "rg-2", "rg-3"]);
        assert_eq!(
            app.state().list_state.selected,
            2,
            "selection stays on rg-3"
        );
    }

    #[tokio::test]
    async fn swap_keeps_selection_on_the_same_item_when_rows_move() {
        let mut app = app_with_cache(
            vec![done(Ok(rgs(vec![
                rg("id-a", "rg-0"),
                rg("id-a", "rg-1"),
                rg("id-a", "rg-2"),
            ])))],
            &[rg("id-a", "rg-1"), rg("id-a", "rg-2")],
        );
        app.state_mut().view = ViewMode::ResourceGroupList;
        let mut rx = app.take_load_receiver().unwrap();
        app.request(rg_request(&["id-a"]));
        app.state_mut().list_state.selected = 1; // rg-2
        app.apply(rx.recv().await.unwrap());
        assert_eq!(app.state().selected_id().unwrap(), rg("id-a", "rg-2").id);
    }

    #[tokio::test]
    async fn without_cache_pages_stream_into_view() {
        let mut app = app_with_cache(
            vec![Script {
                chunks: vec![rgs(vec![rg("id-a", "rg-1")])],
                last: Ok(rgs(vec![rg("id-a", "rg-2")])),
                failed: vec![],
            }],
            &[],
        );
        let mut rx = app.take_load_receiver().unwrap();
        app.request(rg_request(&["id-a"]));
        assert!(names(&app).is_empty());
        app.apply(rx.recv().await.unwrap());
        assert_eq!(names(&app), ["rg-1"], "first page visible right away");
        app.apply(rx.recv().await.unwrap());
        assert_eq!(names(&app), ["rg-1", "rg-2"]);
    }

    #[tokio::test]
    async fn failed_subscriptions_keep_their_cached_rows() {
        let mut app = app_with_cache(
            vec![Script {
                chunks: vec![],
                last: Ok(rgs(vec![rg("id-a", "rg-a2")])),
                failed: vec!["ID-B".into()],
            }],
            &[rg("id-a", "rg-a1"), rg("id-b", "rg-b1")],
        );
        let mut rx = app.take_load_receiver().unwrap();
        load(&mut app, &mut rx, rg_request(&["id-a", "id-b"])).await;

        let repo = app.cache().repository();
        let b = repo.cached_resource_groups(&["id-b".into()]).unwrap().items;
        assert_eq!(b.len(), 1, "a failed subscription's cache is not wiped");
        assert!(app
            .state()
            .notice
            .as_deref()
            .unwrap()
            .contains("1 subscription"));
    }

    #[tokio::test]
    async fn failure_with_cached_rows_is_a_notice_not_an_error() {
        let mut app = app_with_cache(
            vec![done(Err("token expired".into()))],
            &[rg("id-a", "rg-1")],
        );
        let mut rx = app.take_load_receiver().unwrap();
        load(&mut app, &mut rx, rg_request(&["id-a"])).await;

        assert_eq!(names(&app), ["rg-1"]);
        assert!(app.state().error.is_none());
        let notice = app.state().notice.clone().unwrap();
        assert!(
            notice.contains("token expired") && notice.contains("cached"),
            "{notice}"
        );
    }

    #[tokio::test]
    async fn failure_without_cached_rows_is_an_error() {
        let mut app = app_with_cache(vec![done(Err("boom".into()))], &[]);
        let mut rx = app.take_load_receiver().unwrap();
        load(&mut app, &mut rx, rg_request(&["id-a"])).await;
        assert!(app.state().error.as_deref().unwrap().contains("boom"));
    }

    #[test]
    fn offline_shows_cached_rows_with_a_notice() {
        let cache = Cache::new(Path::new(":memory:")).unwrap();
        cache
            .repository()
            .replace_subscriptions(&[sub("a")])
            .unwrap();
        cache
            .repository()
            .replace_resource_groups(&["id-a".into()], &[rg("id-a", "rg-1")])
            .unwrap();
        let mut app = App::offline(cache);
        app.request(rg_request(&["id-a"]));
        assert_eq!(names(&app), ["rg-1"]);
        assert!(app.state().error.is_none());
        assert!(app.state().notice.as_deref().unwrap().contains("cached"));
    }

    #[test]
    fn offline_without_cache_shows_error() {
        let mut app = App::offline(Cache::new(Path::new(":memory:")).unwrap());
        app.request(LoadRequest::Subscriptions);
        assert!(!app.state().loading);
        assert!(app.state().error.as_deref().unwrap().contains("az login"));
    }
}
