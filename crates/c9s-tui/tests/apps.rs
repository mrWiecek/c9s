//! `:app`: app registrations and enterprise applications as one item, and
//! its tabs.

use c9s_core::apps::{AppDetail, Application, ServicePrincipal, MICROSOFT_TENANT};
use c9s_core::cache::Cache;
use c9s_core::loader::{ChunkSink, FetchOutcome, Fetcher, LoadReceiver, LoadRequest, LoadedData};
use c9s_core::models::Subscription;
use c9s_core::rbac::Principal;
use c9s_core::state::{AppTab, ViewMode};
use c9s_core::App;
use c9s_tui::desktop::Desktop;
use c9s_tui::{ui, Event, TuiApp};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures::future::BoxFuture;
use futures::FutureExt;
use ratatui::{backend::TestBackend, Terminal};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Records what would have been opened (and copied).
#[derive(Clone, Default)]
struct Opened(Arc<Mutex<Vec<String>>>);

impl Desktop for Opened {
    fn copy(&mut self, text: &str) -> Result<(), String> {
        self.0.lock().unwrap().push(format!("copy {text}"));
        Ok(())
    }
    fn open(&mut self, url: &str) -> Result<(), String> {
        self.0.lock().unwrap().push(url.to_string());
        Ok(())
    }
}

fn in_days(days: i64) -> String {
    (chrono::Utc::now() + chrono::Duration::days(days) + chrono::Duration::hours(days.signum()))
        .to_rfc3339()
}

fn billing_registration() -> Application {
    serde_json::from_value(serde_json::json!({
        "id": "app-obj", "app_id": "app-1", "display_name": "billing-api",
        "sign_in_audience": "AzureADMyOrg", "created": "2025-01-01T00:00:00Z",
        "identifier_uris": ["api://billing"], "redirect_uris": ["https://billing/cb"],
        "credentials": [
            {"name": "ci", "kind": "secret", "start": "", "end": in_days(-3)},
            {"name": "prod-cert", "kind": "certificate", "start": "", "end": in_days(200)}
        ],
        "required": [{"resource_app_id": "graph", "scopes": ["s1"], "roles": ["r1"]}],
        "scopes": [{"id": "x", "value": "Invoices.Read", "display": "Read invoices",
                    "audience": "Admin", "enabled": true}],
        "roles": [{"id": "y", "value": "Billing.Admin", "display": "Billing admins",
                   "audience": "User, Application", "enabled": true}]
    }))
    .unwrap()
}

fn sp(id: &str, app_id: &str, name: &str, sp_type: &str, tenant: &str) -> ServicePrincipal {
    ServicePrincipal {
        id: id.into(),
        app_id: app_id.into(),
        display_name: name.into(),
        sp_type: sp_type.into(),
        enabled: true,
        assignment_required: true,
        owner_tenant: tenant.into(),
        credentials: vec![],
        alternative_names: vec![],
    }
}

fn detail() -> AppDetail {
    serde_json::from_value(serde_json::json!({
        "app_owners": [{"id": "u1", "display_name": "Ada Lovelace", "kind": "user", "detail": "ada@contoso.com"}],
        "sp_owners": [],
        "delegated": [{"resource_id": "sp-graph", "scopes": ["User.Read"], "consent_type": "AllPrincipals"}],
        "app_roles": [],
        "assigned": [{"resource_id": "sp-1", "resource_name": "", "app_role_id": "y",
                      "principal_id": "g1", "principal_name": "Finance", "principal_type": "Group"}],
        "resources": [{"id": "sp-graph", "app_id": "graph", "display_name": "Microsoft Graph",
            "scopes": [{"id": "s1", "value": "User.Read", "display": "Sign in and read user profile", "audience": "User", "enabled": true}],
            "roles": [{"id": "r1", "value": "User.Read.All", "display": "Read all users", "audience": "Application", "enabled": true}]}],
        "application_json": {"displayName": "billing-api"},
        "service_principal_json": {"displayName": "billing-api", "appRoleAssignmentRequired": true}
    }))
    .unwrap()
}

struct Graph;

