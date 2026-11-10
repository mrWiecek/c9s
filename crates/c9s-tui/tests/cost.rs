//! `:cost` and `$`: what a subscription or resource group cost, as a chart
//! and a table, with period, granularity, grouping, cost type and currency.

use c9s_core::cache::Cache;
use c9s_core::cost::{
    CostGroupBy, CostPeriod, CostQuery, CostReport, CostScope, CostTable, Granularity,
};
use c9s_core::loader::{ChunkSink, FetchOutcome, Fetcher, LoadRequest, LoadedData};
use c9s_core::models::Subscription;
use c9s_core::scope_tree::{ScopeTree, SubscriptionNode};
use c9s_core::state::ViewMode;
use c9s_core::App;
use c9s_tui::{ui, Event, TuiApp};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures::future::BoxFuture;
use futures::FutureExt;
use ratatui::{backend::TestBackend, Terminal};
use serde_json::json;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Answers cost queries: two days of cost in two groups (resource groups,
/// types, services or subscriptions, by the query), 10 + 5 then 12 + 6 EUR;
/// USD is 1.1 times that.
#[derive(Default)]
struct Azure(Mutex<Vec<CostQuery>>);

impl Azure {
    fn queries(&self) -> Vec<CostQuery> {
        self.0.lock().unwrap().clone()
    }
}

fn table(query: &CostQuery) -> CostTable {
    let (a, b) = match query.group_by {
        CostGroupBy::ResourceGroup => ("rg-app", "rg-data"),
        CostGroupBy::ResourceType => ("microsoft.web/sites", "microsoft.sql/servers"),
        CostGroupBy::Service => ("App Service", "SQL Database"),
        CostGroupBy::Subscription => ("s1", "s2"),
    };
    let day = |n: u64| {
        let d = query.from + chrono::Duration::days(n as i64);
        match query.granularity {
            Granularity::Daily => json!(d.format("%Y%m%d").to_string().parse::<u64>().unwrap()),
            Granularity::Monthly => json!(format!("{}T00:00:00", d.format("%Y-%m-01"))),
        }
    };
    let row = |cost: f64, d: u64, group: &str| {
        vec![
            json!(cost),
            json!(cost * 1.1),
            day(d),
            json!(group),
            json!("EUR"),
        ]
    };
    CostTable {
        columns: vec![
            "Cost".into(),
            "CostUSD".into(),
            match query.granularity {
                Granularity::Daily => "UsageDate".into(),
                Granularity::Monthly => "BillingMonth".into(),
            },
            query.group_by.dimension().into(),
            "Currency".into(),
        ],
        rows: vec![
            row(10.0, 0, a),
            row(5.0, 0, b),
            row(12.0, 1, a),
            row(6.0, 1, b),
        ],
    }
}

impl Fetcher for Azure {
    fn fetch(&self, request: LoadRequest, _sink: ChunkSink) -> BoxFuture<'static, FetchOutcome> {
        let data = match request {
            LoadRequest::Cost { query } => {
                self.0.lock().unwrap().push(query.clone());
                let table = table(&query);
                CostReport::from_table(query, &table).map(|r| LoadedData::Cost(Box::new(r)))
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

fn subscription(id: &str, name: &str) -> Subscription {
    Subscription {
        id: id.into(),
        name: name.into(),
        state: "Enabled".into(),
        tenant_id: "root".into(),
    }
}

fn setup() -> (TuiApp, Arc<Azure>) {
    let azure = Arc::new(Azure::default());
    let mut core = App::with_fetcher(
        Cache::new(Path::new(":memory:")).unwrap(),
        Some(azure.clone()),
    );
    let state = core.state_mut();
    state.subscriptions = vec![subscription("s1", "prod"), subscription("s2", "dev")];
    state.scope_tree = Some(ScopeTree {
        root: "root".into(),
        root_display_name: "Tenant Root Group".into(),
        groups: vec![],
        subscriptions: vec![
            SubscriptionNode {
                id: "s1".into(),
                display_name: "prod".into(),
                parent: "root".into(),
            },
            SubscriptionNode {
                id: "s2".into(),
                display_name: "dev".into(),
                parent: "root".into(),
            },
        ],
    });
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

async fn settle(app: &mut TuiApp, loads: &mut c9s_core::loader::LoadReceiver) {
    while app.state().loading {
        app.apply_load(loads.recv().await.unwrap());
    }
}

fn screen(app: &mut TuiApp) -> String {
    let mut terminal = Terminal::new(TestBackend::new(160, 30)).unwrap();
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
async fn dollar_on_a_subscription_shows_its_cost_by_resource_group() {
    let (mut app, azure) = setup();
    let mut loads = app.take_load_receiver().unwrap();
    // The subscription list, prod highlighted.
    app.state_mut().view = ViewMode::SubscriptionList;
    press(&mut app, KeyCode::Char('$')).await;
    assert_eq!(app.state().view, ViewMode::Cost);
    settle(&mut app, &mut loads).await;

    let q = &azure.queries()[0];
    assert_eq!(
        q.scope,
        CostScope::Subscription {
            id: "s1".into(),
            name: "prod".into()
        }
    );
    assert_eq!(q.group_by, CostGroupBy::ResourceGroup);
    assert_eq!(q.granularity, Granularity::Daily);
    assert!(!q.amortized);
    assert_eq!((q.to - q.from).num_days(), 29, "the last 30 days");

    let text = screen(&mut app);
    assert!(text.contains("Cost · prod · last 30 days"), "{text}");
    assert!(text.contains("daily · actual · EUR"), "{text}");
    assert!(text.contains("Total 33.00 EUR"), "{text}");
    assert!(text.contains("Cost by resource group (2)"), "{text}");
    assert!(text.contains("rg-app"), "{text}");
    assert!(text.contains("22.00 EUR"), "{text}");
    assert!(text.contains("66.7%"), "{text}");
    assert!(text.contains('█'), "bars: {text}");
    // The most expensive first.
    assert_eq!(app.state().cost_rows()[0].name, "rg-app");

    // u: USD, from the same report.
    press(&mut app, KeyCode::Char('u')).await;
    assert!(!app.state().loading, "no new query for the currency");
    let text = screen(&mut app);
    assert!(text.contains("Total 36.30 USD"), "{text}");
    press(&mut app, KeyCode::Char('u')).await;

    // a: amortized cost, loaded.
    press(&mut app, KeyCode::Char('a')).await;
    settle(&mut app, &mut loads).await;
    assert!(azure.queries().last().unwrap().amortized);
    assert!(screen(&mut app).contains("daily · amortized · EUR"));
    press(&mut app, KeyCode::Char('a')).await;
    assert!(
        !app.state().loading,
        "back to actual: the report from before, no query"
    );
    assert_eq!(azure.queries().len(), 2);

    // b: by resource type, then service.
    press(&mut app, KeyCode::Char('b')).await;
    settle(&mut app, &mut loads).await;
    assert_eq!(
        azure.queries().last().unwrap().group_by,
        CostGroupBy::ResourceType
    );
    assert!(screen(&mut app).contains("microsoft.web/sites"));
    press(&mut app, KeyCode::Char('b')).await;
    settle(&mut app, &mut loads).await;
    assert!(screen(&mut app).contains("Cost by service (2)"));
    // A subscription isn't split by subscription: back to resource group.
    press(&mut app, KeyCode::Char('b')).await;
    assert_eq!(
        app.state().cost_query.as_ref().unwrap().group_by,
        CostGroupBy::ResourceGroup
    );

    // Enter on rg-app: its own cost, by resource type; Esc comes back.
    press(&mut app, KeyCode::Enter).await;
    settle(&mut app, &mut loads).await;
    let q = azure.queries().last().unwrap().clone();
    assert_eq!(
        q.scope,
        CostScope::ResourceGroup {
            subscription_id: "s1".into(),
            subscription_name: "prod".into(),
            name: "rg-app".into()
        }
    );
    assert_eq!(q.group_by, CostGroupBy::ResourceType);
    assert!(screen(&mut app).contains("Cost · prod / rg-app"));
    let asked = azure.queries().len();
    press(&mut app, KeyCode::Esc).await;
    assert_eq!(app.state().view, ViewMode::Cost);
    let q = app.state().cost_query.clone().unwrap();
    assert!(matches!(q.scope, CostScope::Subscription { .. }));
    assert_eq!(
        q.group_by,
        CostGroupBy::ResourceGroup,
        "the subscription's grouping again"
    );
    settle(&mut app, &mut loads).await;
    assert_eq!(azure.queries().len(), asked, "loaded before: no query");
    press(&mut app, KeyCode::Esc).await;
    assert_eq!(app.state().view, ViewMode::SubscriptionList);
}

#[tokio::test]
async fn periods_granularity_and_the_chart_cursor() {
    let (mut app, azure) = setup();
    let mut loads = app.take_load_receiver().unwrap();
    command(&mut app, "cost prod").await;
    settle(&mut app, &mut loads).await;
    assert_eq!(app.state().view, ViewMode::Cost);

    // The last bar (today) is highlighted; ← moves to yesterday.
    let today = azure.queries()[0].to;
    let text = screen(&mut app);
    assert!(
        text.contains(&format!("{}: 0.00 EUR", today.format("%a %Y-%m-%d"))),
        "{text}"
    );
    press(&mut app, KeyCode::Home).await;
    let first = azure.queries()[0].from;
    let text = screen(&mut app);
    assert!(
        text.contains(&format!("{}: 15.00 EUR", first.format("%a %Y-%m-%d"))),
        "{text}"
    );
    press(&mut app, KeyCode::Right).await;
    assert!(screen(&mut app).contains(": 18.00 EUR"));

    // p: the last 3 months; m: per month.
    press(&mut app, KeyCode::Char('p')).await;
    settle(&mut app, &mut loads).await;
    press(&mut app, KeyCode::Char('m')).await;
    settle(&mut app, &mut loads).await;
    let q = azure.queries().last().unwrap().clone();
    assert_eq!(app.state().cost_settings.period, CostPeriod::Months3);
    assert_eq!(q.granularity, Granularity::Monthly);
    assert_eq!(q.buckets().len(), 3);
    let text = screen(&mut app);
    assert!(text.contains("last 3 months"), "{text}");
    assert!(text.contains("monthly"), "{text}");
    assert!(text.contains("Total 33.00 EUR"), "{text}");
    assert!(text.contains("per month on average"), "{text}");

    // The settings stay for the next scope.
    command(&mut app, "cost dev").await;
    settle(&mut app, &mut loads).await;
    let q = azure.queries().last().unwrap().clone();
    assert_eq!(
        q.scope,
        CostScope::Subscription {
            id: "s2".into(),
            name: "dev".into()
        }
    );
    assert_eq!(q.granularity, Granularity::Monthly);
}

#[tokio::test]
async fn the_tenant_by_subscription_and_errors() {
    let (mut app, azure) = setup();
    let mut loads = app.take_load_receiver().unwrap();
    command(&mut app, "cost all").await;
    settle(&mut app, &mut loads).await;
    let q = azure.queries()[0].clone();
    assert_eq!(
        q.scope,
        CostScope::ManagementGroup {
            id: "root".into(),
            name: "Tenant Root Group".into()
        }
    );
    // Grouped by subscription, shown by name; Enter opens one.
    for _ in 0..3 {
        press(&mut app, KeyCode::Char('b')).await;
        settle(&mut app, &mut loads).await;
    }
    assert_eq!(
        app.state().cost_query.as_ref().unwrap().group_by,
        CostGroupBy::Subscription
    );
    let text = screen(&mut app);
    assert!(text.contains("Cost by subscription (2)"), "{text}");
    assert!(text.contains("prod") && text.contains("dev"), "{text}");
    press(&mut app, KeyCode::Enter).await;
    settle(&mut app, &mut loads).await;
    assert!(matches!(
        azure.queries().last().unwrap().scope,
        CostScope::Subscription { ref id, .. } if id == "s1"
    ));

    // An unknown name says so.
    command(&mut app, "cost nowhere").await;
    assert!(app
        .state()
        .error
        .clone()
        .unwrap_or_default()
        .contains("No management group or subscription named nowhere"));
}

#[tokio::test]
async fn copying_the_table() {
    let (mut app, _) = setup();
    let mut loads = app.take_load_receiver().unwrap();
    command(&mut app, "cost prod").await;
    settle(&mut app, &mut loads).await;
    let (what, text) = c9s_core::export::view_text(app.state()).unwrap();
    assert_eq!(what, "cost of 2 resource groups");
    assert!(
        text.starts_with("resource group\tcost\tcurrency\tshare %\nrg-app\t22.00\tEUR\t66.7"),
        "{text}"
    );
}
