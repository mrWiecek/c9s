//! The resource detail view: opens at once from the list row, loads the full
//! document in the background, tabs and scrolling, copy and Portal links.

use c9s_core::cache::Cache;
use c9s_core::loader::{ChunkSink, FetchOutcome, Fetcher, LoadRequest, LoadedData};
use c9s_core::models::{Resource, Subscription};
use c9s_core::state::{DetailTab, ViewMode};
use c9s_core::App;
use c9s_tui::{ui, Desktop, Event, TuiApp};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures::future::BoxFuture;
use futures::FutureExt;
use ratatui::{backend::TestBackend, Terminal};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

const VM_ID: &str =
    "/subscriptions/sub-1/resourceGroups/rg-app/providers/Microsoft.Compute/virtualMachines/vm-1";

fn vm() -> Resource {
    Resource {
        id: VM_ID.into(),
        name: "vm-1".into(),
        resource_type: "microsoft.compute/virtualmachines".into(),
        location: "westeurope".into(),
        resource_group: "rg-app".into(),
        subscription_id: "sub-1".into(),
        tags: HashMap::from([("env".to_string(), "dev".to_string())]),
    }
}

fn vm_document() -> Value {
    let disks: Vec<Value> = (0..40)
        .map(|i| json!({"lun": i, "name": format!("data-{i}")}))
        .collect();
    json!({
        "id": VM_ID, "name": "vm-1", "type": "microsoft.compute/virtualmachines",
        "location": "westeurope", "zones": ["2"],
        "tags": {"env": "prod", "owner": "platform"},
        "properties": {
            "provisioningState": "Succeeded",
            "hardwareProfile": {"vmSize": "Standard_D4s_v5"},
            "storageProfile": {"dataDisks": disks}
        }
    })
}

/// Answers detail requests with `document` (or an error); records requests.
struct FakeAzure {
    document: Result<Value, String>,
    requests: Mutex<Vec<LoadRequest>>,
}