impl Fetcher for Graph {
    fn fetch(&self, request: LoadRequest, _sink: ChunkSink) -> BoxFuture<'static, FetchOutcome> {
        let data = match request {
            LoadRequest::Applications => Ok(LoadedData::Applications(
                vec![billing_registration()],
                vec![
                    sp("sp-1", "app-1", "billing-api", "Application", "home"),
                    sp("sp-mi", "mi-1", "vm-identity", "ManagedIdentity", ""),
                    sp(
                        "sp-ms",
                        "ms-1",
                        "Office 365",
                        "Application",
                        MICROSOFT_TENANT,
                    ),
                ],
            )),
            LoadRequest::AppDetail { app_id, .. } => {
                Ok(LoadedData::AppDetail(app_id, Box::new(detail())))
            }
            LoadRequest::RoleAssignments { .. } => Ok(LoadedData::RoleAssignments(vec![])),
            LoadRequest::RoleDefinitions { .. } => Ok(LoadedData::RoleDefinitions(vec![])),
            LoadRequest::Principals { .. } => Ok(LoadedData::Principals(Vec::<Principal>::new())),
            LoadRequest::PrincipalGroups { principal_id } => {
                Ok(LoadedData::PrincipalGroups(principal_id, vec![]))
            }
            _ => Err("not in this test".into()),
        };
        async move {
            FetchOutcome {
                data,
                failed_subscriptions: vec![],
                notice: None,
            }
        }
        .boxed()
    }
}

async fn press(app: &mut TuiApp, code: KeyCode) {
    app.handle_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
        .await
        .unwrap();
}

async fn command(app: &mut TuiApp, text: &str) {
    press(app, KeyCode::Char(':')).await;
    for c in text.chars() {
        press(app, KeyCode::Char(c)).await;
    }
    press(app, KeyCode::Enter).await;
}

async fn settle(app: &mut TuiApp, loads: &mut LoadReceiver) {
    while app.state().loading {
        app.apply_load(loads.recv().await.unwrap());
    }
}

