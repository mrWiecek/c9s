//! `:ra` and `:role`: role assignments where you are, role definitions
//! (built-in and custom), and the drill-downs between them.

use c9s_core::cache::Cache;
use c9s_core::loader::{ChunkSink, FetchOutcome, Fetcher, LoadRequest, LoadedData};
use c9s_core::models::Subscription;
use c9s_core::rbac::{RoleAssignment, RoleDefinition};
use c9s_core::scope_tree::{GroupNode, ScopeTree, SubscriptionNode};
use c9s_core::state::ViewMode;
use c9s_core::App;
use c9s_tui::{ui, Event, TuiApp};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures::future::BoxFuture;
use futures::FutureExt;
use ratatui::{backend::TestBackend, Terminal};
use std::path::Path;
use std::sync::Arc;

const READER: &str = "acdd72a7-3385-48ef-bd42-f606fba81ae7";
const OWNER: &str = "8e3af657-a8ff-443c-a75c-2fe8c4bcb635";
const DEPLOYER: &str = "11111111-2222-3333-4444-555555555555";

fn mg(name: &str) -> String {
    format!("/providers/Microsoft.Management/managementGroups/{name}")
}

fn definition(guid: &str, name: &str, custom: bool) -> RoleDefinition {
    RoleDefinition {
        id: format!("/providers/Microsoft.Authorization/roleDefinitions/{guid}"),
        role_name: name.into(),
        role_type: if custom { "CustomRole" } else { "BuiltInRole" }.into(),
        description: format!("{name} role"),
        actions: vec!["*/read".into()],
        ..Default::default()
    }
}

fn assignment(n: u32, scope: &str, role: &str, principal_type: &str) -> RoleAssignment {
    RoleAssignment {
        id: format!("{scope}/providers/Microsoft.Authorization/roleAssignments/ra{n}"),
        scope: scope.into(),
        // Assignments name the role at their subscription; definitions at /.
        role_definition_id: format!(
            "/subscriptions/s1/providers/Microsoft.Authorization/roleDefinitions/{}",
            role.to_uppercase()
        ),
        principal_id: format!("principal-{n}"),
        principal_type: principal_type.into(),
        ..Default::default()
    }
}

struct Azure;

