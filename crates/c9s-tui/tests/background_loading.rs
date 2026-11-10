//! The UI must never wait for Azure. These tests drive `TuiApp` with key
//! events against a deliberately slow fake fetcher.

use c9s_core::cache::Cache;
use c9s_core::loader::{ChunkSink, FetchOutcome, Fetcher, LoadRequest, LoadedData};
use c9s_core::models::{Resource, ResourceGroup, Subscription};
use c9s_core::scope_tree::{GroupNode, ScopeTree, SubscriptionNode};
use c9s_core::state::ViewMode;
use c9s_core::App;
use c9s_tui::{Event, TuiApp};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures::future::BoxFuture;
use futures::FutureExt;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const AZURE_LATENCY: Duration = Duration::from_millis(300);

/// Answers every request after `AZURE_LATENCY` and records what was asked.
#[derive(Default)]
struct SlowAzure {
    requests: Mutex<Vec<LoadRequest>>,
}

impl Fetcher for SlowAzure {
    fn fetch(&self, request: LoadRequest, _sink: ChunkSink) -> BoxFuture<'static, FetchOutcome> {
        self.requests.lock().unwrap().push(request.clone());
        async move {
            tokio::time::sleep(AZURE_LATENCY).await;
            let data = match request {
                LoadRequest::Subscriptions => LoadedData::Subscriptions(vec![sub("prod")]),
                LoadRequest::ResourceGroups { subscription_ids } => LoadedData::ResourceGroups(
                    subscription_ids
                        .iter()
                        .map(|s| ResourceGroup {
                            id: format!("{s}/rg-app"),
                            name: "rg-app".into(),
                            location: "westeurope".into(),
                            subscription_id: s.clone(),
                            tags: HashMap::new(),
                        })
                        .collect(),
                ),
                LoadRequest::Resources {
                    subscription_ids,
                    resource_group,
                } => LoadedData::Resources(
                    subscription_ids
                        .iter()
                        .map(|s| Resource {
                            id: format!("{s}/vm-1"),
                            name: "vm-1".into(),
                            resource_type: "Microsoft.Compute/virtualMachines".into(),
                            location: "westeurope".into(),
                            resource_group: resource_group.clone().unwrap_or_default(),
                            subscription_id: s.clone(),
                            tags: HashMap::new(),
                        })
                        .collect(),
                ),
                LoadRequest::ScopeTree => LoadedData::ScopeTree(tree()),
                LoadRequest::Tenants => LoadedData::Tenants(vec![]),
                LoadRequest::PolicyAssignments => LoadedData::PolicyAssignments(vec![]),
                LoadRequest::RoleAssignments { .. } => LoadedData::RoleAssignments(vec![]),
                LoadRequest::Principals { .. } => LoadedData::Principals(vec![]),
                LoadRequest::Groups => LoadedData::Groups(vec![]),
                LoadRequest::CloudContexts => LoadedData::CloudContexts(Default::default()),
                LoadRequest::Applications => LoadedData::Applications(vec![], vec![]),
                LoadRequest::AppDetail { app_id, .. } => {
                    LoadedData::AppDetail(app_id, Box::default())
                }
                LoadRequest::GroupMembers {
                    group_id,
                    transitive,
                } => LoadedData::GroupMembers(group_id, transitive, vec![]),
                LoadRequest::PrincipalGroups { principal_id } => {
                    LoadedData::PrincipalGroups(principal_id, vec![])
                }
                LoadRequest::RoleDefinitions { .. } => LoadedData::RoleDefinitions(vec![]),
                LoadRequest::PolicyExemptions => LoadedData::PolicyExemptions(vec![]),
                LoadRequest::ComplianceRollup => LoadedData::ComplianceRollup(Default::default()),
                LoadRequest::ComplianceSummary { scope } => {
                    LoadedData::ComplianceSummary(scope, Default::default())
                }
                LoadRequest::PolicyStates { .. } => LoadedData::PolicyStates(vec![]),
                LoadRequest::ResourcePolicy { resource_id, .. } => {
                    LoadedData::ResourcePolicy(resource_id, vec![])
                }
                LoadRequest::PolicyCompliance { scope } => {
                    LoadedData::PolicyCompliance(scope, Default::default())
                }
                LoadRequest::ResourceDetail { .. } => {
                    LoadedData::ResourceDetail(serde_json::json!({}))
                }
                LoadRequest::Cost { query } => {
                    LoadedData::Cost(Box::new(c9s_core::cost::CostReport {
                        query,
                        currency: "EUR".into(),
                        entries: vec![],
                    }))
                }
            };
            FetchOutcome {
                data: Ok(data),
                failed_subscriptions: vec![],
                notice: None,
            }
        }
        .boxed()
    }
}

