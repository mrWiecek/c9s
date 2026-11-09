//! `s`: sort a list by one of its columns, the other way on the second
//! pick, back to the default order with 0; the highlighted item stays.

use c9s_core::cache::Cache;
use c9s_core::models::Resource;
use c9s_core::state::ViewMode;
use c9s_core::App;
use c9s_tui::{ui, Event, TuiApp};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};
use std::path::Path;

fn resource(name: &str, kind: &str, location: &str) -> Resource {
    Resource {
        id: format!("/subscriptions/s1/resourceGroups/rg/providers/{kind}/{name}"),
        name: name.into(),
        resource_type: kind.into(),
        resource_group: "rg".into(),
        location: location.into(),
        subscription_id: "s1".into(),
        tags: Default::default(),
    }
}

fn app() -> TuiApp {
    let mut core = App::offline(Cache::new(Path::new(":memory:")).unwrap());
    let state = core.state_mut();
    state.resources = vec![
        resource("web", "Microsoft.Web/sites", "westeurope"),
        resource("Alpha", "Microsoft.Storage/storageAccounts", "northeurope"),
        resource("db", "Microsoft.Sql/servers", "westeurope"),
        resource("cache", "Microsoft.Cache/redis", "eastus"),
    ];
    state.view = ViewMode::ResourceList;
    TuiApp::new(core)
}

async fn press(app: &mut TuiApp, code: KeyCode) {
    app.handle_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
        .await
        .unwrap();
}

fn names(app: &TuiApp) -> Vec<String> {
    app.state()
        .filtered_resources()
        .iter()
        .map(|r| r.name.clone())
        .collect()
}

fn screen(app: &mut TuiApp) -> String {
    let mut terminal = Terminal::new(TestBackend::new(160, 20)).unwrap();
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
async fn s_sorts_by_a_column_both_ways_and_back() {
    let mut app = app();
    assert_eq!(names(&app), ["web", "Alpha", "db", "cache"], "as loaded");

    // Highlight "db", then sort by name: case-insensitive, db stays highlighted.
    app.state_mut().list_state.selected = 2;
    press(&mut app, KeyCode::Char('s')).await;
    let text = screen(&mut app);
    assert!(
        text.contains("Sort by") && text.contains("2  Type") && text.contains("0  Default order"),
        "{text}"
    );
    press(&mut app, KeyCode::Char('1')).await;
    assert_eq!(names(&app), ["Alpha", "cache", "db", "web"]);
    assert_eq!(
        app.state()
            .selected_resource()
            .map(|r| r.name.clone())
            .as_deref(),
        Some("db"),
        "the highlighted item stays"
    );
    assert!(screen(&mut app).contains("sorted by Name ↑"));

    // The same column again: the other way.
    press(&mut app, KeyCode::Char('s')).await;
    press(&mut app, KeyCode::Enter).await; // the picker starts on the current column
    assert_eq!(names(&app), ["web", "db", "cache", "Alpha"]);
    assert!(screen(&mut app).contains("sorted by Name ↓"));

    // Another column, with a filter: both apply.
    press(&mut app, KeyCode::Char('s')).await;
    press(&mut app, KeyCode::Char('4')).await; // Location
    app.state_mut().filter = "europe".into();
    assert_eq!(
        names(&app),
        ["Alpha", "web", "db"],
        "northeurope before westeurope, ties as loaded"
    );

    // 0: back to the order they came in; Esc closes without changing it.
    app.state_mut().filter.clear();
    press(&mut app, KeyCode::Char('s')).await;
    press(&mut app, KeyCode::Char('0')).await;
    assert_eq!(names(&app), ["web", "Alpha", "db", "cache"]);
    assert!(app.state().sort_spec(&ViewMode::ResourceList).is_none());
    press(&mut app, KeyCode::Char('s')).await;
    press(&mut app, KeyCode::Esc).await;
    assert!(app.state().sort_picker.is_none());
    assert_eq!(names(&app), ["web", "Alpha", "db", "cache"]);
}

#[tokio::test]
async fn s_does_nothing_where_there_is_nothing_to_sort() {
    let mut app = app();
    app.state_mut().view = ViewMode::ManagementGroupTree;
    press(&mut app, KeyCode::Char('s')).await;
    assert!(app.state().sort_picker.is_none(), "a tree keeps its shape");
}

#[test]
fn role_assignments_sort_by_pim_eligible_first() {
    use c9s_core::rbac::RoleAssignment;
    use c9s_core::sort::SortSpec;
    let mut core = App::offline(Cache::new(Path::new(":memory:")).unwrap());
    let state = core.state_mut();
    let at = |id: &str, eligible: bool| RoleAssignment {
        id: id.into(),
        scope: "/subscriptions/s1".into(),
        principal_id: id.into(),
        eligible,
        ..Default::default()
    };
    state.role_assignments = vec![at("a", false), at("b", true), at("c", false), at("d", true)];
    state.view = ViewMode::RoleAssignments;
    let ids = |s: &c9s_core::state::AppState| -> Vec<String> {
        s.role_assignment_rows()
            .into_iter()
            .map(|r| r.assignment.id)
            .collect()
    };
    let column = c9s_core::sort::columns(&ViewMode::RoleAssignments)
        .iter()
        .position(|c| *c == "PIM")
        .unwrap();
    state.sorts.insert(
        ViewMode::RoleAssignments,
        SortSpec {
            column,
            descending: true,
        },
    );
    assert_eq!(
        ids(state),
        ["b", "d", "a", "c"],
        "eligible first, ties in order"
    );
}