impl Fetcher for FakeAzure {
    fn fetch(&self, request: LoadRequest, _sink: ChunkSink) -> BoxFuture<'static, FetchOutcome> {
        self.requests.lock().unwrap().push(request.clone());
        let data = match request {
            LoadRequest::ResourceDetail { .. } => {
                self.document.clone().map(LoadedData::ResourceDetail)
            }
            // A resource group: the VM. Everything: two more resources.
            LoadRequest::Resources { resource_group, .. } => {
                let tagged = |name: &str, env: &str| Resource {
                    id: format!("/subscriptions/sub-1/resourceGroups/rg-web/providers/x/{name}"),
                    name: name.into(),
                    resource_group: "rg-web".into(),
                    tags: HashMap::from([("env".to_string(), env.to_string())]),
                    ..vm()
                };
                Ok(LoadedData::Resources(match resource_group {
                    Some(_) => vec![vm()],
                    None => vec![
                        Resource {
                            tags: HashMap::from([("env".to_string(), "prod".to_string())]),
                            ..vm()
                        },
                        tagged("web-1", "prod"),
                        tagged("db-1", "dev"),
                    ],
                }))
            }
            // The subscription's resource groups: rg-app second.
            LoadRequest::ResourceGroups { .. } => Ok(LoadedData::ResourceGroups(
                ["rg-web", "rg-app"]
                    .into_iter()
                    .map(|name| c9s_core::models::ResourceGroup {
                        id: format!("/subscriptions/sub-1/resourceGroups/{name}"),
                        name: name.into(),
                        location: "westeurope".into(),
                        subscription_id: "sub-1".into(),
                        tags: HashMap::new(),
                    })
                    .collect(),
            )),
            _ => Err("unexpected request".into()),
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

/// Records what would have been copied or opened.
#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<String>>>);

impl Desktop for Recorder {
    fn copy(&mut self, text: &str) -> Result<(), String> {
        self.0.lock().unwrap().push(format!("copy {text}"));
        Ok(())
    }
    fn open(&mut self, url: &str) -> Result<(), String> {
        self.0.lock().unwrap().push(format!("open {url}"));
        Err("no display".into())
    }
}

fn setup(document: Result<Value, String>) -> (TuiApp, Arc<FakeAzure>, Recorder) {
    let azure = Arc::new(FakeAzure {
        document,
        requests: Mutex::new(vec![]),
    });
    let mut core = App::with_fetcher(
        Cache::new(Path::new(":memory:")).unwrap(),
        Some(azure.clone()),
    );
    let state = core.state_mut();
    state.subscriptions = vec![Subscription {
        id: "sub-1".into(),
        name: "prod".into(),
        state: "Enabled".into(),
        tenant_id: "tenant-1".into(),
    }];
    state.selected_subscription = state.subscriptions.first().cloned();
    state.resources = vec![vm()];
    state.view = ViewMode::ResourceList;
    let recorder = Recorder::default();
    (
        TuiApp::with_desktop(core, Box::new(recorder.clone())),
        azure,
        recorder,
    )
}

async fn press(app: &mut TuiApp, code: KeyCode) {
    app.handle_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
        .await
        .unwrap();
}

fn screen(terminal: &mut Terminal<TestBackend>, app: &mut TuiApp) -> String {
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

#[tokio::test]
async fn detail_opens_from_the_list_row_then_shows_the_full_document() {
    let (mut app, azure, _) = setup(Ok(vm_document()));
    let mut loads = app.take_load_receiver().unwrap();
    let mut terminal = Terminal::new(TestBackend::new(110, 40)).unwrap();

    press(&mut app, KeyCode::Enter).await;
    assert_eq!(app.state().view, ViewMode::ResourceDetail);
    // Before Azure answers: what the list knew.
    let text = screen(&mut terminal, &mut app);
    assert!(text.contains("vm-1") && text.contains("rg-app"), "{text}");
    assert!(text.contains("loading details"), "{text}");

    app.apply_load(loads.recv().await.unwrap());
    assert_eq!(
        azure.requests.lock().unwrap().last().unwrap(),
        &LoadRequest::ResourceDetail {
            resource_id: VM_ID.into(),
            subscription_id: "sub-1".into(),
        }
    );
    let text = screen(&mut terminal, &mut app);
    if std::env::var("C9S_SHOW_SCREEN").is_ok() {
        println!("{text}");
    }
    assert!(text.contains("Standard_D4s_v5"), "{text}");
    assert!(text.contains("Succeeded"), "{text}");
    assert!(!text.contains("loading details"));

    // Tags tab, from the document.
    press(&mut app, KeyCode::Char('2')).await;
    let text = screen(&mut terminal, &mut app);
    assert!(
        text.contains("owner") && text.contains("platform"),
        "{text}"
    );

    // JSON tab (4) scrolls; G reaches the end, g the top.
    press(&mut app, KeyCode::Char('4')).await;
    assert_eq!(app.state().detail_tab, DetailTab::Json);
    let top = screen(&mut terminal, &mut app);
    assert!(top.contains("\"hardwareProfile\""), "{top}");
    press(&mut app, KeyCode::Char('G')).await;
    let bottom = screen(&mut terminal, &mut app);
    assert!(bottom.contains("data-39"), "{bottom}");
    assert!(!bottom.contains("\"hardwareProfile\""));
    press(&mut app, KeyCode::Char('g')).await;
    assert_eq!(app.state().detail_scroll, 0);

    // Esc returns to the list with the resource still selected.
    press(&mut app, KeyCode::Esc).await;
    assert_eq!(app.state().view, ViewMode::ResourceList);
    assert_eq!(app.state().selected_id().as_deref(), Some(VM_ID));
}

#[tokio::test]
async fn reopening_shows_the_cached_document_before_azure_answers() {
    let (mut app, _, _) = setup(Ok(vm_document()));
    // The resource must be cached for its details to be.
    let repo = app.core().cache().repository();
    repo.replace_subscriptions(&app.state().subscriptions.clone())
        .unwrap();
    repo.replace_resources(&["sub-1".into()], None, &[vm()])
        .unwrap();
    let mut loads = app.take_load_receiver().unwrap();

    press(&mut app, KeyCode::Enter).await;
    app.apply_load(loads.recv().await.unwrap());
    press(&mut app, KeyCode::Esc).await;

    press(&mut app, KeyCode::Enter).await;
    assert!(app.state().loading);
    assert_eq!(
        app.state().resource_document.as_ref().unwrap()["zones"][0],
        "2"
    );
    let fresh = app.state().current_freshness().unwrap();
    assert_eq!(fresh.source, c9s_core::state::DataSource::Cache);
}

#[tokio::test]
async fn a_failed_load_keeps_the_basic_view_with_a_notice() {
    let (mut app, _, _) = setup(Err("AuthorizationFailed".into()));
    let mut loads = app.take_load_receiver().unwrap();
    press(&mut app, KeyCode::Enter).await;
    app.apply_load(loads.recv().await.unwrap());

    assert_eq!(app.state().view, ViewMode::ResourceDetail);
    assert!(app.state().error.is_none(), "no popup");
    let notice = app.state().notice.clone().unwrap();
    assert!(notice.contains("Details unavailable") && notice.contains("AuthorizationFailed"));
}

#[tokio::test]
async fn y_copies_and_o_opens_the_portal_with_the_tenant() {
    let (mut app, _, recorder) = setup(Ok(vm_document()));
    press(&mut app, KeyCode::Char('y')).await; // from the list
    press(&mut app, KeyCode::Enter).await;
    press(&mut app, KeyCode::Char('o')).await; // from the detail view

    let calls = recorder.0.lock().unwrap().clone();
    assert_eq!(
        calls,
        [
            format!("copy {VM_ID}"),
            format!("open https://portal.azure.com/#@tenant-1/resource{VM_ID}"),
            // No browser (e.g. WSL without wslview): the link is copied instead,
            // because it is too long for the status bar.
            format!("copy https://portal.azure.com/#@tenant-1/resource{VM_ID}"),
        ]
    );
    let notice = app.state().notice.clone().unwrap();
    assert!(
        notice.contains("No browser could be started") && notice.contains("link copied"),
        "{notice}"
    );
}

#[tokio::test]
async fn c_copies_the_current_view() {
    let (mut app, _, recorder) = setup(Ok(vm_document()));
    let mut loads = app.take_load_receiver().unwrap();

    // In the resource list: every row, tab-separated with a header.
    press(&mut app, KeyCode::Char('c')).await;
    // In the detail view, JSON tab: the whole document.
    press(&mut app, KeyCode::Enter).await;
    app.apply_load(loads.recv().await.unwrap());
    press(&mut app, KeyCode::Char('4')).await;
    press(&mut app, KeyCode::Char('c')).await;

    let calls = recorder.0.lock().unwrap().clone();
    assert!(
        calls[0].starts_with("copy name\ttype\tlocation\tresource group\tsubscription\tid\nvm-1\t"),
        "{}",
        calls[0]
    );
    let json = calls[1].strip_prefix("copy ").unwrap();
    let parsed: Value = serde_json::from_str(json).expect("valid JSON is copied");
    assert_eq!(parsed, vm_document());
    assert!(app
        .state()
        .notice
        .as_deref()
        .unwrap()
        .starts_with("Copied JSON of vm-1"));
}

async fn type_text(app: &mut TuiApp, text: &str) {
    for c in text.chars() {
        press(app, KeyCode::Char(c)).await;
    }
}

#[tokio::test]
async fn slash_searches_the_detail_view_like_vim() {
    let (mut app, _, _) = setup(Ok(vm_document()));
    let mut loads = app.take_load_receiver().unwrap();
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    press(&mut app, KeyCode::Enter).await;
    app.apply_load(loads.recv().await.unwrap());
    press(&mut app, KeyCode::Char('4')).await; // JSON
    screen(&mut terminal, &mut app);
    let line_of = |app: &TuiApp| app.state().detail_match.unwrap().0;

    // Typing jumps to the first match, which is far below the screen.
    press(&mut app, KeyCode::Char('/')).await;
    assert_eq!(app.state().view, ViewMode::ResourceDetail, "no list filter");
    type_text(&mut app, "data-3").await;
    let first = line_of(&app);
    let text = screen(&mut terminal, &mut app);
    assert!(text.contains("/data-3_"), "prompt in the footer");
    assert!(text.contains("\"data-3\""), "scrolled to the match: {text}");
    assert!(app.state().detail_scroll > 0);
    assert!(app
        .state()
        .notice
        .as_deref()
        .unwrap()
        .contains("match 1 of 11"));

    // Enter keeps it; n and N move; n past the last wraps to the top.
    press(&mut app, KeyCode::Enter).await;
    press(&mut app, KeyCode::Char('n')).await;
    assert!(line_of(&app) > first);
    press(&mut app, KeyCode::Char('N')).await;
    assert_eq!(line_of(&app), first);
    for _ in 0..11 {
        press(&mut app, KeyCode::Char('n')).await;
    }
    assert_eq!(line_of(&app), first, "11 matches: back where it started");
    assert!(app
        .state()
        .notice
        .as_deref()
        .unwrap()
        .contains("continuing at TOP"));

    // Esc clears the search first, then goes back to the list.
    press(&mut app, KeyCode::Esc).await;
    assert!(app.state().detail_search.is_empty());
    assert_eq!(app.state().view, ViewMode::ResourceDetail);
    press(&mut app, KeyCode::Esc).await;
    assert_eq!(app.state().view, ViewMode::ResourceList);
}

#[tokio::test]
async fn esc_while_typing_cancels_and_restores_the_view() {
    let (mut app, _, _) = setup(Ok(vm_document()));
    let mut loads = app.take_load_receiver().unwrap();
    press(&mut app, KeyCode::Enter).await;
    app.apply_load(loads.recv().await.unwrap());
    press(&mut app, KeyCode::Char('4')).await;

    press(&mut app, KeyCode::Char('/')).await;
    type_text(&mut app, "nothing-like-this").await;
    assert!(app
        .state()
        .notice
        .as_deref()
        .unwrap()
        .contains("Pattern not found"));
    press(&mut app, KeyCode::Esc).await;
    assert!(app.state().detail_search_input.is_none());
    assert!(app.state().detail_search.is_empty());
    assert_eq!(app.state().detail_scroll, 0);
    assert_eq!(
        app.state().view,
        ViewMode::ResourceDetail,
        "Esc only cancels"
    );

    // Capital letters make the search exact-case (smartcase).
    press(&mut app, KeyCode::Char('/')).await;
    type_text(&mut app, "VMSIZE").await;
    assert!(app.state().notice.as_deref().unwrap().contains("not found"));
    press(&mut app, KeyCode::Esc).await;
    press(&mut app, KeyCode::Char('/')).await;
    type_text(&mut app, "vmsize").await;
    assert!(app
        .state()
        .notice
        .as_deref()
        .unwrap()
        .contains("match 1 of 1"));
}

#[tokio::test]
async fn enter_on_a_tag_lists_the_tenant_resources_with_it_and_esc_restores_the_list() {
    let (mut app, azure, _) = setup(Ok(vm_document()));
    let mut loads = app.take_load_receiver().unwrap();
    {
        let state = app.state_mut();
        state.resources.clear();
        state.resource_groups = vec![c9s_core::models::ResourceGroup {
            id: "/subscriptions/sub-1/resourceGroups/rg-app".into(),
            name: "rg-app".into(),
            location: "westeurope".into(),
            subscription_id: "sub-1".into(),
            tags: HashMap::new(),
        }];
        state.view = ViewMode::ResourceGroupList;
    }
    press(&mut app, KeyCode::Enter).await; // the group's resources
    app.apply_load(loads.recv().await.unwrap());
    assert_eq!(app.state().filtered_resources().len(), 1);
    press(&mut app, KeyCode::Enter).await; // the VM's detail
    app.apply_load(loads.recv().await.unwrap());
    press(&mut app, KeyCode::Char('2')).await; // Tags: env=prod, owner=platform

    // Enter on env=prod: every resource in the tenant tagged env=prod.
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(app.state().view, ViewMode::ResourceList);
    assert!(app.state().cross_subscription_mode);
    app.apply_load(loads.recv().await.unwrap());
    let names: Vec<String> = app
        .state()
        .filtered_resources()
        .iter()
        .map(|r| r.name.clone())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(names.contains(&"web-1".to_string()) && names.contains(&"vm-1".to_string()));
    assert!(matches!(
        azure.requests.lock().unwrap().last(),
        Some(LoadRequest::Resources {
            resource_group: None,
            ..
        })
    ));

    // Esc: the detail again; Esc: the resource group's list, loaded again.
    press(&mut app, KeyCode::Esc).await;
    assert_eq!(app.state().view, ViewMode::ResourceDetail);
    assert_eq!(app.state().detail_tab, DetailTab::Tags);
    press(&mut app, KeyCode::Esc).await;
    assert_eq!(app.state().view, ViewMode::ResourceList);
    assert_eq!(
        app.state().current_resource_group.as_deref(),
        Some("rg-app")
    );
    app.apply_load(loads.recv().await.unwrap());
    let names: Vec<String> = app
        .state()
        .filtered_resources()
        .iter()
        .map(|r| r.name.clone())
        .collect();
    assert_eq!(names, ["vm-1"], "the group's list, not the tag's");
    assert!(
        app.state().resource_query.is_empty(),
        "the tag filter is gone"
    );
}

/// The Overview row with this label, and its index among the rows.
fn overview_row(app: &TuiApp, label: &str) -> (usize, String) {
    app.state()
        .detail_overview()
        .unwrap()
        .into_iter()
        .flat_map(|s| s.rows)
        .enumerate()
        .find(|(_, (l, _))| l == label)
        .map(|(i, (_, v))| (i, v))
        .unwrap()
}

#[tokio::test]
async fn overview_rows_go_to_the_resource_group_and_subscription() {
    let (mut app, azure, _) = setup(Ok(vm_document()));
    let mut loads = app.take_load_receiver().unwrap();
    press(&mut app, KeyCode::Enter).await;
    app.apply_load(loads.recv().await.unwrap());
    assert_eq!(app.state().view, ViewMode::ResourceDetail);

    // The subscription is named, not only its ID.
    let (_, sub) = overview_row(&app, "Subscription");
    assert_eq!(sub, "prod (sub-1)");

    // j to the Resource group row (highlighted), Enter: its resources,
    // the VM highlighted.
    let (at, rg) = overview_row(&app, "Resource group");
    assert_eq!(rg, "rg-app");
    for _ in 0..at {
        press(&mut app, KeyCode::Char('j')).await;
    }
    assert_eq!(
        app.state().selected_detail_field().unwrap().0,
        "Resource group"
    );
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(app.state().view, ViewMode::ResourceList);
    assert_eq!(
        app.state().current_resource_group.as_deref(),
        Some("rg-app")
    );
    app.apply_load(loads.recv().await.unwrap());
    assert_eq!(app.state().selected_resource().unwrap().id, VM_ID);
    assert!(azure.requests.lock().unwrap().iter().any(|r| matches!(
        r,
        LoadRequest::Resources { resource_group: Some(g), subscription_ids }
            if g == "rg-app" && subscription_ids == &["sub-1".to_string()]
    )));

    // Esc: back on the detail view, on the same row.
    press(&mut app, KeyCode::Esc).await;
    assert_eq!(app.state().view, ViewMode::ResourceDetail);
    assert_eq!(
        app.state().selected_detail_field().unwrap().0,
        "Resource group"
    );

    // The Subscription row: its resource groups, the VM's highlighted
    // once they load.
    press(&mut app, KeyCode::Char('j')).await;
    assert_eq!(
        app.state().selected_detail_field().unwrap().0,
        "Subscription"
    );
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(app.state().view, ViewMode::ResourceGroupList);
    assert_eq!(
        app.state().selected_subscription.as_ref().unwrap().id,
        "sub-1"
    );
    while app.state().loading {
        app.apply_load(loads.recv().await.unwrap());
    }
    assert_eq!(
        app.state().selected_resource_group().unwrap().name,
        "rg-app"
    );
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    let text = screen(&mut terminal, &mut app);
    assert!(text.contains("rg-web"), "{text}");
}

#[tokio::test]
async fn r_and_s_from_the_resource_list_and_detail() {
    let (mut app, _, _) = setup(Ok(vm_document()));
    let mut loads = app.take_load_receiver().unwrap();

    // S in the list: the subscription's resource groups; Esc comes back.
    press(&mut app, KeyCode::Char('S')).await;
    assert_eq!(app.state().view, ViewMode::ResourceGroupList);
    while app.state().loading {
        app.apply_load(loads.recv().await.unwrap());
    }
    assert_eq!(
        app.state().selected_resource_group().unwrap().name,
        "rg-app"
    );
    press(&mut app, KeyCode::Esc).await;
    assert_eq!(app.state().view, ViewMode::ResourceList);

    // R in the detail view: the resource group's resources.
    press(&mut app, KeyCode::Enter).await;
    app.apply_load(loads.recv().await.unwrap());
    press(&mut app, KeyCode::Char('R')).await;
    assert_eq!(app.state().view, ViewMode::ResourceList);
    assert_eq!(
        app.state().current_resource_group.as_deref(),
        Some("rg-app")
    );
    while app.state().loading {
        app.apply_load(loads.recv().await.unwrap());
    }
    press(&mut app, KeyCode::Esc).await;
    assert_eq!(app.state().view, ViewMode::ResourceDetail);
}
