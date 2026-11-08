//! `:ctx`: listing tenants and switching between them.

use c9s_core::cache::Cache;
use c9s_core::loader::{ChunkSink, FetchOutcome, Fetcher, LoadRequest, LoadedData};
use c9s_core::models::{Subscription, Tenant};
use c9s_core::state::ViewMode;
use c9s_core::App;
use c9s_tui::{ui, Event, TuiApp};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures::future::BoxFuture;
use futures::FutureExt;
use ratatui::{backend::TestBackend, Terminal};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Azure(Mutex<Vec<LoadRequest>>);

impl Fetcher for Azure {
    fn fetch(&self, request: LoadRequest, _sink: ChunkSink) -> BoxFuture<'static, FetchOutcome> {
        self.0.lock().unwrap().push(request.clone());
        let data = match request {
            LoadRequest::Tenants => Ok(LoadedData::Tenants(vec![
                Tenant {
                    id: "t-1".into(),
                    name: "Contoso".into(),
                    domain: Some("contoso.onmicrosoft.com".into()),
                },
                Tenant {
                    id: "t-2".into(),
                    name: "Fabrikam".into(),
                    domain: Some("fabrikam.onmicrosoft.com".into()),
                },
            ])),
            // gcloud's configurations: none in these tests (no gcloud).
            LoadRequest::CloudContexts => Ok(LoadedData::CloudContexts(Default::default())),
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

fn setup() -> (TuiApp, Arc<Azure>) {
    let azure = Arc::new(Azure::default());
    let mut core = App::with_fetcher(
        Cache::new(Path::new(":memory:")).unwrap(),
        Some(azure.clone()),
    );
    let sub = |id: &str, tenant: &str| Subscription {
        id: id.into(),
        name: format!("sub-{id}"),
        state: "Enabled".into(),
        tenant_id: tenant.into(),
    };
    core.state_mut().subscriptions = vec![sub("a", "t-1"), sub("b", "t-2"), sub("c", "t-2")];
    (TuiApp::new(core), azure)
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

async fn last_request(azure: &Azure) -> LoadRequest {
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    azure.0.lock().unwrap().last().cloned().unwrap()
}

fn screen(app: &mut TuiApp) -> String {
    let mut terminal = Terminal::new(TestBackend::new(140, 20)).unwrap();
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
async fn ctx_lists_tenants_and_enter_switches() {
    let (mut app, azure) = setup();
    let mut loads = app.take_load_receiver().unwrap();

    command(&mut app, "ctx").await;
    assert_eq!(app.state().view, ViewMode::TenantList);
    last_request(&azure).await;
    assert!(azure.0.lock().unwrap().contains(&LoadRequest::Tenants));
    app.apply_load(loads.recv().await.unwrap());
    app.apply_load(loads.recv().await.unwrap()); // gcloud configurations
    let text = screen(&mut app);
    assert!(text.contains("● azure  All tenants"), "{text}");
    assert!(
        text.contains("Fabrikam") && text.contains("2 subscriptions"),
        "{text}"
    );

    // Rows: All tenants, Contoso, Fabrikam.
    press(&mut app, KeyCode::Char('G')).await;
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(app.state().tenant.as_deref(), Some("t-2"));
    assert!(screen(&mut app).contains("[Fabrikam]"), "breadcrumb");

    // `all` now means this tenant's subscriptions.
    command(&mut app, "rg all").await;
    assert_eq!(
        last_request(&azure).await,
        LoadRequest::ResourceGroups {
            subscription_ids: vec!["b".into(), "c".into()]
        }
    );
    command(&mut app, "sub").await;
    assert_eq!(app.state().filtered_subscriptions().len(), 2);

    command(&mut app, "ctx all").await;
    assert_eq!(app.state().tenant, None);
    command(&mut app, "sub").await;
    assert_eq!(app.state().filtered_subscriptions().len(), 3);
}

#[tokio::test]
async fn ctx_switches_by_name_domain_or_id_and_rejects_unknown() {
    let (mut app, _) = setup();
    let mut loads = app.take_load_receiver().unwrap();
    command(&mut app, "ctx").await;
    app.apply_load(loads.recv().await.unwrap());

    command(&mut app, "ctx contoso").await;
    assert_eq!(app.state().tenant.as_deref(), Some("t-1"));
    command(&mut app, "ctx fabrikam.onmicrosoft.com").await;
    assert_eq!(app.state().tenant.as_deref(), Some("t-2"));
    command(&mut app, "ctx 00000000-0000-0000-0000-000000000042").await;
    assert_eq!(
        app.state().tenant.as_deref(),
        Some("00000000-0000-0000-0000-000000000042"),
        "a GUID works before its name is known"
    );

    command(&mut app, "ctx nowhere").await;
    assert!(app
        .state()
        .error
        .as_deref()
        .unwrap()
        .contains("Context not found: nowhere"));
}

#[tokio::test]
async fn ctx_autocompletes_tenant_names() {
    let (mut app, _) = setup();
    let mut loads = app.take_load_receiver().unwrap();
    command(&mut app, "ctx").await;
    app.apply_load(loads.recv().await.unwrap());

    press(&mut app, KeyCode::Char(':')).await;
    for c in "ctx fab".chars() {
        press(&mut app, KeyCode::Char(c)).await;
    }
    press(&mut app, KeyCode::Tab).await;
    assert_eq!(app.state().command_input, "ctx Fabrikam ");
}

#[tokio::test]
async fn ctx_lists_gcloud_configurations_and_switches_cloud() {
    let (mut app, _) = setup();
    app.state_mut().gcp_configurations =
        vec![c9s_gcp_config("work", "ada@acme.com", "billing-prod")];
    command(&mut app, "ctx").await;
    let text = screen(&mut app);
    assert!(text.contains("gcp") && text.contains("work"), "{text}");
    assert!(text.contains("ada@acme.com · billing-prod"), "{text}");
    assert!(text.contains("Contexts ("), "{text}");

    // The last row is the gcloud configuration: Enter switches to Google Cloud.
    press(&mut app, KeyCode::Char('G')).await;
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(app.state().provider, c9s_core::provider::Provider::Gcp);
    assert_eq!(app.state().gcp_configuration.as_deref(), Some("work"));
    assert_eq!(app.state().view, ViewMode::SubscriptionList);
    assert!(
        app.state().subscriptions.is_empty(),
        "Azure's subscriptions are dropped"
    );
    assert!(screen(&mut app).contains("gcp ·") || screen(&mut app).contains("gcp |"));

    // By name works too, and Azure is one :ctx away.
    command(&mut app, "ctx all").await;
    assert_eq!(app.state().provider, c9s_core::provider::Provider::Azure);
    command(&mut app, "ctx work").await;
    assert_eq!(app.state().provider, c9s_core::provider::Provider::Gcp);
}

fn c9s_gcp_config(name: &str, account: &str, project: &str) -> c9s_core::state::GcpConfiguration {
    c9s_core::state::GcpConfiguration {
        name: name.into(),
        active: false,
        account: account.into(),
        project: project.into(),
    }
}

#[tokio::test]
async fn ctx_lists_aws_profiles_and_switches_cloud() {
    let (mut app, _) = setup();
    app.state_mut().aws_profiles = vec![c9s_core::aws::Profile {
        name: "prod-admin".into(),
        region: "eu-west-1".into(),
        kind: "sso".into(),
        account_id: "111122223333".into(),
    }];
    command(&mut app, "ctx").await;
    let text = screen(&mut app);
    assert!(
        text.contains("aws") && text.contains("prod-admin"),
        "{text}"
    );
    assert!(text.contains("sso · eu-west-1"), "{text}");

    // The last row is the profile: Enter switches to AWS.
    press(&mut app, KeyCode::Char('G')).await;
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(app.state().provider, c9s_core::provider::Provider::Aws);
    assert_eq!(app.state().aws_profile.as_deref(), Some("prod-admin"));
    assert_eq!(app.state().view, ViewMode::SubscriptionList);
    assert!(app.state().subscriptions.is_empty());

    command(&mut app, "ctx all").await;
    assert_eq!(app.state().provider, c9s_core::provider::Provider::Azure);
    assert_eq!(app.state().aws_profile, None);
    command(&mut app, "ctx prod-admin").await;
    assert_eq!(app.state().provider, c9s_core::provider::Provider::Aws);
}

#[tokio::test]
async fn enter_takes_the_highlighted_suggestion_and_runs() {
    let (mut app, azure) = setup();
    let mut loads = app.take_load_receiver().unwrap();
    command(&mut app, "ctx").await;
    app.apply_load(loads.recv().await.unwrap());

    // Part of a name, then Enter: no Tab needed.
    command(&mut app, "ctx fab").await;
    assert_eq!(
        app.state().tenant.as_deref(),
        Some("t-2"),
        "switched to Fabrikam"
    );
    assert!(app.state().error.is_none(), "{:?}", app.state().error);

    // ↑/↓ choose another suggestion; Enter takes that one.
    press(&mut app, KeyCode::Char(':')).await;
    for c in "ctx ".chars() {
        press(&mut app, KeyCode::Char(c)).await;
    }
    let first = c9s_core::complete::complete(&app.state().command_input, app.state()).items;
    assert!(first.len() > 1, "{first:?}");
    press(&mut app, KeyCode::Down).await;
    press(&mut app, KeyCode::Enter).await;
    assert!(!app.state().command_mode, "it ran");
    let _ = azure;

    // Nothing typed after the command and nothing picked: run as typed
    // (where you are), not the first suggestion.
    press(&mut app, KeyCode::Char(':')).await;
    for c in "res ".chars() {
        press(&mut app, KeyCode::Char(c)).await;
    }
    assert!(c9s_core::complete::enter(
        &app.state().command_input,
        app.state().command_selection,
        app.state().command_picked,
        app.state()
    )
    .is_none());
    press(&mut app, KeyCode::Esc).await;

    // A key suggestion still needs its value: Enter fills it in and waits.
    press(&mut app, KeyCode::Char(':')).await;
    for c in "res ty".chars() {
        press(&mut app, KeyCode::Char(c)).await;
    }
    press(&mut app, KeyCode::Enter).await;
    assert!(app.state().command_mode, "still typing");
    assert_eq!(app.state().command_input, "res type=");
    press(&mut app, KeyCode::Esc).await;

    // A word that already is a suggestion is kept as typed.
    press(&mut app, KeyCode::Char(':')).await;
    for c in "ctx all".chars() {
        press(&mut app, KeyCode::Char(c)).await;
    }
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(app.state().tenant, None, ":ctx all ran as typed");
}