/// Tenant Root Group
/// ├── Platform: prod
/// └── Workloads: dev
fn tree() -> ScopeTree {
    let mg = |name: &str, parent: &str| GroupNode {
        name: name.into(),
        display_name: name.to_uppercase(),
        parent: parent.into(),
    };
    let sub = |id: &str, parent: &str| SubscriptionNode {
        id: id.into(),
        display_name: id.into(),
        parent: parent.into(),
    };
    ScopeTree {
        root: "tenant".into(),
        root_display_name: "Tenant Root Group".into(),
        groups: vec![mg("platform", "tenant"), mg("workloads", "tenant")],
        subscriptions: vec![sub("id-prod", "platform"), sub("id-dev", "workloads")],
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

fn setup() -> (TuiApp, Arc<SlowAzure>) {
    let azure = Arc::new(SlowAzure::default());
    let cache = Cache::new(Path::new(":memory:")).unwrap();
    let mut core = App::with_fetcher(cache, Some(azure.clone()));
    core.state_mut().subscriptions = vec![sub("prod"), sub("dev")];
    (TuiApp::new(core), azure)
}

/// What the fake was last asked for, once spawned load tasks have started.
async fn last_request(azure: &SlowAzure) -> LoadRequest {
    tokio::time::sleep(Duration::from_millis(20)).await;
    azure.requests.lock().unwrap().last().cloned().unwrap()
}

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

async fn press(app: &mut TuiApp, code: KeyCode) -> Duration {
    let started = Instant::now();
    app.handle_event(key(code)).await.unwrap();
    started.elapsed()
}

async fn type_command(app: &mut TuiApp, command: &str) {
    press(app, KeyCode::Char(':')).await;
    for c in command.chars() {
        press(app, KeyCode::Char(c)).await;
    }
    press(app, KeyCode::Enter).await;
}

#[tokio::test]
async fn opening_a_subscription_switches_view_immediately_and_loads_in_background() {
    let (mut app, _azure) = setup();
    let mut loads = app.take_load_receiver().unwrap();

    let took = press(&mut app, KeyCode::Enter).await;
    assert!(took < AZURE_LATENCY / 3, "Enter blocked for {took:?}");
    assert_eq!(app.state().view, ViewMode::ResourceGroupList);
    assert!(app.state().loading);

    // The user can keep navigating while the load runs.
    let took = press(&mut app, KeyCode::Char('j')).await;
    assert!(took < AZURE_LATENCY / 3);

    app.apply_load(loads.recv().await.unwrap());
    assert!(!app.state().loading);
    assert_eq!(app.state().resource_groups.len(), 1);
}

#[tokio::test]
async fn opening_a_resource_group_requests_only_that_group() {
    let (mut app, azure) = setup();
    let mut loads = app.take_load_receiver().unwrap();

    press(&mut app, KeyCode::Enter).await; // subscription -> RG list
    app.apply_load(loads.recv().await.unwrap());
    press(&mut app, KeyCode::Enter).await; // RG -> resources
    assert_eq!(app.state().view, ViewMode::ResourceList);

    let last = last_request(&azure).await;
    assert_eq!(
        last,
        LoadRequest::Resources {
            subscription_ids: vec!["id-prod".into()],
            resource_group: Some("rg-app".into()),
        }
    );
    app.apply_load(loads.recv().await.unwrap());
    assert!(app
        .state()
        .resources
        .iter()
        .all(|r| r.resource_group == "rg-app"));
}

#[tokio::test]
async fn going_back_while_loading_then_opening_another_subscription_discards_stale_result() {
    let (mut app, _azure) = setup();
    let mut loads = app.take_load_receiver().unwrap();

    press(&mut app, KeyCode::Enter).await; // open "prod"
    press(&mut app, KeyCode::Esc).await; // back before it loads
    press(&mut app, KeyCode::Char('j')).await; // select "dev"
    press(&mut app, KeyCode::Enter).await; // open "dev"

    // Only the latest request's result is applied.
    app.apply_load(loads.recv().await.unwrap());
    let rgs = &app.state().resource_groups;
    assert_eq!(rgs.len(), 1);
    assert_eq!(rgs[0].subscription_id, "id-dev");
}

#[tokio::test]
async fn all_subscriptions_view_skips_disabled_and_survives_back_navigation() {
    let (mut app, azure) = setup();
    app.state_mut().subscriptions.push(Subscription {
        state: "Disabled".into(),
        ..sub("old")
    });
    let mut loads = app.take_load_receiver().unwrap();

    type_command(&mut app, "rg all").await;
    assert!(app.state().cross_subscription_mode);
    let last = last_request(&azure).await;
    assert_eq!(
        last,
        LoadRequest::ResourceGroups {
            subscription_ids: vec!["id-prod".into(), "id-dev".into()],
        }
    );
    app.apply_load(loads.recv().await.unwrap());

    press(&mut app, KeyCode::Enter).await; // into a group
    press(&mut app, KeyCode::Esc).await; // back to the all-subscriptions list
    assert_eq!(app.state().view, ViewMode::ResourceGroupList);
    assert!(app.state().cross_subscription_mode, "scope lost on back");
}

#[tokio::test]
async fn reopening_a_subscription_shows_cached_groups_before_azure_answers() {
    let (mut app, _azure) = setup();
    // The cache knows the subscriptions (resource groups reference them).
    let subs = app.state().subscriptions.clone();
    app.core()
        .cache()
        .repository()
        .replace_subscriptions(&subs)
        .unwrap();
    let mut loads = app.take_load_receiver().unwrap();

    // First visit: nothing cached, wait for Azure.
    press(&mut app, KeyCode::Enter).await;
    assert!(app.state().resource_groups.is_empty());
    app.apply_load(loads.recv().await.unwrap());
    assert_eq!(app.state().resource_groups.len(), 1);

    // Back, and open the same subscription again.
    press(&mut app, KeyCode::Esc).await;
    let started = Instant::now();
    press(&mut app, KeyCode::Enter).await;
    assert!(started.elapsed() < AZURE_LATENCY / 3);

    // Rows are on screen immediately, from the cache, while Azure loads.
    assert!(app.state().loading);
    assert_eq!(app.state().resource_groups.len(), 1);
    let fresh = app.state().current_freshness().unwrap();
    assert_eq!(fresh.source, c9s_core::state::DataSource::Cache);

    app.apply_load(loads.recv().await.unwrap());
    let fresh = app.state().current_freshness().unwrap();
    assert_eq!(fresh.source, c9s_core::state::DataSource::Azure);
}

/// Labels of the visible tree rows, indented by depth.
fn tree_labels(app: &TuiApp) -> Vec<String> {
    app.state()
        .tree_rows()
        .iter()
        .map(|r| format!("{}{}", "  ".repeat(r.depth), r.label))
        .collect()
}

#[tokio::test]
async fn management_group_tree_expands_and_opens_a_group_scoped_to_its_subscriptions() {
    let (mut app, azure) = setup();
    let mut loads = app.take_load_receiver().unwrap();

    type_command(&mut app, "mg").await;
    assert_eq!(app.state().view, ViewMode::ManagementGroupTree);
    app.apply_load(loads.recv().await.unwrap());
    assert_eq!(
        tree_labels(&app),
        ["Tenant Root Group", "  PLATFORM", "  WORKLOADS"]
    );

    // Open "Platform" (Space), then close it again with h.
    press(&mut app, KeyCode::Char('j')).await;
    press(&mut app, KeyCode::Char(' ')).await;
    assert_eq!(
        tree_labels(&app),
        [
            "Tenant Root Group",
            "  PLATFORM",
            "    id-prod",
            "  WORKLOADS"
        ]
    );
    press(&mut app, KeyCode::Char('h')).await;
    assert_eq!(tree_labels(&app).len(), 3);
    // h on a closed group moves to its parent.
    press(&mut app, KeyCode::Char('h')).await;
    assert_eq!(app.state().list_state.selected, 0);

    // Enter on "Workloads": resource groups of its subscriptions only.
    press(&mut app, KeyCode::Char('G')).await;
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(app.state().view, ViewMode::ResourceGroupList);
    assert_eq!(
        last_request(&azure).await,
        LoadRequest::ResourceGroups {
            subscription_ids: vec!["id-dev".into()],
        }
    );
    app.apply_load(loads.recv().await.unwrap());

    // Refresh keeps the management group scope.
    press(&mut app, KeyCode::Char('r')).await;
    assert_eq!(
        last_request(&azure).await,
        LoadRequest::ResourceGroups {
            subscription_ids: vec!["id-dev".into()],
        }
    );
    app.apply_load(loads.recv().await.unwrap());

    // Back to the tree, with "Workloads" still selected.
    press(&mut app, KeyCode::Esc).await;
    assert_eq!(app.state().view, ViewMode::ManagementGroupTree);
    assert_eq!(app.state().selected_id().as_deref(), Some("workloads"));
}

#[tokio::test]
async fn enter_on_a_subscription_in_the_tree_opens_its_resource_groups() {
    let (mut app, azure) = setup();
    let mut loads = app.take_load_receiver().unwrap();
    type_command(&mut app, "mg").await;
    app.apply_load(loads.recv().await.unwrap());

    press(&mut app, KeyCode::Char('j')).await; // Platform
    press(&mut app, KeyCode::Char('l')).await; // open
    press(&mut app, KeyCode::Char('j')).await; // prod
    press(&mut app, KeyCode::Enter).await;

    assert_eq!(app.state().view, ViewMode::ResourceGroupList);
    assert!(!app.state().cross_subscription_mode);
    assert_eq!(
        app.state().selected_subscription.as_ref().unwrap().name,
        "prod"
    );
    assert_eq!(
        last_request(&azure).await,
        LoadRequest::ResourceGroups {
            subscription_ids: vec!["id-prod".into()],
        }
    );
}

#[tokio::test]
async fn filtering_the_tree_shows_matches_with_their_groups() {
    let (mut app, _azure) = setup();
    let mut loads = app.take_load_receiver().unwrap();
    type_command(&mut app, "mg").await;
    app.apply_load(loads.recv().await.unwrap());

    press(&mut app, KeyCode::Char('/')).await;
    for c in "prod".chars() {
        press(&mut app, KeyCode::Char(c)).await;
    }
    assert_eq!(
        tree_labels(&app),
        ["Tenant Root Group", "  PLATFORM", "    id-prod"]
    );
}

fn rg_request(ids: &[&str]) -> LoadRequest {
    LoadRequest::ResourceGroups {
        subscription_ids: ids.iter().map(|s| s.to_string()).collect(),
    }
}

#[tokio::test]
async fn rg_without_scope_uses_the_highlighted_subscription() {
    let (mut app, azure) = setup();
    press(&mut app, KeyCode::Char('j')).await; // highlight "dev"
    type_command(&mut app, "rg").await;
    assert_eq!(last_request(&azure).await, rg_request(&["id-dev"]));
    assert!(!app.state().cross_subscription_mode);
    assert_eq!(
        app.state().selected_subscription.as_ref().unwrap().name,
        "dev"
    );
}

#[tokio::test]
async fn res_without_scope_stays_in_the_subscription_you_are_in() {
    let (mut app, azure) = setup();
    let mut loads = app.take_load_receiver().unwrap();
    press(&mut app, KeyCode::Enter).await; // into "prod"
    app.apply_load(loads.recv().await.unwrap());

    type_command(&mut app, "res").await;
    assert_eq!(
        last_request(&azure).await,
        LoadRequest::Resources {
            subscription_ids: vec!["id-prod".into()],
            resource_group: None,
        }
    );
    assert!(!app.state().cross_subscription_mode);
}

#[tokio::test]
async fn rg_without_scope_in_the_tree_uses_the_highlighted_group() {
    let (mut app, azure) = setup();
    let mut loads = app.take_load_receiver().unwrap();
    type_command(&mut app, "mg").await;
    app.apply_load(loads.recv().await.unwrap());

    press(&mut app, KeyCode::Char('G')).await; // "Workloads"
    type_command(&mut app, "rg").await;
    assert_eq!(last_request(&azure).await, rg_request(&["id-dev"]));
    assert!(app.state().cross_subscription_mode);
    assert_eq!(app.state().scope_group.as_deref(), Some("workloads"));
    app.apply_load(loads.recv().await.unwrap());

    // From that group's list, :res keeps the group.
    type_command(&mut app, "res").await;
    assert_eq!(
        last_request(&azure).await,
        LoadRequest::Resources {
            subscription_ids: vec!["id-dev".into()],
            resource_group: None,
        }
    );
}

#[tokio::test]
async fn all_still_means_all_and_carries_over_to_the_next_command() {
    let (mut app, azure) = setup();
    let mut loads = app.take_load_receiver().unwrap();
    press(&mut app, KeyCode::Char('j')).await; // highlight "dev"
    type_command(&mut app, "rg all").await;
    assert_eq!(
        last_request(&azure).await,
        rg_request(&["id-prod", "id-dev"])
    );
    app.apply_load(loads.recv().await.unwrap());

    // In an all-subscriptions list, :res means all subscriptions too.
    type_command(&mut app, "res").await;
    assert_eq!(
        last_request(&azure).await,
        LoadRequest::Resources {
            subscription_ids: vec!["id-prod".into(), "id-dev".into()],
            resource_group: None,
        }
    );
}

#[tokio::test]
async fn default_subscription_from_the_config_applies_without_context() {
    let (mut app, azure) = setup();
    {
        let state = app.state_mut();
        state.config.default_subscription = Some("dev".into());
        state.view = ViewMode::ResourceList; // no highlighted subscription
    }
    type_command(&mut app, "rg").await;
    assert_eq!(last_request(&azure).await, rg_request(&["id-dev"]));
    // `all` still means all.
    type_command(&mut app, "rg all").await;
    assert_eq!(
        last_request(&azure).await,
        rg_request(&["id-prod", "id-dev"])
    );
}

#[tokio::test]
async fn auto_refresh_reloads_the_view_when_idle_and_due() {
    let (mut app, azure) = setup();
    let mut loads = app.take_load_receiver().unwrap();
    press(&mut app, KeyCode::Enter).await; // prod's resource groups
    app.apply_load(loads.recv().await.unwrap());
    let before = azure.requests.lock().unwrap().len();
    let now = std::time::Instant::now();

    // Off by default.
    app.tick_at(now + Duration::from_secs(3600)).await.unwrap();
    assert_eq!(azure.requests.lock().unwrap().len(), before);

    app.state_mut().config.refresh_interval = 60;
    app.tick_at(now + Duration::from_secs(30)).await.unwrap();
    assert_eq!(azure.requests.lock().unwrap().len(), before, "not due yet");

    // Not while typing a command.
    press(&mut app, KeyCode::Char(':')).await;
    app.tick_at(now + Duration::from_secs(61)).await.unwrap();
    assert_eq!(azure.requests.lock().unwrap().len(), before, "busy");
    press(&mut app, KeyCode::Esc).await;

    app.tick_at(now + Duration::from_secs(61)).await.unwrap();
    assert_eq!(last_request(&azure).await, rg_request(&["id-prod"]));
    assert_eq!(azure.requests.lock().unwrap().len(), before + 1);
}