impl Fetcher for Azure {
    fn fetch(&self, request: LoadRequest, _sink: ChunkSink) -> BoxFuture<'static, FetchOutcome> {
        let data = match request {
            LoadRequest::RoleAssignments { .. } => Ok(LoadedData::RoleAssignments(vec![
                assignment(1, "/", OWNER, "User"),
                assignment(2, &mg("platform"), READER, "Group"),
                assignment(3, "/subscriptions/s1", DEPLOYER, "ServicePrincipal"),
                assignment(4, "/subscriptions/s1/resourceGroups/rg-app", READER, "User"),
                assignment(5, "/subscriptions/s2", OWNER, "User"),
            ])),
            LoadRequest::RoleDefinitions { .. } => Ok(LoadedData::RoleDefinitions(vec![
                definition(READER, "Reader", false),
                definition(OWNER, "Owner", false),
                definition(DEPLOYER, "App Deployer", true),
                definition(
                    "99999999-0000-0000-0000-000000000000",
                    "Unused Custom",
                    true,
                ),
            ])),
            // Two of the principals have names; the rest stay IDs.
            LoadRequest::Principals { ids } => Ok(LoadedData::Principals(
                ids.iter()
                    .filter_map(|id| {
                        let (name, kind, detail) = match id.as_str() {
                            "principal-1" => ("Ada Lovelace", "user", "ada@contoso.com"),
                            "principal-3" => ("deploy-pipeline", "servicePrincipal", "app-1"),
                            _ => return None,
                        };
                        Some(c9s_core::rbac::Principal {
                            id: id.clone(),
                            display_name: name.into(),
                            kind: kind.into(),
                            detail: detail.into(),
                        })
                    })
                    .collect(),
            )),
            // Ada (principal-1) is in the group principal-2, through a nested group.
            LoadRequest::PrincipalGroups { principal_id } => Ok(LoadedData::PrincipalGroups(
                principal_id.clone(),
                if principal_id == "principal-1" {
                    ["principal-2", "nested-group"]
                        .iter()
                        .map(|id| c9s_core::rbac::Principal {
                            id: id.to_string(),
                            display_name: format!("Group {id}"),
                            kind: "group".into(),
                            detail: String::new(),
                        })
                        .collect()
                } else {
                    vec![]
                },
            )),
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

fn setup() -> TuiApp {
    let mut core = App::with_fetcher(
        Cache::new(Path::new(":memory:")).unwrap(),
        Some(Arc::new(Azure)),
    );
    let state = core.state_mut();
    state.subscriptions = ["s1", "s2"]
        .iter()
        .map(|id| Subscription {
            id: id.to_string(),
            name: format!("sub-{id}"),
            state: "Enabled".into(),
            tenant_id: "root".into(),
        })
        .collect();
    state.scope_tree = Some(ScopeTree {
        root: "root".into(),
        root_display_name: "Tenant Root Group".into(),
        groups: vec![GroupNode {
            name: "platform".into(),
            display_name: "Platform".into(),
            parent: "root".into(),
        }],
        subscriptions: vec![
            SubscriptionNode {
                id: "s1".into(),
                display_name: "sub-s1".into(),
                parent: "platform".into(),
            },
            SubscriptionNode {
                id: "s2".into(),
                display_name: "sub-s2".into(),
                parent: "root".into(),
            },
        ],
    });
    TuiApp::new(core)
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
async fn role_assignments_where_you_are_then_the_role_they_grant() {
    let mut app = setup();
    let mut loads = app.take_load_receiver().unwrap();

    // `:ra sub-s1`: inherited from the root and Platform, here, and below.
    command(&mut app, "ra sub-s1").await;
    assert_eq!(app.state().view, ViewMode::RoleAssignments);
    // Assignments, definitions, then the principals' names.
    for _ in 0..3 {
        app.apply_load(loads.recv().await.unwrap());
    }
    let rows = app.state().role_assignment_rows();
    let summary: Vec<(&str, &str, &str)> = rows
        .iter()
        .map(|r| {
            (
                r.role_name.as_str(),
                r.scope_label.as_str(),
                r.origin_label.as_str(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("Owner", "tenant root (/)", "inherited"),
            ("Reader", "Platform", "inherited"),
            ("App Deployer", "sub-s1", "this scope"),
            ("Reader", "sub-s1 / rg-app", "below"),
        ],
        "another subscription's assignment is left out"
    );
    let text = screen(&mut app);
    for expected in [
        "Role assignments · sub-s1 (2 inherited, 1 here, 1 below)",
        "C App Deployer",
        "B Owner",
        "ServicePrincipal",
        "deploy-pipeline (app-1)",
        "Ada Lovelace (ada@contoso.com)",
        "principal-2",
    ] {
        assert!(text.contains(expected), "{expected}\n{text}");
    }

    // `/` finds principals by name or sign-in name too.
    app.state_mut().filter = "ada@".into();
    assert_eq!(app.state().role_assignment_rows().len(), 1);

    // `/custom`: only custom roles' assignments.
    app.state_mut().filter = "custom".into();
    assert_eq!(app.state().role_assignment_rows().len(), 1);
    app.state_mut().filter.clear();

    // Enter: the role it grants (the definition Resource Graph knows, by
    // GUID, though the assignment names it at the subscription).
    press(&mut app, KeyCode::Down).await;
    press(&mut app, KeyCode::Down).await; // App Deployer
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(app.state().view, ViewMode::ResourceDetail);
    let role = app.state().selected_resource_detail.clone().unwrap();
    assert_eq!(
        role.id,
        format!("/providers/Microsoft.Authorization/roleDefinitions/{DEPLOYER}")
    );
    assert_eq!(role.name, "App Deployer");

    // Esc, then d: the assignment itself.
    press(&mut app, KeyCode::Esc).await;
    assert_eq!(app.state().view, ViewMode::RoleAssignments);
    press(&mut app, KeyCode::Char('d')).await;
    let ra = app.state().selected_resource_detail.clone().unwrap();
    assert!(ra.id.ends_with("/roleAssignments/ra3"), "{}", ra.id);
    assert_eq!(ra.subscription_id, "s1");
}

#[tokio::test]
async fn roles_custom_first_with_their_assignments() {
    let mut app = setup();
    let mut loads = app.take_load_receiver().unwrap();
    command(&mut app, "role").await;
    assert_eq!(app.state().view, ViewMode::RoleDefinitions);
    app.apply_load(loads.recv().await.unwrap());
    app.apply_load(loads.recv().await.unwrap());
    let rows = app.state().role_definition_rows();
    let summary: Vec<(&str, usize)> = rows
        .iter()
        .map(|r| (r.definition.role_name.as_str(), r.assignments))
        .collect();
    assert_eq!(
        summary,
        [
            ("App Deployer", 1),
            ("Unused Custom", 0),
            ("Owner", 2),
            ("Reader", 2)
        ]
    );
    let text = screen(&mut app);
    for expected in [
        "Roles (2 custom, 2 built-in)",
        "custom",
        "unused",
        "2 assignments",
    ] {
        assert!(text.contains(expected), "{expected}\n{text}");
    }
    app.state_mut().filter = "built-in".into();
    assert_eq!(app.state().role_definition_rows().len(), 2);
    app.state_mut().filter.clear();

    // Enter on Owner: its assignments across the tenant.
    press(&mut app, KeyCode::Char('G')).await;
    press(&mut app, KeyCode::Up).await;
    press(&mut app, KeyCode::Enter).await;
    assert_eq!(app.state().view, ViewMode::RoleAssignments);
    let scopes: Vec<String> = app
        .state()
        .role_assignment_rows()
        .into_iter()
        .map(|r| r.assignment.scope)
        .collect();
    assert_eq!(scopes, ["/", "/subscriptions/s2"]);
    assert!(
        screen(&mut app).contains("Role assignments · whole tenant · role Owner (2 assignments)")
    );

    // Esc back to the roles; d: Owner's definition.
    press(&mut app, KeyCode::Esc).await;
    assert_eq!(app.state().view, ViewMode::RoleDefinitions);
    press(&mut app, KeyCode::Char('d')).await;
    assert_eq!(
        app.state().selected_resource_detail.as_ref().unwrap().name,
        "Owner"
    );
}

async fn settle(app: &mut TuiApp, loads: &mut c9s_core::loader::LoadReceiver) {
    while app.state().loading {
        app.apply_load(loads.recv().await.unwrap());
    }
}

#[tokio::test]
async fn a_principals_access_direct_and_through_groups() {
    let mut app = setup();
    let mut loads = app.take_load_receiver().unwrap();
    command(&mut app, "ra all").await;
    settle(&mut app, &mut loads).await;

    // p on Ada's Owner assignment: everything Ada can do.
    let ada = app
        .state()
        .role_assignment_rows()
        .iter()
        .position(|r| r.assignment.principal_id == "principal-1")
        .unwrap();
    app.state_mut().list_state.selected = ada;
    press(&mut app, KeyCode::Char('p')).await;
    settle(&mut app, &mut loads).await;
    let rows = app.state().role_assignment_rows();
    let summary: Vec<(&str, &str, &str)> = rows
        .iter()
        .map(|r| {
            (
                r.role_name.as_str(),
                r.scope_label.as_str(),
                r.origin_label.as_str(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("Owner", "tenant root (/)", "direct"),
            ("Reader", "Platform", "via Group principal-2"),
        ]
    );
    let text = screen(&mut app);
    assert!(
        text.contains(
            "Access · Ada Lovelace (ada@contoso.com) (1 direct, 1 via groups, member of 2 groups)"
        ),
        "{text}"
    );

    // Esc: back to the tenant's list, unfiltered, on Ada's row.
    press(&mut app, KeyCode::Esc).await;
    assert_eq!(app.state().view, ViewMode::RoleAssignments);
    assert_eq!(app.state().ra_principal, None);
    assert_eq!(app.state().role_assignment_rows().len(), 5);
    assert_eq!(
        app.state()
            .selected_role_assignment()
            .unwrap()
            .assignment
            .principal_id,
        "principal-1"
    );

    // :access by sign-in name finds the same principal.
    command(&mut app, "access ada@contoso.com").await;
    assert_eq!(app.state().ra_principal.as_deref(), Some("principal-1"));
    command(&mut app, "access nobody").await;
    assert!(app
        .state()
        .error
        .as_deref()
        .unwrap()
        .contains("No principal named nobody"));
}

#[tokio::test]
async fn pim_eligible_assignments_are_shown_and_counted() {
    let mut app = setup();
    let mut loads = app.take_load_receiver().unwrap();
    command(&mut app, "ra all").await;
    settle(&mut app, &mut loads).await;

    // Ada is eligible for Reader at the subscription until 2099; her group
    // (principal-2) is eligible for Owner, permanently.
    let mut eligible = assignment(7, "/subscriptions/s1", READER, "User");
    eligible.principal_id = "principal-1".into();
    eligible.eligible = true;
    eligible.ends = "2099-01-01T00:00:00Z".into();
    let mut via_group = assignment(8, &mg("platform"), OWNER, "Group");
    via_group.principal_id = "principal-2".into();
    via_group.eligible = true;
    let pim = [eligible, via_group];
    app.state_mut().role_assignments.extend(pim.clone());

    let text = screen(&mut app);
    assert!(text.contains("eligible until 2099-01-01"), "{text}");
    assert!(text.contains("2 PIM eligible"), "{text}");

    // /eligible keeps only them; /active the others.
    app.state_mut().filter = "eligible".into();
    assert_eq!(app.state().role_assignment_rows().len(), 2);
    app.state_mut().filter = "active".into();
    assert!(app
        .state()
        .role_assignment_rows()
        .iter()
        .all(|r| !r.assignment.eligible));
    app.state_mut().filter.clear();

    // d on an eligible one explains it instead of failing to load it.
    let at = app
        .state()
        .role_assignment_rows()
        .iter()
        .position(|r| r.assignment.id.ends_with("ra7"))
        .unwrap();
    app.state_mut().list_state.selected = at;
    press(&mut app, KeyCode::Char('d')).await;
    assert_eq!(app.state().view, ViewMode::RoleAssignments);
    let notice = app.state().notice.clone().unwrap_or_default();
    assert!(notice.contains("PIM eligible until 2099-01-01"), "{notice}");

    // Ada's access: her own eligibility and her group's.
    press(&mut app, KeyCode::Char('p')).await;
    settle(&mut app, &mut loads).await;
    // (The fake API's reload has no PIM rows: put them back.)
    app.state_mut().role_assignments.extend(pim);
    let rows = app.state().role_assignment_rows();
    let eligible: Vec<(&str, &str)> = rows
        .iter()
        .filter(|r| r.assignment.eligible)
        .map(|r| (r.role_name.as_str(), r.origin_label.as_str()))
        .collect();
    assert_eq!(
        eligible,
        [("Owner", "via Group principal-2"), ("Reader", "direct")]
    );
}