fn screen(app: &mut TuiApp) -> String {
    let mut terminal = Terminal::new(TestBackend::new(170, 30)).unwrap();
    terminal.draw(|f| ui::draw(f, app).unwrap()).unwrap();
    let buf = terminal.backend().buffer();
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_shows(app: &mut TuiApp, expected: &[&str]) {
    let text = screen(app);
    for e in expected {
        assert!(text.contains(e), "{e}\n{text}");
    }
}

#[tokio::test]
async fn applications_one_row_each_with_tabs() {
    let mut app = TuiApp::new(App::with_fetcher(
        Cache::new(Path::new(":memory:")).unwrap(),
        Some(Arc::new(Graph)),
    ));
    let mut loads = app.take_load_receiver().unwrap();

    command(&mut app, "app").await;
    assert_eq!(app.state().view, ViewMode::AppList);
    settle(&mut app, &mut loads).await;
    let names: Vec<String> = app.state().app_rows().into_iter().map(|r| r.name).collect();
    assert_eq!(names, ["billing-api", "vm-identity"], "Microsoft's hidden");
    assert_shows(
        &mut app,
        &[
            "Applications (1 registered, 0 external, 1 managed identities, 1 with credentials expired or expiring)",
            "registered",
            "managed identity",
            "expired 3 days ago (secret)",
            "no credentials",
        ],
    );
    press(&mut app, KeyCode::Char('m')).await;
    assert_eq!(app.state().app_rows().len(), 3, "m shows Microsoft's");
    press(&mut app, KeyCode::Char('m')).await;

    // Enter: the app, Overview first (registration and enterprise app).
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(app.state().view, ViewMode::AppDetail);
    settle(&mut app, &mut loads).await;
    assert_shows(
        &mut app,
        &[
            "billing-api · registered",
            "1 Overview",
            "7 JSON",
            "Registration object ID",
            "app-obj",
            "Enterprise application",
            "Assignment required",
            "https://billing/cb",
        ],
    );

    // 2: requested vs granted, with names and consent.
    press(&mut app, KeyCode::Char('2')).await;
    assert_eq!(app.state().app_tab, AppTab::Permissions);
    assert_shows(
        &mut app,
        &[
            "not granted",
            "application",
            "User.Read.All",
            "granted (admin consent)",
            "Microsoft Graph",
            "Sign in and read user profile",
        ],
    );
    // 3 exposed API, 4 owners, 5 credentials, 6 assigned.
    press(&mut app, KeyCode::Char('3')).await;
    assert_shows(
        &mut app,
        &["api://billing", "Invoices.Read", "Admin", "Billing.Admin"],
    );
    press(&mut app, KeyCode::Char('4')).await;
    assert_shows(
        &mut app,
        &["App registration owners", "Ada Lovelace", "ada@contoso.com"],
    );
    press(&mut app, KeyCode::Char('5')).await;
    assert_shows(
        &mut app,
        &["expired 3 days ago", "expires in 200 days", "prod-cert"],
    );
    press(&mut app, KeyCode::Char('6')).await;
    assert_shows(&mut app, &["Group", "Finance", "Billing.Admin"]);
    press(&mut app, KeyCode::Char('7')).await;
    assert_shows(
        &mut app,
        &["App registration", "\"displayName\": \"billing-api\""],
    );

    // p: the enterprise app's Azure access. Esc back to the app, Esc to the list.
    press(&mut app, KeyCode::Char('p')).await;
    assert_eq!(app.state().view, ViewMode::RoleAssignments);
    assert_eq!(app.state().ra_principal.as_deref(), Some("sp-1"));
    press(&mut app, KeyCode::Esc).await;
    assert_eq!(app.state().view, ViewMode::AppDetail);
    press(&mut app, KeyCode::Esc).await;
    assert_eq!(app.state().view, ViewMode::AppList);
}

#[tokio::test]
async fn managed_identities_say_which_kind_and_where_they_live() {
    let opened = Opened::default();
    let mut core = App::with_fetcher(
        Cache::new(Path::new(":memory:")).unwrap(),
        Some(Arc::new(Graph)),
    );
    core.state_mut().subscriptions = vec![Subscription {
        id: "s1".into(),
        name: "prod-identity".into(),
        state: "Enabled".into(),
        tenant_id: "tenant-1".into(),
    }];
    let mut app = TuiApp::with_desktop(core, Box::new(opened.clone()));
    let mut loads = app.take_load_receiver().unwrap();
    command(&mut app, "app").await;
    settle(&mut app, &mut loads).await;

    const UAMI: &str = "/subscriptions/s1/resourceGroups/rg-identities/providers/Microsoft.ManagedIdentity/userAssignedIdentities/id-deploy";
    const VM: &str = "/subscriptions/s1/resourceGroups/rg-app/providers/Microsoft.Compute/virtualMachines/vm-web";
    let mut user = sp("sp-u", "mi-u", "id-deploy", "ManagedIdentity", "");
    user.alternative_names = vec!["isExplicit=True".into(), UAMI.into()];
    let mut system = sp("sp-s", "mi-s", "vm-web", "ManagedIdentity", "");
    system.alternative_names = vec!["isExplicit=False".into(), VM.into()];
    app.state_mut().service_principals = vec![user, system];
    app.state_mut().applications = vec![];

    assert_shows(&mut app, &["user-assigned MI", "system-assigned MI"]);
    // "managed" still finds both.
    press(&mut app, KeyCode::Char('/')).await;
    for c in "managed".chars() {
        press(&mut app, KeyCode::Char(c)).await;
    }
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(app.state().app_rows().len(), 2);
    press(&mut app, KeyCode::Esc).await; // clears the filter

    // The user-assigned identity: its own resource, and o opens it.
    let at = app
        .state()
        .app_rows()
        .iter()
        .position(|r| r.name == "id-deploy")
        .unwrap();
    app.state_mut().list_state.selected = at;
    press(&mut app, KeyCode::Enter).await;
    settle(&mut app, &mut loads).await;
    assert_eq!(app.state().view, ViewMode::AppDetail);
    assert_shows(
        &mut app,
        &[
            "Managed identity",
            "User-assigned (an Azure resource of its own)",
            "id-deploy",
            "Microsoft.ManagedIdentity/userAssignedIdentities",
            "prod-identity (s1)",
            "rg-identities",
            "Portal (o)",
        ],
    );
    // y still copies the app ID (the cursor starts on it); j/k select
    // another field, c copies it.
    press(&mut app, KeyCode::Char('y')).await;
    assert_eq!(
        opened.0.lock().unwrap().last().map(String::as_str),
        Some("copy mi-u")
    );
    let fields = c9s_core::app_view::overview_fields(app.state());
    let rg = fields
        .iter()
        .position(|f| f.label == "Resource group")
        .unwrap();
    while app.state().app_field < rg {
        press(&mut app, KeyCode::Char('j')).await;
    }
    press(&mut app, KeyCode::Char('c')).await;
    assert_eq!(
        opened.0.lock().unwrap().last().map(String::as_str),
        Some("copy rg-identities")
    );
    let id = fields
        .iter()
        .position(|f| f.label == "Resource ID")
        .unwrap();
    while app.state().app_field < id {
        press(&mut app, KeyCode::Char('j')).await;
    }
    press(&mut app, KeyCode::Char('y')).await;
    assert_eq!(
        opened.0.lock().unwrap().last().map(String::as_str),
        Some(format!("copy {UAMI}").as_str())
    );

    // o asks which page: the enterprise application or the Azure resource
    // (no registration for a managed identity); the selected field's page
    // (the Managed identity section) comes first.
    press(&mut app, KeyCode::Char('o')).await;
    let (pages, selected) = app.state().portal_choice.clone().expect("a choice");
    let names: Vec<&str> = pages.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "Enterprise application",
            "Azure resource (the user-assigned identity)"
        ]
    );
    assert_eq!(selected, 1);
    assert_shows(
        &mut app,
        &["Open in the Azure Portal", "1  Enterprise application"],
    );
    press(&mut app, KeyCode::Enter).await;
    assert!(app.state().portal_choice.is_none());
    assert_eq!(
        opened.0.lock().unwrap().last().map(String::as_str),
        Some(format!("https://portal.azure.com/#@tenant-1/resource{UAMI}").as_str())
    );
    // 1 opens the enterprise application's Entra page instead.
    press(&mut app, KeyCode::Char('o')).await;
    press(&mut app, KeyCode::Char('1')).await;
    let last = opened.0.lock().unwrap().last().cloned().unwrap();
    assert!(
        last.contains("ManagedAppMenuBlade") && last.contains("sp-u"),
        "{last}"
    );
    // Esc closes the choice without opening anything.
    let before = opened.0.lock().unwrap().len();
    press(&mut app, KeyCode::Char('o')).await;
    press(&mut app, KeyCode::Esc).await;
    assert!(app.state().portal_choice.is_none());
    assert_eq!(opened.0.lock().unwrap().len(), before);
    assert_eq!(
        app.state().view,
        ViewMode::AppDetail,
        "Esc only closed the choice"
    );

    // Search works on every tab, the JSON one too.
    press(&mut app, KeyCode::Char('7')).await;
    press(&mut app, KeyCode::Char('/')).await;
    for c in "billing".chars() {
        press(&mut app, KeyCode::Char(c)).await;
    }
    press(&mut app, KeyCode::Enter).await;
    assert!(
        app.state().detail_match.is_some(),
        "a match on the JSON tab"
    );
    assert!(
        app.state()
            .notice
            .as_deref()
            .unwrap_or_default()
            .contains("match 1 of"),
        "{:?}",
        app.state().notice
    );
    press(&mut app, KeyCode::Char('n')).await;
    assert!(app
        .state()
        .notice
        .as_deref()
        .unwrap_or_default()
        .contains("match"));
    press(&mut app, KeyCode::Esc).await; // clears the search
    assert!(app.state().detail_search.is_empty());
    press(&mut app, KeyCode::Char('1')).await;

    // The system-assigned one: the resource it belongs to.
    press(&mut app, KeyCode::Esc).await;
    assert_eq!(
        app.state().view,
        ViewMode::AppList,
        "Esc goes back to the list"
    );
    let at = app
        .state()
        .app_rows()
        .iter()
        .position(|r| r.name == "vm-web")
        .unwrap();
    app.state_mut().list_state.selected = at;
    press(&mut app, KeyCode::Char('o')).await;
    press(&mut app, KeyCode::Char('2')).await;
    assert_eq!(
        opened.0.lock().unwrap().last().map(String::as_str),
        Some(format!("https://portal.azure.com/#@tenant-1/resource{VM}").as_str()),
        "o in the list offers the VM too"
    );
    press(&mut app, KeyCode::Enter).await;
    settle(&mut app, &mut loads).await;
    assert_shows(
        &mut app,
        &[
            "System-assigned (part of the resource below)",
            "Assigned to",
            "vm-web",
            "Microsoft.Compute/virtualMachines",
            "rg-app",
        ],
    );
}
