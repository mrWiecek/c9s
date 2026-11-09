pub mod breadcrumb;
pub mod help;
pub mod resource_type;
pub mod search;
pub mod status_bar;
pub mod theme;
pub mod widgets;

use crate::TuiApp;
use anyhow::Result;
use breadcrumb::Breadcrumb;
use c9s_core::detail;
use c9s_core::provider::{Feature, Provider};
use c9s_core::scope_tree::TreeRowKind;
use c9s_core::state::{AppState, DetailTab, ViewMode};
use help::render_help;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, StatefulWidget, Tabs, Wrap},
    Frame,
};
use status_bar::StatusBar;
use theme::theme;
use widgets::{
    cell, list_item_spans, name_width, truncate_with_ellipsis, SelectableList, SelectableListState,
};

/// Draw the main UI
///
/// This is the main entry point for rendering the TUI. It divides the screen
/// into header, content, and footer sections, and delegates to view-specific
/// rendering functions.
pub fn draw(f: &mut Frame, app: &mut TuiApp) -> Result<()> {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header (breadcrumb)
            Constraint::Min(0),    // Main content
            Constraint::Length(3), // Status bar
            Constraint::Length(3), // Footer (keybindings/command)
        ])
        .split(f.area());

    theme::paint_background(f, f.area());
    draw_header(f, chunks[0], app);
    draw_content(f, chunks[1], app)?;
    draw_status_bar(f, chunks[2], app);
    draw_footer(f, chunks[3], app);

    // Autocomplete suggestions just above the command line
    if app.state().command_mode {
        draw_command_suggestions(f, chunks[3], app);
    }

    // Command palette
    if app.state().palette.is_some() {
        draw_palette(f, f.area(), app);
    }

    // `o` asking which page to open
    if app.state().portal_choice.is_some() {
        draw_portal_choice(f, app);
    }

    // `s` asking which column to sort by
    if app.state().sort_picker.is_some() {
        draw_sort_picker(f, app);
    }

    // Draw error overlay if present
    if app.state().error.is_some() {
        draw_error_overlay(f, app);
    }

    // Draw help screen if open
    if app.state().help_open {
        let mut scroll = app.state().help_scroll;
        render_help(f, f.area(), &mut scroll);
        app.state_mut().help_scroll = scroll;
    }

    clean_buffer(f.buffer_mut());
    Ok(())
}

/// The last step of every frame: no cell may carry a control character or
/// a bidi override, whatever widget put it there. Cloud data is untrusted
/// (any tenant member can name a resource or tag it), and an escape
/// sequence reaching the terminal could set its title, write the
/// clipboard or redraw the screen.
fn clean_buffer(buf: &mut ratatui::buffer::Buffer) {
    for cell in buf.content.iter_mut() {
        if cell.symbol().chars().any(c9s_core::safe_text::is_unsafe) {
            let cleaned = c9s_core::safe_text::clean(cell.symbol()).into_owned();
            cell.set_symbol(&cleaned);
        }
    }
}

/// Draw the header with breadcrumb navigation
fn draw_header(f: &mut Frame, area: Rect, app: &TuiApp) {
    let breadcrumb_line = Breadcrumb::render(app.state());

    let header = Paragraph::new(breadcrumb_line).block(Block::default().borders(Borders::ALL));

    f.render_widget(header, area);
}

/// Draw the status bar with connection and context information
fn draw_status_bar(f: &mut Frame, area: Rect, app: &TuiApp) {
    let status_line = StatusBar::render(app.state());

    let status = Paragraph::new(status_line).block(Block::default().borders(Borders::ALL));

    f.render_widget(status, area);
}

/// Draw the main content area based on current view
fn draw_content(f: &mut Frame, area: Rect, app: &mut TuiApp) -> Result<()> {
    match app.state().view {
        ViewMode::ManagementGroupTree => draw_tree(f, area, app),
        ViewMode::SubscriptionList => draw_subscription_list(f, area, app),
        ViewMode::ResourceGroupList => draw_resource_group_list(f, area, app),
        ViewMode::ResourceList => draw_resource_list(f, area, app),
        ViewMode::ResourceTypeSummary => draw_type_summary(f, area, app),
        ViewMode::TagSummary => draw_tag_summary(f, area, app),
        ViewMode::TenantList => draw_tenant_list(f, area, app),
        ViewMode::PolicyAssignments => draw_policy(f, area, app),
        ViewMode::PolicyNonCompliant => draw_non_compliant(f, area, app),
        ViewMode::ComplianceRanking => draw_compliance(f, area, app),
        ViewMode::ExemptionList => draw_exemptions(f, area, app),
        ViewMode::RoleAssignments => draw_role_assignments(f, area, app),
        ViewMode::RoleDefinitions => draw_role_definitions(f, area, app),
        ViewMode::GroupList => draw_groups(f, area, app),
        ViewMode::AppList => draw_apps(f, area, app),
        ViewMode::AppDetail => draw_app(f, area, app),
        ViewMode::GroupMembers | ViewMode::MemberOf => draw_members(f, area, app),
        ViewMode::ResourceDetail => draw_resource_detail(f, area, app),
        ViewMode::Logs => draw_logs(f, area, app),
    }
}

/// Title suffix for resource group and resource lists: " [ALL SUBSCRIPTIONS]"
/// or " [MG: name]" for cross-subscription lists, then the active filters.
fn scope_info(state: &AppState) -> String {
    let mut info = String::new();
    if state.cross_subscription_mode {
        let group = state
            .scope_group
            .as_deref()
            .and_then(|g| state.scope_tree.as_ref()?.label_of(g));
        info = match group {
            Some(name) => format!(" [MG: {}]", truncate_with_ellipsis(name, 40)),
            None => " [ALL SUBSCRIPTIONS]".to_string(),
        };
    }
    if !state.resource_query.is_empty() {
        info.push_str(&format!(" [{}]", state.resource_query));
    }
    info
}

/// What Enter on a tree row opens: the containers, or (GCP) resources.
fn opens_onto(provider: Provider) -> String {
    opens_onto_title(provider).to_lowercase()
}

fn opens_onto_title(provider: Provider) -> &'static str {
    if provider.supports(Feature::Containers) {
        provider.terms().containers
    } else {
        "Resources"
    }
}

/// Draw the management group tree
fn draw_tree(f: &mut Frame, area: Rect, app: &mut TuiApp) -> Result<()> {
    let rows = app.state().tree_rows();

    if rows.is_empty() {
        let state = app.state();
        let message = if state.loading && state.scope_tree.is_none() {
            format!("Loading {}...", state.provider.terms().tree.to_lowercase())
        } else if !state.filter.is_empty() {
            "Nothing in the tree matches the current filter.\n\nPress Esc to clear the filter."
                .to_string()
        } else {
            format!(
                "No {} loaded.\n\nPress 'r' to retry or ':sub' for {}.",
                state.provider.terms().tree.to_lowercase(),
                state.provider.terms().accounts.to_lowercase()
            )
        };
        let content = Paragraph::new(message)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(app.state().provider.terms().tree),
            )
            .style(Style::default().fg(theme().subtle))
            .wrap(Wrap { trim: true });
        f.render_widget(content, area);
        return Ok(());
    }

    let viewport = area.height.saturating_sub(2) as usize;
    let mut list_state = app.state().list_state.clone();
    let visible = list_state.visible_range(rows.len(), viewport);
    let accounts = app.state().provider.terms().accounts.to_lowercase();
    let items: Vec<_> = rows[visible.clone()]
        .iter()
        .map(|row| {
            let indent = "  ".repeat(row.depth);
            let marker = match (row.has_children, row.expanded) {
                (false, _) => "  ",
                (true, true) => "▾ ",
                (true, false) => "▸ ",
            };
            let (icon, color) = match row.kind {
                TreeRowKind::ManagementGroup => ("◆ ", theme().accent),
                TreeRowKind::Unplaced => ("◇ ", theme().muted),
                TreeRowKind::Subscription => match row.state.as_deref() {
                    Some(s) if s.eq_ignore_ascii_case("Disabled") => ("● ", theme().error),
                    Some(s) if s.eq_ignore_ascii_case("Warned") => ("● ", theme().highlight),
                    _ => ("● ", theme().ok),
                },
            };
            let width = 60usize.saturating_sub(indent.chars().count() + 4).max(10);
            let label = truncate_with_ellipsis(&row.label, width);
            let detail = if row.is_group() {
                format!("{} {accounts}", row.subscription_count)
            } else {
                match row.state.as_deref() {
                    Some(s) if !s.eq_ignore_ascii_case("Enabled") => format!("{s}  {}", row.key),
                    _ => row.key.clone(),
                }
            };
            let compliance = if row.is_group() {
                app.state().group_compliance(&row.key).map(|s| s.resources)
            } else {
                app.state().subscription_compliance(&row.key)
            };
            let (compliance, compliance_style) = compliance_cell(compliance);
            let label_style = if row.is_group() {
                Style::default()
                    .fg(theme().text)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme().text)
            };
            list_item_spans(vec![
                Span::raw(indent),
                Span::styled(marker, Style::default().fg(theme().muted)),
                Span::styled(icon, Style::default().fg(color)),
                Span::styled(format!("{label:<width$}  "), label_style),
                Span::styled(cell(&compliance, COMPLIANCE_WIDTH), compliance_style),
                Span::styled(detail, Style::default().fg(theme().muted)),
            ])
        })
        .collect();

    let total = app
        .state()
        .scope_tree
        .as_ref()
        .map_or(0, |t| t.groups.len() + 1);
    let title = if app.state().filter_mode {
        format!(
            "{} ({total}) - Filter: {}_",
            app.state().provider.terms().tree,
            app.state().filter
        )
    } else if !app.state().filter.is_empty() {
        format!(
            "{} ({total}) - Filtered by: {}",
            app.state().provider.terms().tree,
            app.state().filter
        )
    } else {
        format!(
            "{} ({total}) - Enter opens {}, Space expands",
            app.state().provider.terms().tree,
            opens_onto(app.state().provider)
        )
    };

    let list = SelectableList::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(theme().selection());
    let mut widget_state = SelectableListState {
        selected: list_state.selected - visible.start,
        offset: 0,
    };
    list.render(area, f.buffer_mut(), &mut widget_state);
    app.state_mut().list_state = list_state;
    Ok(())
}

/// Draw subscription list view
fn draw_subscription_list(f: &mut Frame, area: Rect, app: &mut TuiApp) -> Result<()> {
    let filtered_subscriptions = app.state().filtered_subscriptions();

    if filtered_subscriptions.is_empty() {
        let message = if app.state().loading {
            "Loading subscriptions...\n\nPlease wait while we fetch your Azure subscriptions."
        } else if !app.state().filter.is_empty() {
            "No subscriptions match the current filter.\n\nPress Esc to clear the filter."
        } else {
            "No subscriptions loaded.\n\nPress 'r' to refresh or check your Azure authentication."
        };

        let content = Paragraph::new(message)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(app.state().provider.terms().accounts),
            )
            .style(Style::default().fg(theme().subtle))
            .wrap(Wrap { trim: true });
        f.render_widget(content, area);
        return Ok(());
    }

    // Create list items with color-coded state
    // Only build widgets for the rows that fit on screen (minus the borders).
    let viewport = area.height.saturating_sub(2) as usize;
    let mut list_state = app.state().list_state.clone();
    let visible = list_state.visible_range(filtered_subscriptions.len(), viewport);
    // Fixed columns: state (12) and the subscription ID (36).
    let name_col = name_width(area.width, 12 + 36, 60);
    let items: Vec<_> = filtered_subscriptions[visible.clone()]
        .iter()
        .map(|sub| {
            let state_color = match sub.state.as_str() {
                "Enabled" => theme().ok,
                "Disabled" => theme().error,
                "Warned" => theme().highlight,
                _ => theme().subtle,
            };

            list_item_spans(vec![
                Span::styled(
                    cell(&sub.name, name_col),
                    Style::default()
                        .fg(theme().text)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(cell(&sub.state, 12), Style::default().fg(state_color)),
                Span::styled(sub.id.clone(), Style::default().fg(theme().muted)),
            ])
        })
        .collect();

    // Create title with filter indicator
    let title = if app.state().filter_mode {
        format!(
            "{} ({}) - Filter: {}_",
            app.state().provider.terms().accounts,
            filtered_subscriptions.len(),
            app.state().filter
        )
    } else if !app.state().filter.is_empty() {
        format!(
            "{} ({}) - Filtered by: {}",
            app.state().provider.terms().accounts,
            filtered_subscriptions.len(),
            app.state().filter
        )
    } else {
        format!(
            "{} ({}) - Press Enter to select",
            app.state().provider.terms().accounts,
            filtered_subscriptions.len()
        )
    };

    // Create the list widget
    let list = SelectableList::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(theme().selection());

    // Items are already windowed: render from the top with a relative selection.
    let mut widget_state = SelectableListState {
        selected: list_state.selected - visible.start,
        offset: 0,
    };
    list.render(area, f.buffer_mut(), &mut widget_state);

    // Persist the scroll offset so scrolling up doesn't snap the selection.
    app.state_mut().list_state = list_state;

    Ok(())
}

/// Draw resource group list view
fn draw_resource_group_list(f: &mut Frame, area: Rect, app: &mut TuiApp) -> Result<()> {
    let filtered_resource_groups = app.state().filtered_resource_groups();

    if filtered_resource_groups.is_empty() {
        let message = if app.state().loading {
            "Loading resource groups...\n\nPlease wait while we fetch resource groups from Azure."
        } else if !app.state().filter.is_empty() {
            "No resource groups match the current filter.\n\nPress Esc to clear the filter."
        } else {
            "No resource groups loaded.\n\nSelect a subscription to view resource groups.\nPress 'r' to refresh."
        };

        let content = Paragraph::new(message)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(app.state().provider.terms().containers),
            )
            .style(Style::default().fg(theme().subtle))
            .wrap(Wrap { trim: true });
        f.render_widget(content, area);
        return Ok(());
    }

    // Create list items for resource groups
    // Only build widgets for the rows that fit on screen (minus the borders).
    let viewport = area.height.saturating_sub(2) as usize;
    let mut list_state = app.state().list_state.clone();
    let visible = list_state.visible_range(filtered_resource_groups.len(), viewport);
    // Fixed columns: location (20) and the tag count (~10).
    let name_col = name_width(area.width, 20 + 10, 80);
    let items: Vec<_> = filtered_resource_groups[visible.clone()]
        .iter()
        .map(|rg| {
            // Display name, location, and tag count
            let tag_count = rg.tags.len();
            let tag_info = if tag_count > 0 {
                format!("{} tags", tag_count)
            } else {
                "no tags".to_string()
            };

            list_item_spans(vec![
                Span::styled(
                    cell(&rg.name, name_col),
                    Style::default()
                        .fg(theme().text)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    cell(&rg.location, 20),
                    Style::default().fg(theme().highlight),
                ),
                Span::styled(tag_info, Style::default().fg(theme().muted)),
            ])
        })
        .collect();

    // Create title with filter indicator and scope
    let scope_info = scope_info(app.state());

    let title = if app.state().filter_mode {
        format!(
            "{} ({}){}  - Filter: {}_",
            app.state().provider.terms().containers,
            filtered_resource_groups.len(),
            scope_info,
            app.state().filter
        )
    } else if !app.state().filter.is_empty() {
        format!(
            "{} ({}){}  - Filtered by: {}",
            app.state().provider.terms().containers,
            filtered_resource_groups.len(),
            scope_info,
            app.state().filter
        )
    } else {
        format!(
            "{} ({}){}  - Press Enter to view resources",
            app.state().provider.terms().containers,
            filtered_resource_groups.len(),
            scope_info
        )
    };

    // Create the list widget
    let list = SelectableList::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(theme().selection());

    // Items are already windowed: render from the top with a relative selection.
    let mut widget_state = SelectableListState {
        selected: list_state.selected - visible.start,
        offset: 0,
    };
    list.render(area, f.buffer_mut(), &mut widget_state);

    // Persist the scroll offset so scrolling up doesn't snap the selection.
    app.state_mut().list_state = list_state;

    Ok(())
}

/// Draw the resource counts per type (`:type`)
fn draw_type_summary(f: &mut Frame, area: Rect, app: &mut TuiApp) -> Result<()> {
    let rows = app.state().type_summary();
    let state = app.state();
    let scope = scope_info(state);

    if rows.is_empty() {
        let message = if state.loading {
            "Loading resources...".to_string()
        } else if !state.filter.is_empty() {
            "No resource types match the current filter.\n\nPress Esc to clear the filter."
                .to_string()
        } else {
            "No resources loaded.\n\nPress 'r' to refresh.".to_string()
        };
        let content = Paragraph::new(message)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("Resource Types{scope}")),
            )
            .style(Style::default().fg(theme().subtle))
            .wrap(Wrap { trim: true });
        f.render_widget(content, area);
        return Ok(());
    }

    let total: usize = rows.iter().map(|(_, n)| n).sum();
    let count_width = rows[0].1.to_string().len().max(5);
    let viewport = area.height.saturating_sub(2) as usize;
    let mut list_state = state.list_state.clone();
    let visible = list_state.visible_range(rows.len(), viewport);
    let items: Vec<_> = rows[visible.clone()]
        .iter()
        .map(|(resource_type, count)| {
            list_item_spans(vec![
                Span::styled(
                    format!("{count:>count_width$}  "),
                    Style::default()
                        .fg(theme().highlight)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    cell(&resource_type::short_label(resource_type), 20),
                    Style::default()
                        .fg(theme().text)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(resource_type.clone(), Style::default().fg(theme().muted)),
            ])
        })
        .collect();

    let title = if state.filter_mode {
        format!(
            "Resource Types ({}){scope} - Filter: {}_",
            rows.len(),
            state.filter
        )
    } else {
        format!(
            "Resource Types ({}, {total} resources){scope} - Enter lists them",
            rows.len()
        )
    };
    let list = SelectableList::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(theme().selection());
    let mut widget_state = SelectableListState {
        selected: list_state.selected - visible.start,
        offset: 0,
    };
    list.render(area, f.buffer_mut(), &mut widget_state);
    app.state_mut().list_state = list_state;
    Ok(())
}

/// Draw the policy assignments that apply at a scope (`:policy`)
/// Width of the compliance column in the tree and `:compliance`.
const COMPLIANCE_WIDTH: usize = 30;

/// A scope's compliance as "92.4% · 140 non-compliant", coloured: green when
/// fully compliant, red below 80%, else yellow. Empty while not loaded.
fn compliance_cell(c: Option<c9s_core::policy::Compliance>) -> (String, Style) {
    let t = theme();
    match c {
        None => (String::new(), Style::default()),
        Some(c) if c.total == 0 => ("not evaluated".into(), Style::default().fg(t.muted)),
        Some(c) => {
            let compliant = c.total - c.non_compliant.min(c.total);
            let percent = 100.0 * compliant as f64 / c.total as f64;
            let color = if c.non_compliant == 0 {
                t.ok
            } else if percent < 80.0 {
                t.error
            } else {
                t.highlight
            };
            let text = if c.non_compliant == 0 {
                format!("{percent:.1}% · all {} compliant", grouped(c.total))
            } else {
                format!("{percent:.1}% · {} non-compliant", grouped(c.non_compliant))
            };
            (text, Style::default().fg(color))
        }
    }
}

/// "C " for a custom role (highlighted), "B " for built-in, blank if unknown.
fn role_kind_span(custom: Option<bool>) -> Span<'static> {
    let t = theme();
    match custom {
        Some(true) => Span::styled(
            "C ",
            Style::default()
                .fg(t.highlight)
                .add_modifier(Modifier::BOLD),
        ),
        Some(false) => Span::styled("B ", Style::default().fg(t.muted)),
        None => Span::raw("  "),
    }
}

/// A message in a bordered box, for an empty list.
fn draw_empty(f: &mut Frame, area: Rect, title: String, message: String) {
    let content = Paragraph::new(message)
        .block(Block::default().borders(Borders::ALL).title(title))
        .style(Style::default().fg(theme().subtle))
        .wrap(Wrap { trim: true });
    f.render_widget(content, area);
}

/// A list of rows in a bordered box with the selection, as every list view
/// draws it.
fn draw_list(
    f: &mut Frame,
    area: Rect,
    app: &mut TuiApp,
    title: String,
    rows: usize,
    item: impl Fn(usize) -> widgets::ListItem<'static>,
) {
    let viewport = area.height.saturating_sub(2) as usize;
    let mut list_state = app.state().list_state.clone();
    let visible = list_state.visible_range(rows, viewport);
    let items: Vec<_> = visible.clone().map(item).collect();
    let list = SelectableList::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(theme().selection());
    let mut widget_state = SelectableListState {
        selected: list_state.selected - visible.start,
        offset: 0,
    };
    list.render(area, f.buffer_mut(), &mut widget_state);
    app.state_mut().list_state = list_state;
}

/// `:app`: registrations and enterprise applications, one row per app.
fn draw_apps(f: &mut Frame, area: Rect, app: &mut TuiApp) -> Result<()> {
    use c9s_core::apps::AppKind;
    let rows = app.state().app_rows();
    let state = app.state();
    let t = theme();
    if rows.is_empty() {
        let message = if state.loading {
            "Loading applications…".to_string()
        } else if !state.filter.is_empty() {
            "No applications match the current filter.\n\nPress Esc to clear the filter.".into()
        } else {
            "No applications loaded (they come from Microsoft Graph). Press r to retry.".into()
        };
        draw_empty(f, area, "Applications".into(), message);
        return Ok(());
    }
    let count = |k: AppKind| rows.iter().filter(|r| r.kind == k).count();
    let expiring = rows
        .iter()
        .filter(|r| {
            r.soonest.as_ref().is_some_and(|(e, _)| {
                matches!(e, c9s_core::policy::Expiry::Expired(_))
                    || matches!(e, c9s_core::policy::Expiry::In(d) if *d <= c9s_core::policy::EXPIRY_WARNING_DAYS)
            })
        })
        .count();
    let microsoft = if state.show_microsoft_apps {
        "m hide Microsoft's"
    } else {
        "m show Microsoft's"
    };
    let title = if state.filter_mode {
        format!("Applications ({}) - Filter: {}_", rows.len(), state.filter)
    } else {
        format!(
            "Applications ({} registered, {} external, {} managed identities, {expiring} with \
             credentials expired or expiring) · Enter open, p access, {microsoft}",
            count(AppKind::Registered),
            count(AppKind::External) + count(AppKind::Microsoft),
            count(AppKind::ManagedIdentity),
        )
    };
    let name_col = name_width(area.width, 20 + 38 + 36, 44);
    draw_list(f, area, app, title, rows.len(), |i| {
        let r = &rows[i];
        let (expiry, style) = match &r.soonest {
            Some((e, c)) => (
                format!("{} ({})", c9s_core::policy::expiry_label(*e), c.kind),
                expiry_span(*e, 0).style,
            ),
            None => ("no credentials".into(), Style::default().fg(t.muted)),
        };
        let kind_style = match r.kind {
            AppKind::Registered => Style::default().fg(t.special),
            AppKind::ManagedIdentity => Style::default().fg(t.accent),
            _ => Style::default().fg(t.muted),
        };
        list_item_spans(vec![
            Span::styled(
                cell(&r.name, name_col),
                Style::default().fg(t.text).add_modifier(Modifier::BOLD),
            ),
            Span::styled(cell(r.kind_label(), 20), kind_style),
            Span::styled(cell(&r.app_id, 38), Style::default().fg(t.muted)),
            Span::styled(cell(&expiry, 36), style),
        ])
    });
    Ok(())
}

/// One application: tabs over its registration and enterprise application.
fn draw_app(f: &mut Frame, area: Rect, app: &mut TuiApp) -> Result<()> {
    use c9s_core::app_view::Tone;
    use c9s_core::state::AppTab;
    let state = app.state();
    let Some(row) = state.app_open.clone() else {
        draw_empty(
            f,
            area,
            "Application".into(),
            "No application selected.".into(),
        );
        return Ok(());
    };
    let t = theme();
    let tone = |tone: Tone| match tone {
        Tone::Heading => Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        Tone::Label => Style::default().fg(t.highlight),
        Tone::Text => Style::default().fg(t.text),
        Tone::Muted => Style::default().fg(t.muted),
        Tone::Accent => Style::default().fg(t.special),
        Tone::Good => Style::default().fg(t.ok),
        Tone::Warn => Style::default().fg(t.highlight),
        Tone::Bad => Style::default().fg(t.error).add_modifier(Modifier::BOLD),
    };
    let mut lines: Vec<Line> = c9s_core::app_view::app_lines(state)
        .into_iter()
        .map(|l| {
            Line::from(
                l.into_iter()
                    .map(|(text, k)| Span::styled(text, tone(k)))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    highlight_search(&mut lines, state);
    // The Overview's selected field: y and c copy it.
    if state.app_tab == AppTab::Overview {
        let fields = c9s_core::app_view::overview_fields(state);
        if let Some(field) = fields.get(state.app_field.min(fields.len().saturating_sub(1))) {
            if let Some(line) = lines.get_mut(field.line) {
                *line = std::mem::take(line).patch_style(t.selection());
            }
        }
    }
    let loading = if state.open_app_detail().is_none() && state.loading {
        " · loading details…"
    } else {
        ""
    };
    let block = Block::default().borders(Borders::ALL).title(format!(
        "{} · {}{loading} · ↑↓ field, y/c copy it, / search, o Portal, p access",
        truncate_with_ellipsis(&row.name, 60),
        row.kind_label()
    ));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(inner);
    let titles: Vec<Line> = AppTab::ALL
        .iter()
        .enumerate()
        .map(|(i, tab)| Line::from(format!("{} {}", i + 1, tab.title())))
        .collect();
    let tabs = Tabs::new(titles)
        .select(state.app_tab.index())
        .style(Style::default().fg(t.subtle))
        .highlight_style(t.selection())
        .divider("│");
    f.render_widget(tabs, parts[0]);
    let viewport = parts[1].height as usize;
    let mut scroll = state.detail_scroll;
    // Bring a search match or the selected field into view.
    if let Some(line) = state.detail_scroll_to {
        if line < scroll || line >= scroll + viewport {
            scroll = line.saturating_sub(viewport / 2);
        }
    }
    let scroll = scroll.min(lines.len().saturating_sub(viewport));
    let content = Paragraph::new(lines).scroll((scroll.min(u16::MAX as usize) as u16, 0));
    f.render_widget(content, parts[1]);
    let state = app.state_mut();
    state.detail_scroll = scroll;
    state.detail_scroll_to = None;
    Ok(())
}

/// `:group`: directory groups.
fn draw_groups(f: &mut Frame, area: Rect, app: &mut TuiApp) -> Result<()> {
    let rows: Vec<c9s_core::rbac::Group> = app.state().group_rows().into_iter().cloned().collect();
    let state = app.state();
    let t = theme();
    if rows.is_empty() {
        let message = if state.loading {
            "Loading groups…".to_string()
        } else if !state.filter.is_empty() {
            "No groups match the current filter.\n\nPress Esc to clear the filter.".into()
        } else {
            "No groups loaded (they come from Microsoft Graph). Press r to retry.".into()
        };
        draw_empty(f, area, "Groups".into(), message);
        return Ok(());
    }
    let name_col = name_width(area.width, 16 + 10 + 4 + 34, 48);
    let title = if state.filter_mode {
        format!("Groups ({}) - Filter: {}_", rows.len(), state.filter)
    } else {
        let dynamic = rows.iter().filter(|g| g.dynamic).count();
        let assignable = rows.iter().filter(|g| g.role_assignable).count();
        format!(
            "Groups ({}, {dynamic} dynamic, {assignable} role-assignable) · R role-assignable · \
             Enter members, p access",
            rows.len()
        )
    };
    draw_list(f, area, app, title, rows.len(), |i| {
        let g = &rows[i];
        list_item_spans(vec![
            Span::styled(
                if g.role_assignable { "R " } else { "  " },
                Style::default()
                    .fg(t.highlight)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                cell(&g.display_name, name_col),
                Style::default().fg(t.text).add_modifier(Modifier::BOLD),
            ),
            Span::styled(cell(g.kind_label(), 16), Style::default().fg(t.accent)),
            Span::styled(
                cell(if g.dynamic { "dynamic" } else { "assigned" }, 10),
                Style::default().fg(if g.dynamic { t.special } else { t.muted }),
            ),
            Span::styled(cell(&g.mail, 34), Style::default().fg(t.muted)),
            Span::styled(g.description.clone(), Style::default().fg(t.muted)),
        ])
    });
    Ok(())
}

/// A directory principal's kind, short: "user", "group", "app", "device".
fn kind_short(kind: &str) -> &str {
    match kind {
        "servicePrincipal" => "app",
        k => k,
    }
}

/// A group's members, or the groups a principal belongs to.
fn draw_members(f: &mut Frame, area: Rect, app: &mut TuiApp) -> Result<()> {
    let state = app.state();
    let t = theme();
    let members = state.view == ViewMode::GroupMembers;
    let (rows, whose) = if members {
        (
            state.member_rows(),
            state
                .members_group
                .as_deref()
                .map(|g| state.principal_label(g)),
        )
    } else {
        (
            state.member_of_rows(),
            state.member_of.as_deref().map(|p| state.principal_label(p)),
        )
    };
    let whose = whose.unwrap_or_default();
    let heading = if members {
        let mode = if state.members_transitive {
            "nested included"
        } else {
            "direct"
        };
        format!("Members · {whose} · {mode}")
    } else {
        format!("Member of · {whose} · nested included")
    };
    let Some(rows) = rows.filter(|r| !r.is_empty()) else {
        let message = match (state.loading, state.filter.is_empty(), members) {
            (true, _, _) => "Loading…".to_string(),
            (false, false, _) => {
                "Nothing matches the current filter.\n\nPress Esc to clear the filter.".into()
            }
            (false, true, true) => "No members.".into(),
            (false, true, false) => "Not a member of any group.".into(),
        };
        draw_empty(f, area, heading, message);
        return Ok(());
    };
    let mut counts: Vec<(String, usize)> = Vec::new();
    for r in &rows {
        let kind = kind_short(&r.kind).to_string();
        match counts.iter_mut().find(|(k, _)| *k == kind) {
            Some((_, n)) => *n += 1,
            None => counts.push((kind, 1)),
        }
    }
    let counts = counts
        .iter()
        .map(|(k, n)| format!("{n} {k}{}", if *n == 1 { "" } else { "s" }))
        .collect::<Vec<_>>()
        .join(", ");
    let keys = if members {
        "Enter open, t nested on/off, p access"
    } else {
        "Enter members, p access"
    };
    let title = if state.filter_mode {
        format!("{heading} ({}) - Filter: {}_", rows.len(), state.filter)
    } else {
        format!("{heading} ({counts}) · {keys}")
    };
    let name_col = name_width(area.width, 8 + 40, 48);
    draw_list(f, area, app, title, rows.len(), |i| {
        let p = &rows[i];
        list_item_spans(vec![
            Span::styled(cell(kind_short(&p.kind), 8), Style::default().fg(t.accent)),
            Span::styled(
                cell(&p.display_name, name_col),
                Style::default().fg(t.text).add_modifier(Modifier::BOLD),
            ),
            Span::styled(cell(&p.detail, 40), Style::default().fg(t.muted)),
            Span::styled(p.id.clone(), Style::default().fg(t.muted)),
        ])
    });
    Ok(())
}

/// `:ra`: role assignments that matter at a scope.
fn draw_role_assignments(f: &mut Frame, area: Rect, app: &mut TuiApp) -> Result<()> {
    use c9s_core::rbac::Reach;
    let rows = app.state().role_assignment_rows();
    let state = app.state();
    let t = theme();
    let scope_name = state
        .ra_scope
        .as_ref()
        .map(|s| state.scope_label(&s.arm_id()))
        .unwrap_or_else(|| "whole tenant".into());
    let role_note = state
        .ra_role
        .as_ref()
        .map(|guid| {
            let name = state
                .role_definition(guid)
                .map_or(guid.clone(), |d| d.role_name.clone());
            format!(" · role {name}")
        })
        .unwrap_or_default();
    // One principal's access: who, and through how many groups.
    let principal = state.ra_principal.as_ref().map(|id| {
        let p = state.principals.get(id);
        let name = p.map_or(id.clone(), |p| p.display_name.clone());
        let detail = p
            .map(|p| p.detail.clone())
            .filter(|d| !d.is_empty())
            .map(|d| format!(" ({d})"))
            .unwrap_or_default();
        (name + &detail, state.principal_groups.get(id).map(Vec::len))
    });
    let heading = match &principal {
        Some((who, _)) => format!("Access · {who}"),
        None => format!("Role assignments · {scope_name}{role_note}"),
    };
    if rows.is_empty() {
        let message = if state.loading {
            "Loading role assignments…".to_string()
        } else if !state.filter.is_empty() {
            "No role assignments match the current filter.\n\nPress Esc to clear the filter.".into()
        } else {
            format!("No role assignments at {scope_name}.")
        };
        draw_empty(f, area, heading, message);
        return Ok(());
    }

    // Columns as wide as their values (within limits); the scope comes last
    // and gets the rest of the line, so it is shown whole.
    let principal_text = |r: &c9s_core::state::RoleAssignmentRow| {
        if r.principal_detail.is_empty() {
            r.principal_name.clone()
        } else {
            format!("{} ({})", r.principal_name, r.principal_detail)
        }
    };
    let widest = |values: &mut dyn Iterator<Item = usize>, min: usize, max: usize| {
        values.max().unwrap_or(0).clamp(min, max) + 2
    };
    let name_col = widest(
        &mut rows.iter().map(|r| r.role_name.chars().count()),
        10,
        40,
    );
    let type_w = widest(
        &mut rows
            .iter()
            .map(|r| r.assignment.principal_type.chars().count()),
        4,
        16,
    );
    let principal_w = widest(
        &mut rows.iter().map(|r| principal_text(r).chars().count()),
        10,
        56,
    );
    let origin_w = widest(
        &mut rows.iter().map(|r| r.origin_label.chars().count()),
        0,
        30,
    );
    // PIM: eligible assignments say so (and until when); none: no column.
    let pim_w = if rows.iter().any(|r| r.assignment.eligible) {
        widest(
            &mut rows
                .iter()
                .map(|r| r.assignment.pim_label().chars().count()),
            8,
            26,
        )
    } else {
        0
    };
    let viewport = area.height.saturating_sub(2) as usize;
    let mut list_state = state.list_state.clone();
    let visible = list_state.visible_range(rows.len(), viewport);
    let items: Vec<_> = rows[visible.clone()]
        .iter()
        .map(|r| {
            let a = &r.assignment;
            let origin_style = match r.reach {
                Reach::Inherited(_) => Style::default().fg(t.muted),
                Reach::Here => Style::default().fg(t.special),
                Reach::Below => Style::default().fg(t.accent),
            };
            list_item_spans(vec![
                role_kind_span(r.custom),
                Span::styled(
                    cell(&r.role_name, name_col),
                    Style::default().fg(t.text).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    cell(&a.pim_label(), pim_w),
                    Style::default().fg(t.highlight),
                ),
                Span::styled(
                    cell(&a.principal_type, type_w),
                    Style::default().fg(t.accent),
                ),
                Span::styled(
                    cell(&principal_text(r), principal_w),
                    Style::default().fg(t.text),
                ),
                Span::styled(cell(&r.origin_label, origin_w), origin_style),
                Span::styled(r.scope_label.clone(), Style::default().fg(t.muted)),
            ])
        })
        .collect();

    let count = |reach: fn(&Reach) -> bool| rows.iter().filter(|r| reach(&r.reach)).count();
    let summary = if let Some((_, groups)) = &principal {
        let direct = rows.iter().filter(|r| r.via.is_none()).count();
        let groups = match groups {
            Some(n) => format!("member of {n} group{}", if *n == 1 { "" } else { "s" }),
            None => "groups loading…".into(),
        };
        format!(
            "{direct} direct, {} via groups, {groups}",
            rows.len() - direct
        )
    } else if state.ra_scope.is_some() {
        format!(
            "{} inherited, {} here, {} below",
            count(|r| matches!(r, Reach::Inherited(_))),
            count(|r| *r == Reach::Here),
            count(|r| *r == Reach::Below)
        )
    } else {
        format!(
            "{} assignment{}",
            rows.len(),
            if rows.len() == 1 { "" } else { "s" }
        )
    };
    let eligible = rows.iter().filter(|r| r.assignment.eligible).count();
    let summary = if eligible > 0 {
        format!("{summary}; {eligible} PIM eligible")
    } else {
        summary
    };
    let title = if state.filter_mode {
        format!("{heading} ({}) - Filter: {}_", rows.len(), state.filter)
    } else {
        format!("{heading} ({summary}) · p: all this principal's access · Enter role · d details")
    };
    let list = SelectableList::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(theme().selection());
    let mut widget_state = SelectableListState {
        selected: list_state.selected - visible.start,
        offset: 0,
    };
    list.render(area, f.buffer_mut(), &mut widget_state);
    app.state_mut().list_state = list_state;
    Ok(())
}

/// `:role`: role definitions, custom first.
fn draw_role_definitions(f: &mut Frame, area: Rect, app: &mut TuiApp) -> Result<()> {
    let rows = app.state().role_definition_rows();
    let state = app.state();
    let t = theme();
    let heading = "Roles".to_string();
    if rows.is_empty() {
        let message = if state.loading {
            "Loading role definitions…".to_string()
        } else if !state.filter.is_empty() {
            "No roles match the current filter.\n\nPress Esc to clear the filter.".into()
        } else {
            "No role definitions loaded. Press r to retry.".into()
        };
        draw_empty(f, area, heading, message);
        return Ok(());
    }
    let name_col = name_width(area.width, 2 + 10 + 14, 50);
    let viewport = area.height.saturating_sub(2) as usize;
    let mut list_state = state.list_state.clone();
    let visible = list_state.visible_range(rows.len(), viewport);
    let items: Vec<_> = rows[visible.clone()]
        .iter()
        .map(|r| {
            let d = &r.definition;
            let assignments = match r.assignments {
                0 => "unused".to_string(),
                1 => "1 assignment".to_string(),
                n => format!("{} assignments", grouped(n as u64)),
            };
            list_item_spans(vec![
                role_kind_span(Some(d.is_custom())),
                Span::styled(
                    cell(&d.role_name, name_col),
                    Style::default().fg(t.text).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    cell(d.kind_label(), 10),
                    Style::default().fg(if d.is_custom() { t.highlight } else { t.muted }),
                ),
                Span::styled(
                    cell(&assignments, 18),
                    Style::default().fg(if r.assignments == 0 {
                        t.muted
                    } else {
                        t.accent
                    }),
                ),
                Span::styled(d.description.clone(), Style::default().fg(t.muted)),
            ])
        })
        .collect();
    let custom = rows.iter().filter(|r| r.definition.is_custom()).count();
    let title = if state.filter_mode {
        format!("{heading} ({}) - Filter: {}_", rows.len(), state.filter)
    } else {
        format!(
            "{heading} ({custom} custom, {} built-in) · Enter assignments, d details",
            rows.len() - custom
        )
    };
    let list = SelectableList::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(theme().selection());
    let mut widget_state = SelectableListState {
        selected: list_state.selected - visible.start,
        offset: 0,
    };
    list.render(area, f.buffer_mut(), &mut widget_state);
    app.state_mut().list_state = list_state;
    Ok(())
}

/// An exemption's expiry, coloured: red when expired, yellow within
/// [`EXPIRY_WARNING_DAYS`](c9s_core::policy::EXPIRY_WARNING_DAYS).
fn expiry_span(expiry: c9s_core::policy::Expiry, width: usize) -> Span<'static> {
    use c9s_core::policy::{expiry_label, Expiry, EXPIRY_WARNING_DAYS};
    let t = theme();
    let style = match expiry {
        Expiry::Expired(_) => Style::default().fg(t.error).add_modifier(Modifier::BOLD),
        Expiry::In(d) if d <= EXPIRY_WARNING_DAYS => Style::default().fg(t.highlight),
        Expiry::In(_) => Style::default().fg(t.ok),
        Expiry::Never => Style::default().fg(t.muted),
    };
    Span::styled(cell(&expiry_label(expiry), width), style)
}

/// `:exemption`: exemptions at or below a scope, soonest to expire first.
fn draw_exemptions(f: &mut Frame, area: Rect, app: &mut TuiApp) -> Result<()> {
    use c9s_core::policy::{Expiry, EXPIRY_WARNING_DAYS};
    let rows = app.state().exemption_rows();
    let state = app.state();
    let t = theme();
    let scope_name = state
        .exemption_scope
        .as_ref()
        .map(|s| state.scope_label(&s.arm_id()))
        .unwrap_or_else(|| "whole tenant".into());
    let heading = format!("Exemptions · {scope_name}");
    if rows.is_empty() {
        let message = if state.loading {
            "Loading policy exemptions…".to_string()
        } else if !state.filter.is_empty() {
            "No exemptions match the current filter.\n\nPress Esc to clear the filter.".into()
        } else {
            format!("No policy exemptions at or below {scope_name}.")
        };
        let content = Paragraph::new(message)
            .block(Block::default().borders(Borders::ALL).title(heading))
            .style(Style::default().fg(t.subtle))
            .wrap(Wrap { trim: true });
        f.render_widget(content, area);
        return Ok(());
    }

    let (category_w, expiry_w, assignment_w, scope_w) = (11, 24, 32, 30);
    let name_col = name_width(
        area.width,
        category_w + expiry_w + assignment_w + scope_w,
        36,
    );
    let viewport = area.height.saturating_sub(2) as usize;
    let mut list_state = state.list_state.clone();
    let visible = list_state.visible_range(rows.len(), viewport);
    let items: Vec<_> = rows[visible.clone()]
        .iter()
        .map(|r| {
            list_item_spans(vec![
                Span::styled(
                    cell(&r.exemption.display_name, name_col),
                    Style::default().fg(t.text).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    cell(&r.exemption.category, category_w),
                    Style::default().fg(t.accent),
                ),
                expiry_span(r.expiry, expiry_w),
                Span::styled(
                    cell(&r.assignment, assignment_w),
                    Style::default().fg(t.highlight),
                ),
                Span::styled(cell(&r.scope_label, scope_w), Style::default().fg(t.muted)),
                Span::styled(
                    r.exemption.description.clone(),
                    Style::default().fg(t.muted),
                ),
            ])
        })
        .collect();

    let expired = rows
        .iter()
        .filter(|r| matches!(r.expiry, Expiry::Expired(_)))
        .count();
    let soon = rows
        .iter()
        .filter(|r| matches!(r.expiry, Expiry::In(d) if d <= EXPIRY_WARNING_DAYS))
        .count();
    let title = if state.filter_mode {
        format!("{heading} ({}) - Filter: {}_", rows.len(), state.filter)
    } else {
        format!(
            "{heading} ({} exemption{}, {expired} expired, {soon} expiring within \
             {EXPIRY_WARNING_DAYS} days) · Enter details",
            rows.len(),
            if rows.len() == 1 { "" } else { "s" }
        )
    };
    let list = SelectableList::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(theme().selection());
    let mut widget_state = SelectableListState {
        selected: list_state.selected - visible.start,
        offset: 0,
    };
    list.render(area, f.buffer_mut(), &mut widget_state);
    app.state_mut().list_state = list_state;
    Ok(())
}

/// `:compliance`: subscriptions ranked by non-compliant resources.
fn draw_compliance(f: &mut Frame, area: Rect, app: &mut TuiApp) -> Result<()> {
    let rows = app.state().compliance_rows();
    let state = app.state();
    let t = theme();
    let scope_name = match &state.compliance_group {
        Some(group) => state.scope_label(&format!(
            "/providers/Microsoft.Management/managementGroups/{group}"
        )),
        None => "whole tenant".into(),
    };
    let heading = format!("Compliance · {scope_name}");
    if rows.is_empty() {
        let message = if state.compliance_rollup.is_none() && state.loading {
            "Loading compliance per subscription…".to_string()
        } else if state.compliance_rollup.is_none() {
            "Compliance per subscription is not loaded (it needs Resource Graph access at \
             the tenant root). Press r to retry."
                .to_string()
        } else if !state.filter.is_empty() {
            "No subscriptions match the current filter.\n\nPress Esc to clear the filter.".into()
        } else {
            "No subscriptions here.".into()
        };
        let content = Paragraph::new(message)
            .block(Block::default().borders(Borders::ALL).title(heading))
            .style(Style::default().fg(t.subtle))
            .wrap(Wrap { trim: true });
        f.render_widget(content, area);
        return Ok(());
    }

    let group_width = 28;
    let name_col = name_width(area.width, 8 + COMPLIANCE_WIDTH + group_width, 50);
    let viewport = area.height.saturating_sub(2) as usize;
    let mut list_state = state.list_state.clone();
    let visible = list_state.visible_range(rows.len(), viewport);
    let items: Vec<_> = rows[visible.clone()]
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let (compliance, style) = compliance_cell(Some(r.compliance));
            list_item_spans(vec![
                Span::styled(
                    format!("{:>4}  ", visible.start + i + 1),
                    Style::default().fg(t.muted),
                ),
                Span::styled(
                    cell(&r.name, name_col),
                    Style::default().fg(t.text).add_modifier(Modifier::BOLD),
                ),
                Span::styled(cell(&compliance, COMPLIANCE_WIDTH), style),
                Span::styled(cell(&r.group, group_width), Style::default().fg(t.muted)),
            ])
        })
        .collect();

    let affected = rows
        .iter()
        .filter(|r| r.compliance.non_compliant > 0)
        .count();
    let title = if state.filter_mode {
        format!("{heading} ({}) - Filter: {}_", rows.len(), state.filter)
    } else {
        format!(
            "{heading} ({affected} of {} subscriptions with non-compliant resources) · \
             Enter policy",
            rows.len()
        )
    };
    let list = SelectableList::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(theme().selection());
    let mut widget_state = SelectableListState {
        selected: list_state.selected - visible.start,
        offset: 0,
    };
    list.render(area, f.buffer_mut(), &mut widget_state);
    app.state_mut().list_state = list_state;
    Ok(())
}

/// Resources that are non-compliant with one assignment, in the policy scope.
fn draw_non_compliant(f: &mut Frame, area: Rect, app: &mut TuiApp) -> Result<()> {
    let rows = app.state().non_compliant_rows();
    let state = app.state();
    let t = theme();
    let assignment = state
        .policy_states_assignment
        .as_ref()
        .map(|a| a.display_name.clone())
        .unwrap_or_default();
    let scope_name = state
        .policy_scope
        .as_ref()
        .map(|s| state.scope_label(&s.arm_id()))
        .unwrap_or_default();
    let heading = format!(
        "Non-compliant · {} · {scope_name}",
        truncate_with_ellipsis(&assignment, 60)
    );

    if rows.is_empty() {
        let message = if state.loading {
            "Loading non-compliant resources…".to_string()
        } else if !state.filter.is_empty() {
            "No resources match the current filter.\n\nPress Esc to clear the filter.".into()
        } else {
            format!(
                "No resource in {scope_name} is non-compliant with this assignment.\n\n\
                 Azure evaluates policy about once a day; press r to refresh."
            )
        };
        let content = Paragraph::new(message)
            .block(Block::default().borders(Borders::ALL).title(heading))
            .style(Style::default().fg(t.subtle))
            .wrap(Wrap { trim: true });
        f.render_widget(content, area);
        return Ok(());
    }

    let type_width = 22;
    let rg_width = 28;
    let name_col = name_width(area.width, type_width + rg_width + 30, 60);
    let viewport = area.height.saturating_sub(2) as usize;
    let mut list_state = state.list_state.clone();
    let visible = list_state.visible_range(rows.len(), viewport);
    let items: Vec<_> = rows[visible.clone()]
        .iter()
        .map(|r| {
            let policies = match r.policies.len() {
                0 => String::new(),
                1 => r.policies[0].clone(),
                n => format!("{} (+{} more)", r.policies[0], n - 1),
            };
            list_item_spans(vec![
                Span::styled(
                    cell(r.state.resource_name(), name_col),
                    Style::default().fg(t.text).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    cell(
                        &resource_type::short_label(&r.state.resource_type),
                        type_width,
                    ),
                    Style::default().fg(t.accent),
                ),
                Span::styled(
                    cell(&r.state.resource_group, rg_width),
                    Style::default().fg(t.muted),
                ),
                Span::styled(policies, Style::default().fg(t.error)),
            ])
        })
        .collect();

    let title = if state.filter_mode {
        format!("{heading} ({}) - Filter: {}_", rows.len(), state.filter)
    } else {
        format!(
            "{heading} ({} resource{}) · Enter details",
            rows.len(),
            if rows.len() == 1 { "" } else { "s" }
        )
    };
    let list = SelectableList::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(theme().selection());
    let mut widget_state = SelectableListState {
        selected: list_state.selected - visible.start,
        offset: 0,
    };
    list.render(area, f.buffer_mut(), &mut widget_state);
    app.state_mut().list_state = list_state;
    Ok(())
}

fn draw_policy(f: &mut Frame, area: Rect, app: &mut TuiApp) -> Result<()> {
    let rows = app.state().policy_rows();
    let state = app.state();
    let t = theme();
    let scope_name = state
        .policy_scope
        .as_ref()
        .map(|s| state.scope_label(&s.arm_id()))
        .unwrap_or_default();
    let below = state.policy_scope.as_ref().map_or(0, |s| {
        c9s_core::policy::assigned_below(&state.policy_assignments, s, state.scope_tree.as_ref())
    });
    let below_note = if below > 0 {
        format!(", {below} more assigned below")
    } else {
        String::new()
    };

    if rows.is_empty() {
        let message = if state.loading {
            "Loading policy assignments and compliance…".to_string()
        } else if !state.filter.is_empty() {
            "No assignments match the current filter.\n\nPress Esc to clear the filter.".into()
        } else {
            format!(
                "No policy assignments apply at {scope_name}{below_note}.\n\nOpen a child scope \
                 (:policy <subscription>) or press r to refresh."
            )
        };
        let content = Paragraph::new(message)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("Policy · {scope_name}")),
            )
            .style(Style::default().fg(t.subtle))
            .wrap(Wrap { trim: true });
        f.render_widget(content, area);
        return Ok(());
    }

    let compliance_width = 22;
    let origin_width = 30;
    let name_col = name_width(area.width, 3 + compliance_width + 14 + origin_width, 70);
    // Inside the border: the compliance summary, then the list.
    let inner = Block::default().borders(Borders::ALL).inner(area);
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);
    let summary = compliance_summary_line(state, &rows);
    let viewport = parts[1].height as usize;
    let mut list_state = state.list_state.clone();
    let visible = list_state.visible_range(rows.len(), viewport);
    let items: Vec<_> = rows[visible.clone()]
        .iter()
        .map(|r| {
            let kind = if r.assignment.is_initiative() {
                "I "
            } else {
                "P "
            };
            let (compliance, style) = match r.compliance {
                Some(c) if c.non_compliant > 0 => (
                    format!("{} / {} non-compliant", c.non_compliant, c.total),
                    Style::default().fg(t.error).add_modifier(Modifier::BOLD),
                ),
                Some(c) => (
                    format!("compliant ({})", c.total),
                    Style::default().fg(t.ok),
                ),
                None if state.loading => ("…".into(), Style::default().fg(t.muted)),
                None => ("no data".into(), Style::default().fg(t.muted)),
            };
            let enforcement = if r.assignment.is_enforced() {
                ""
            } else {
                "not enforced"
            };
            list_item_spans(vec![
                Span::styled(kind, Style::default().fg(t.accent)),
                Span::styled(
                    cell(&r.assignment.display_name, name_col),
                    Style::default().fg(t.text).add_modifier(Modifier::BOLD),
                ),
                Span::styled(cell(&compliance, compliance_width), style),
                Span::styled(cell(enforcement, 14), Style::default().fg(t.highlight)),
                Span::styled(
                    cell(&r.origin_label, origin_width),
                    Style::default().fg(if r.inherited { t.muted } else { t.special }),
                ),
            ])
        })
        .collect();

    let title = if state.filter_mode {
        format!(
            "Policy · {scope_name} ({}) - Filter: {}_",
            rows.len(),
            state.filter
        )
    } else {
        format!(
            "Policy · {scope_name} ({} assignment{}{below_note}) · I initiative, P policy · \
             Enter non-compliant, d details",
            rows.len(),
            if rows.len() == 1 { "" } else { "s" }
        )
    };
    f.render_widget(Block::default().borders(Borders::ALL).title(title), area);
    f.render_widget(Paragraph::new(summary), parts[0]);
    let list = SelectableList::new(items).highlight_style(theme().selection());
    let mut widget_state = SelectableListState {
        selected: list_state.selected - visible.start,
        offset: 0,
    };
    list.render(parts[1], f.buffer_mut(), &mut widget_state);
    app.state_mut().list_state = list_state;
    Ok(())
}

/// The policy view's first line: how compliant the scope is overall.
fn compliance_summary_line(state: &AppState, rows: &[c9s_core::state::PolicyRow]) -> Line<'static> {
    let t = theme();
    let label = Span::styled(
        "Compliance ",
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
    );
    let Some(summary) = state.policy_summary() else {
        let text = if state.loading {
            "loading…"
        } else {
            "no data"
        };
        return Line::from(vec![
            label,
            Span::styled(text, Style::default().fg(t.muted)),
        ]);
    };
    let Some(percent) = summary.percent_compliant() else {
        return Line::from(vec![
            label,
            Span::styled(
                "no evaluated resources yet (Azure evaluates policy about once a day)",
                Style::default().fg(t.muted),
            ),
        ]);
    };
    let r = summary.resources;
    let good = r.non_compliant == 0;
    let mut spans = vec![
        label,
        Span::styled(
            format!("{percent:.1}%"),
            Style::default()
                .fg(if good { t.ok } else { t.error })
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "  ·  {} of {} resources non-compliant",
                grouped(r.non_compliant),
                grouped(r.total)
            ),
            Style::default().fg(t.text),
        ),
    ];
    if summary.subscriptions > 1 {
        spans.push(Span::styled(
            format!(
                "  ·  {} of {} subscriptions affected",
                summary.subscriptions_affected, summary.subscriptions
            ),
            Style::default().fg(t.text),
        ));
    }
    let failing = rows
        .iter()
        .filter(|r| r.compliance.is_some_and(|c| c.non_compliant > 0))
        .count();
    spans.push(Span::styled(
        format!(
            "  ·  {failing} of {} assignments with non-compliant resources",
            rows.len()
        ),
        Style::default().fg(t.muted),
    ));
    Line::from(spans)
}

/// Now, in unix seconds.
fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// A count with thousands separators: 12345 → "12,345".
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Draw the tenants to switch to (`:ctx`)
fn draw_tenant_list(f: &mut Frame, area: Rect, app: &mut TuiApp) -> Result<()> {
    let rows = app.state().tenant_rows();
    let state = app.state();
    let name_col = name_width(area.width, 2 + 36 + 2 + 12 + 30, 40);
    let viewport = area.height.saturating_sub(2) as usize;
    let mut list_state = state.list_state.clone();
    let visible = list_state.visible_range(rows.len(), viewport);
    let items: Vec<_> = rows[visible.clone()]
        .iter()
        .map(|r| {
            let marker = if r.current { "● " } else { "  " };
            let accounts = match r.provider {
                c9s_core::provider::Provider::Azure => {
                    format!("{} subscriptions", r.subscriptions)
                }
                _ => "gcloud config".to_string(),
            };
            list_item_spans(vec![
                Span::styled(marker, Style::default().fg(theme().ok)),
                Span::styled(
                    cell(r.provider.as_str(), 7),
                    Style::default().fg(theme().special),
                ),
                Span::styled(
                    cell(&r.name, name_col),
                    Style::default()
                        .fg(theme().text)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(cell(&accounts, 18), Style::default().fg(theme().highlight)),
                Span::styled(
                    cell(r.domain.as_deref().unwrap_or(""), 32),
                    Style::default().fg(theme().accent),
                ),
                Span::styled(
                    r.id.clone().unwrap_or_default(),
                    Style::default().fg(theme().muted),
                ),
            ])
        })
        .collect();
    let names_note = if state.tenants.is_empty() && state.loading {
        " · loading names…"
    } else {
        ""
    };
    let title = if state.filter_mode {
        format!("Contexts ({}) - Filter: {}_", rows.len(), state.filter)
    } else {
        format!(
            "Contexts ({}){names_note}: Azure tenants, gcloud configurations, AWS profiles · Enter switches · ● current",
            rows.len().saturating_sub(1)
        )
    };
    let list = SelectableList::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(theme().selection());
    let mut widget_state = SelectableListState {
        selected: list_state.selected - visible.start,
        offset: 0,
    };
    list.render(area, f.buffer_mut(), &mut widget_state);
    app.state_mut().list_state = list_state;
    Ok(())
}

/// Draw the tag key/value counts (`:tag`)
fn draw_tag_summary(f: &mut Frame, area: Rect, app: &mut TuiApp) -> Result<()> {
    let rows = app.state().tag_summary();
    let state = app.state();
    let scope = scope_info(state);

    if rows.is_empty() {
        let message = if state.loading {
            "Loading resources..."
        } else if !state.filter.is_empty() {
            "No tags match the current filter.\n\nPress Esc to clear the filter."
        } else if !state.resources.is_empty() {
            "None of these resources have tags."
        } else {
            "No resources loaded.\n\nPress 'r' to refresh."
        };
        let content = Paragraph::new(message)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("Tags{scope}")),
            )
            .style(Style::default().fg(theme().subtle))
            .wrap(Wrap { trim: true });
        f.render_widget(content, area);
        return Ok(());
    }

    let count_width = rows[0].count.to_string().len().max(5);
    // Fixed column: the count; the key gets up to a third of the rest.
    let key_col = name_width(area.width, count_width + 2 + 20, 40).min(
        rows.iter()
            .map(|t| t.key.chars().count() + 2)
            .max()
            .unwrap_or(20),
    );
    let viewport = area.height.saturating_sub(2) as usize;
    let mut list_state = state.list_state.clone();
    let visible = list_state.visible_range(rows.len(), viewport);
    let items: Vec<_> = rows[visible.clone()]
        .iter()
        .map(|t| {
            list_item_spans(vec![
                Span::styled(
                    format!("{:>count_width$}  ", t.count),
                    Style::default()
                        .fg(theme().highlight)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    cell(&t.key, key_col),
                    Style::default()
                        .fg(theme().accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(t.value.clone(), Style::default().fg(theme().text)),
            ])
        })
        .collect();

    let keys = {
        let mut keys: Vec<String> = rows.iter().map(|t| t.key.to_lowercase()).collect();
        keys.sort();
        keys.dedup();
        keys.len()
    };
    let title = if state.filter_mode {
        format!("Tags ({}){scope} - Filter: {}_", rows.len(), state.filter)
    } else {
        let plural = |n: usize, word: &str| {
            if n == 1 {
                format!("1 {word}")
            } else {
                format!("{n} {word}s")
            }
        };
        format!(
            "Tags ({} of {}){scope} - Enter lists the resources",
            plural(rows.len(), "value"),
            plural(keys, "key")
        )
    };
    let list = SelectableList::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(theme().selection());
    let mut widget_state = SelectableListState {
        selected: list_state.selected - visible.start,
        offset: 0,
    };
    list.render(area, f.buffer_mut(), &mut widget_state);
    app.state_mut().list_state = list_state;
    Ok(())
}

/// Draw resource list view
fn draw_resource_list(f: &mut Frame, area: Rect, app: &mut TuiApp) -> Result<()> {
    let filtered_resources = app.state().filtered_resources();

    if filtered_resources.is_empty() {
        let message = if app.state().loading {
            "Loading resources...\n\nPlease wait while we fetch resources from Azure."
        } else if !app.state().filter.is_empty() {
            "No resources match the current filter.\n\nPress Esc to clear the filter."
        } else if !app.state().resource_query.is_empty() && !app.state().resources.is_empty() {
            "No resources match the filters in the title.\n\nRun :res without filters to see all."
        } else {
            "No resources loaded.\n\nSelect a subscription to view resources.\nPress 'r' to refresh."
        };

        let content = Paragraph::new(message)
            .block(Block::default().borders(Borders::ALL).title("Resources"))
            .style(Style::default().fg(theme().subtle))
            .wrap(Wrap { trim: true });
        f.render_widget(content, area);
        return Ok(());
    }

    // Create list items with improved resource type display
    // Only build widgets for the rows that fit on screen (minus the borders).
    let viewport = area.height.saturating_sub(2) as usize;
    let mut list_state = app.state().list_state.clone();
    let visible = list_state.visible_range(filtered_resources.len(), viewport);
    // Fixed columns: type (20) and location (20).
    let name_col = name_width(area.width, 20 + 20, 80);
    let items: Vec<_> = filtered_resources[visible.clone()]
        .iter()
        .map(|res| {
            // Truncate long names to fit better
            let display_name = cell(&res.name, name_col);

            // Use improved resource type display
            let short_type = resource_type::short_label(&res.resource_type);
            let display_type = cell(&short_type, 20);

            // Lowercase first: Resource Graph returns types in lowercase.
            let type_color = match res.resource_type.to_ascii_lowercase().as_str() {
                t if t.contains("virtualmachine") => theme().accent,
                t if t.contains("storage") => theme().highlight,
                t if t.contains("network") => theme().info,
                t if t.contains("database") || t.contains("sql") || t.contains("cosmos") => {
                    theme().special
                }
                t if t.contains("keyvault") => theme().error,
                t if t.contains("container") || t.contains("kubernetes") => theme().ok,
                _ => theme().text,
            };

            list_item_spans(vec![
                Span::styled(
                    display_name,
                    Style::default()
                        .fg(theme().text)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(display_type, Style::default().fg(type_color)),
                Span::styled(
                    cell(&res.location, 20),
                    Style::default().fg(theme().highlight),
                ),
            ])
        })
        .collect();

    // Create title with filter indicator and scope
    let scope_info = scope_info(app.state());
    let title = if app.state().filter_mode {
        format!(
            "Resources ({}){}  - Filter: {}_",
            filtered_resources.len(),
            scope_info,
            app.state().filter
        )
    } else if !app.state().filter.is_empty() {
        format!(
            "Resources ({}){}  - Filtered by: {}",
            filtered_resources.len(),
            scope_info,
            app.state().filter
        )
    } else {
        format!(
            "Resources ({}){}  - Press Enter for details",
            filtered_resources.len(),
            scope_info
        )
    };

    // Create the list widget
    let list = SelectableList::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(theme().selection());

    // Items are already windowed: render from the top with a relative selection.
    let mut widget_state = SelectableListState {
        selected: list_state.selected - visible.start,
        offset: 0,
    };
    list.render(area, f.buffer_mut(), &mut widget_state);

    // Persist the scroll offset so scrolling up doesn't snap the selection.
    app.state_mut().list_state = list_state;

    Ok(())
}

/// Search (`/`): highlight every match in a detail view's lines, the
/// current one more strongly.
fn highlight_search(lines: &mut [Line<'static>], state: &AppState) {
    let pattern = state.detail_pattern();
    if pattern.is_empty() {
        return;
    }
    let texts: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    let matches = search::find_matches(&texts, pattern);
    let current = state.detail_match;
    for (i, line) in lines.iter_mut().enumerate() {
        let ranges: Vec<(usize, usize)> = matches
            .iter()
            .filter(|m| m.line == i)
            .map(|m| (m.start, m.end))
            .collect();
        if !ranges.is_empty() {
            let cur = current.filter(|(l, _)| *l == i).map(|(_, c)| c);
            *line = search::highlight(std::mem::take(line), &ranges, cur);
        }
    }
}

fn draw_resource_detail(f: &mut Frame, area: Rect, app: &mut TuiApp) -> Result<()> {
    let Some(resource) = app.state().selected_resource_detail.clone() else {
        let content = Paragraph::new("No resource selected.\n\nPress Esc to go back.")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Resource Detail"),
            )
            .style(Style::default().fg(theme().subtle))
            .wrap(Wrap { trim: true });
        f.render_widget(content, area);
        return Ok(());
    };
    let state = app.state();
    let doc = state.resource_document.as_ref();
    let tab = state.detail_tab;

    let mut lines = detail_lines(state);
    highlight_search(&mut lines, state);
    // The cursor of a tab with rows (Tags, Policy): Enter opens that row.
    if let Some(n) = state.detail_rows().filter(|n| *n > 0) {
        let line = detail_row_line(tab, state.detail_cursor.min(n - 1));
        if let Some(line) = lines.get_mut(line) {
            *line = std::mem::take(line).patch_style(theme().selection());
        }
    }
    let loading_hint = if doc.is_none() && state.loading {
        " · loading details…"
    } else {
        ""
    };

    let block = Block::default().borders(Borders::ALL).title(format!(
        "{} · {}{}",
        truncate_with_ellipsis(&resource.name, 60),
        resource_type::short_label(&resource.resource_type),
        loading_hint
    ));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(inner);

    let titles: Vec<Line> = DetailTab::ALL
        .iter()
        .enumerate()
        .map(|(i, t)| Line::from(format!("{} {}", i + 1, t.title())))
        .collect();
    let tabs = Tabs::new(titles)
        .select(tab.index())
        .style(Style::default().fg(theme().subtle))
        .highlight_style(theme().selection())
        .divider("│");
    f.render_widget(tabs, parts[0]);

    // Clamp the scroll so the last line can reach the bottom but not beyond.
    let viewport = parts[1].height as usize;
    let max_scroll = lines.len().saturating_sub(viewport);
    let mut scroll = state.detail_scroll;
    // Bring a search match into view, centred, if it is off screen.
    if let Some(line) = state.detail_scroll_to {
        if line < scroll || line >= scroll + viewport {
            scroll = line.saturating_sub(viewport / 2);
        }
    }
    let scroll = scroll.min(max_scroll);
    let content = Paragraph::new(lines).scroll((scroll.min(u16::MAX as usize) as u16, 0));
    f.render_widget(content, parts[1]);
    let state = app.state_mut();
    state.detail_scroll = scroll;
    state.detail_scroll_to = None;
    Ok(())
}

/// The lines of the detail view's current tab, before search highlighting.
/// Search (`/`, `n`, `N`) runs over the same lines, so positions agree.
pub fn detail_lines(state: &AppState) -> Vec<Line<'static>> {
    let Some(resource) = state.selected_resource_detail.as_ref() else {
        return vec![];
    };
    let doc = state.resource_document.as_ref();
    match state.detail_tab {
        DetailTab::Overview => overview_lines(&detail::overview(doc, resource)),
        DetailTab::Tags => {
            let tags = detail::tags(doc, resource);
            if tags.is_empty() {
                vec![Line::from(Span::styled(
                    "No tags.",
                    Style::default().fg(theme().muted),
                ))]
            } else {
                let width = tags
                    .iter()
                    .map(|(k, _)| k.chars().count())
                    .max()
                    .unwrap_or(0);
                tags.into_iter()
                    .map(|(k, v)| {
                        Line::from(vec![
                            Span::styled(
                                format!("{k:<width$}  "),
                                Style::default().fg(theme().highlight),
                            ),
                            Span::styled(v, Style::default().fg(theme().text)),
                        ])
                    })
                    .collect()
            }
        }
        DetailTab::Policy => policy_state_lines(state),
        DetailTab::Json => detail::json_text(doc).lines().map(json_line).collect(),
    }
}

/// The Policy tab: every policy state of the resource, non-compliant first.
fn policy_state_lines(state: &AppState) -> Vec<Line<'static>> {
    let t = theme();
    let muted = |text: &str| {
        vec![Line::from(Span::styled(
            text.to_string(),
            Style::default().fg(t.muted),
        ))]
    };
    let Some(rows) = state.detail_policy_states() else {
        return muted("Loading policy states…");
    };
    if rows.is_empty() {
        return muted("No policy evaluates this resource (or Azure has not evaluated it yet).");
    }
    // Column widths: as wide as the longest value, within limits.
    let widest = |f: &dyn Fn(&(String, c9s_core::policy::PolicyState)) -> usize,
                  header: &str,
                  max: usize| {
        rows.iter()
            .map(f)
            .max()
            .unwrap_or(0)
            .max(header.len())
            .min(max)
            + 2
    };
    let policy_w = widest(&|(_, s)| s.policy_label().chars().count(), "Policy", 50);
    let via_w = widest(&|(_, s)| s.source_label().chars().count(), "Via", 40);
    let assignment_w = widest(&|(name, _)| name.chars().count(), "Assignment", 36);
    let mut lines = vec![Line::from(Span::styled(
        format!(
            "{:<14}{}{}{}Assigned at",
            "State",
            cell("Policy", policy_w),
            cell("Via", via_w),
            cell("Assignment", assignment_w)
        ),
        Style::default().fg(t.muted).add_modifier(Modifier::BOLD),
    ))];
    lines.extend(rows.into_iter().map(|(name, s)| {
        let style = if s.is_non_compliant() {
            Style::default().fg(t.error).add_modifier(Modifier::BOLD)
        } else if s.state.eq_ignore_ascii_case("Compliant") {
            Style::default().fg(t.ok)
        } else {
            Style::default().fg(t.muted)
        };
        let scope = state.scope_label(c9s_core::policy::assignment_scope(&s.assignment_id));
        // Exempt: which exemption, and until when (Enter opens it).
        let exemption = state
            .exemption_for(&s)
            .filter(|_| s.state.eq_ignore_ascii_case("Exempt"))
            .map(|e| {
                let expiry = e.expiry(chrono_now());
                (
                    format!(
                        "  · exemption {}, {}",
                        e.display_name,
                        c9s_core::policy::expiry_label(expiry)
                    ),
                    expiry,
                )
            });
        let mut spans = vec![
            Span::styled(format!("{:<13} ", s.state), style),
            Span::styled(
                cell(s.policy_label(), policy_w),
                Style::default().fg(t.text),
            ),
            Span::styled(
                cell(&s.source_label(), via_w),
                Style::default().fg(if s.via_initiative() {
                    t.accent
                } else {
                    t.muted
                }),
            ),
            Span::styled(cell(&name, assignment_w), Style::default().fg(t.highlight)),
            Span::styled(scope, Style::default().fg(t.muted)),
        ];
        if let Some((text, expiry)) = exemption {
            let color = match expiry {
                c9s_core::policy::Expiry::Expired(_) => t.error,
                _ => t.accent,
            };
            spans.push(Span::styled(text, Style::default().fg(color)));
        }
        Line::from(spans)
    }));
    lines
}

/// The line of the detail view's content that shows row `row` of a tab with
/// rows (the Policy tab starts with a header line).
pub fn detail_row_line(tab: DetailTab, row: usize) -> usize {
    match tab {
        DetailTab::Policy => row + 1,
        _ => row,
    }
}

/// The Overview tab: a heading per section, then aligned label/value rows.
fn overview_lines(sections: &[detail::Section]) -> Vec<Line<'static>> {
    let width = sections
        .iter()
        .flat_map(|s| s.rows.iter().map(|(l, _)| l.chars().count()))
        .max()
        .unwrap_or(0)
        .min(40);
    let mut lines = Vec::new();
    for (i, section) in sections.iter().enumerate() {
        if i > 0 {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            section.title,
            Style::default()
                .fg(theme().accent)
                .add_modifier(Modifier::BOLD),
        )));
        for (label, value) in &section.rows {
            let label = truncate_with_ellipsis(label, width);
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {label:<width$}  "),
                    Style::default().fg(theme().highlight),
                ),
                Span::styled(value.clone(), Style::default().fg(theme().text)),
            ]));
        }
    }
    lines
}

/// One line of pretty-printed JSON with keys and values coloured.
fn json_line(line: &str) -> Line<'static> {
    let indent_len = line.len() - line.trim_start().len();
    let (indent, rest) = line.split_at(indent_len);
    let mut spans = vec![Span::raw(indent.to_string())];
    // `"key": value` — split after the key's closing quote and colon.
    let value = match rest.find("\": ") {
        Some(i) if rest.starts_with('"') => {
            spans.push(Span::styled(
                rest[..i + 1].to_string(),
                Style::default().fg(theme().accent),
            ));
            spans.push(Span::raw(": "));
            &rest[i + 3..]
        }
        _ => rest,
    };
    let bare = value.trim_end_matches(',');
    let color = if bare.starts_with('"') {
        theme().ok
    } else if bare == "true" || bare == "false" || bare == "null" {
        theme().special
    } else if bare.starts_with(|c: char| c == '-' || c.is_ascii_digit()) {
        theme().highlight
    } else {
        theme().subtle
    };
    spans.push(Span::styled(value.to_string(), Style::default().fg(color)));
    Line::from(spans)
}

/// Draw logs view
fn draw_logs(f: &mut Frame, area: Rect, _app: &TuiApp) -> Result<()> {
    let content =
        Paragraph::new("Logs view\n\nSelect a resource to view its logs.\n\nPress Esc to go back.")
            .block(Block::default().borders(Borders::ALL).title("Logs"))
            .style(Style::default().fg(theme().subtle))
            .wrap(Wrap { trim: true });
    f.render_widget(content, area);
    Ok(())
}

/// Suggestions for the `:` command being typed, in a box that sits on top
/// of the footer and grows upwards.
fn draw_command_suggestions(f: &mut Frame, footer: Rect, app: &TuiApp) {
    let state = app.state();
    let completion = c9s_core::complete::complete(&state.command_input, state);
    if completion.items.is_empty() {
        return;
    }
    const VISIBLE: usize = 8;
    let height = completion.items.len().min(VISIBLE) as u16 + 2;
    let width = footer.width.min(80);
    let area = Rect {
        x: footer.x,
        y: footer.y.saturating_sub(height),
        width,
        height: height.min(footer.y),
    };
    let selected = state.command_selection.min(completion.items.len() - 1);
    let first = selected.saturating_sub(VISIBLE - 1);
    let text_width = completion
        .items
        .iter()
        .map(|s| s.text.chars().count())
        .max()
        .unwrap_or(0)
        .min(40)
        + 2;
    let lines: Vec<Line> = completion
        .items
        .iter()
        .enumerate()
        .skip(first)
        .take(VISIBLE)
        .map(|(i, s)| {
            let style = if i == selected {
                theme().selection()
            } else {
                Style::default().fg(theme().text)
            };
            Line::from(vec![
                Span::styled(cell(&s.text, text_width), style),
                Span::styled(s.detail.clone(), Style::default().fg(theme().muted)),
            ])
        })
        .collect();
    theme::clear(f, area);
    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(format!(
            " {} suggestion{} · Tab completes · ↑↓ choose ",
            completion.items.len(),
            if completion.items.len() == 1 { "" } else { "s" }
        ))),
        area,
    );
}

/// The command palette: a search line and the best matches, with the
/// matched characters highlighted.
fn draw_palette(f: &mut Frame, area: Rect, app: &TuiApp) {
    let Some(palette) = &app.state().palette else {
        return;
    };
    let width = (area.width * 7 / 10).max(40).min(area.width);
    let height = (area.height * 6 / 10).max(10).min(area.height);
    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 4,
        width,
        height,
    };
    theme::clear(f, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme().accent))
        .title(format!(
            " Go to · {} match{} · Enter go · Esc close ",
            palette.results.len(),
            if palette.results.len() == 1 { "" } else { "es" }
        ));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let mut lines = vec![
        Line::from(vec![
            Span::styled("> ", Style::default().fg(theme().accent)),
            Span::styled(
                format!("{}_", palette.query),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
    ];
    let visible = (inner.height as usize).saturating_sub(2);
    let first = palette.selected.saturating_sub(visible.saturating_sub(1));
    let label_width = (inner.width as usize / 2).max(20);
    for (i, result) in palette.results.iter().enumerate().skip(first).take(visible) {
        let selected = i == palette.selected;
        let base = if selected {
            theme().selection()
        } else {
            Style::default().fg(theme().text)
        };
        let mut spans = vec![Span::styled(
            format!("{:<4}", result.item.kind.tag()),
            if selected {
                base
            } else {
                Style::default().fg(theme().muted)
            },
        )];
        // The label, with matched characters highlighted, cut to fit.
        let label = cell(&result.item.label, label_width);
        for (j, ch) in label.chars().enumerate() {
            let style = if result.positions.contains(&j) {
                base.fg(if selected {
                    theme().on_selection()
                } else {
                    theme().highlight
                })
                .add_modifier(Modifier::BOLD)
            } else {
                base
            };
            spans.push(Span::styled(ch.to_string(), style));
        }
        spans.push(Span::styled(
            result.item.detail.clone(),
            if selected {
                base
            } else {
                Style::default().fg(theme().muted)
            },
        ));
        lines.push(Line::from(spans));
    }
    if palette.results.is_empty() {
        lines.push(Line::from(Span::styled(
            "Nothing matches. Only loaded data is searched: run :res all first to find any resource.",
            Style::default().fg(theme().muted),
        )));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

/// Draw the footer with keybindings or command input
fn draw_footer(f: &mut Frame, area: Rect, app: &TuiApp) {
    // If in command mode, show command input
    if app.state().command_mode {
        let command_text = format!(":{}_", app.state().command_input);
        let footer = Paragraph::new(command_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Command · Tab completes · Enter runs · Esc cancels"),
            )
            .style(Style::default().fg(theme().accent));
        f.render_widget(footer, area);
        return;
    }
    // Typing a search in the detail view
    if let Some(input) = &app.state().detail_search_input {
        let footer = Paragraph::new(format!("/{input}_"))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Search · Enter keep · Esc cancel"),
            )
            .style(Style::default().fg(theme().accent));
        f.render_widget(footer, area);
        return;
    }

    let keybindings = if app.state().filter_mode {
        vec![
            ("Type", "Filter"),
            ("Enter", "Apply"),
            ("Esc", "Cancel"),
            ("Backspace", "Delete"),
        ]
    } else if app.state().help_open {
        vec![("Esc/?/q", "Close Help")]
    } else {
        match app.state().view {
            ViewMode::ManagementGroupTree => vec![
                ("↑↓/jk", "Navigate"),
                ("Space", "Expand"),
                ("←→/hl", "Close/Open"),
                ("Enter", opens_onto_title(app.state().provider)),
                ("/", "Filter"),
                (":", "Command"),
                ("^P", "Go to"),
                ("?", "Help"),
                ("r", "Refresh"),
                ("q", "Quit"),
            ],
            ViewMode::SubscriptionList => vec![
                ("↑↓/jk", "Navigate"),
                ("g/G", "First/Last"),
                ("PgUp/PgDn", "Page"),
                ("Enter", "Select"),
                ("/", "Filter"),
                (":", "Command"),
                ("^P", "Go to"),
                ("?", "Help"),
                ("r", "Refresh"),
                ("q", "Quit"),
            ],
            ViewMode::ResourceGroupList => vec![
                ("↑↓/jk", "Navigate"),
                ("g/G", "First/Last"),
                ("PgUp/PgDn", "Page"),
                ("Enter", "View Resources"),
                ("/", "Filter"),
                (":", "Command"),
                ("^P", "Go to"),
                ("?", "Help"),
                ("r", "Refresh"),
                ("Esc", "Back"),
                ("q", "Quit"),
            ],
            ViewMode::ResourceList => vec![
                ("↑↓/jk", "Navigate"),
                ("g/G", "First/Last"),
                ("PgUp/PgDn", "Page"),
                ("Enter", "Details"),
                ("/", "Filter"),
                (":", "Command"),
                ("^P", "Go to"),
                ("?", "Help"),
                ("r", "Refresh"),
                ("Esc", "Back"),
                ("q", "Quit"),
            ],
            ViewMode::PolicyAssignments => vec![
                ("↑↓/jk", "Navigate"),
                ("Enter", "Non-compliant"),
                ("d", "Details"),
                ("/", "Filter"),
                ("c", "Copy"),
                ("y", "Copy ID"),
                ("o", "Portal"),
                (":", "Command"),
                ("r", "Refresh"),
                ("Esc", "Back"),
                ("q", "Quit"),
            ],
            ViewMode::RoleAssignments => vec![
                ("p", "Principal's access"),
                ("Enter", "Role"),
                ("d", "Details"),
                ("↑↓/jk", "Navigate"),
                ("/", "Filter"),
                ("c", "Copy"),
                ("y", "Copy ID"),
                ("o", "Portal"),
                ("r", "Refresh"),
                ("Esc", "Back"),
                ("q", "Quit"),
            ],
            ViewMode::AppList => vec![
                ("↑↓/jk", "Navigate"),
                ("Enter", "Open"),
                ("m", "Microsoft's"),
                ("p", "Access"),
                ("/", "Filter"),
                ("c", "Copy"),
                ("y", "Copy app ID"),
                ("o", "Portal"),
                ("r", "Refresh"),
                ("q", "Quit"),
            ],
            ViewMode::AppDetail => vec![
                ("Tab/1-7", "Tabs"),
                ("↑↓/jk", "Field/Scroll"),
                ("y/c", "Copy field"),
                ("C", "Copy tab"),
                ("/", "Search"),
                ("o", "Portal"),
                ("p", "Access"),
                ("r", "Refresh"),
                ("Esc", "Back"),
                ("q", "Quit"),
            ],
            ViewMode::GroupList => vec![
                ("↑↓/jk", "Navigate"),
                ("Enter", "Members"),
                ("p", "Access"),
                ("/", "Filter"),
                ("c", "Copy"),
                ("y", "Copy ID"),
                ("o", "Portal"),
                ("r", "Refresh"),
                ("q", "Quit"),
            ],
            ViewMode::GroupMembers | ViewMode::MemberOf => vec![
                ("↑↓/jk", "Navigate"),
                ("Enter", "Open"),
                ("t", "Nested"),
                ("p", "Access"),
                ("/", "Filter"),
                ("c", "Copy"),
                ("y", "Copy ID"),
                ("o", "Portal"),
                ("Esc", "Back"),
                ("q", "Quit"),
            ],
            ViewMode::RoleDefinitions => vec![
                ("↑↓/jk", "Navigate"),
                ("Enter", "Assignments"),
                ("d", "Details"),
                ("/", "Filter"),
                ("c", "Copy"),
                ("y", "Copy ID"),
                ("r", "Refresh"),
                ("q", "Quit"),
            ],
            ViewMode::ExemptionList => vec![
                ("↑↓/jk", "Navigate"),
                ("Enter", "Details"),
                ("/", "Filter"),
                ("c", "Copy"),
                ("y", "Copy ID"),
                ("o", "Portal"),
                (":", "Command"),
                ("r", "Refresh"),
                ("q", "Quit"),
            ],
            ViewMode::ComplianceRanking => vec![
                ("↑↓/jk", "Navigate"),
                ("Enter", "Policy"),
                ("/", "Filter"),
                ("c", "Copy"),
                ("y", "Copy ID"),
                ("o", "Portal"),
                (":", "Command"),
                ("r", "Refresh"),
                ("q", "Quit"),
            ],
            ViewMode::PolicyNonCompliant => vec![
                ("↑↓/jk", "Navigate"),
                ("Enter", "Details"),
                ("/", "Filter"),
                ("c", "Copy"),
                ("y", "Copy ID"),
                ("o", "Portal"),
                (":", "Command"),
                ("r", "Refresh"),
                ("Esc", "Back"),
                ("q", "Quit"),
            ],
            ViewMode::TenantList => vec![
                ("↑↓/jk", "Navigate"),
                ("Enter", "Switch"),
                ("/", "Filter"),
                (":", "Command"),
                ("^P", "Go to"),
                ("?", "Help"),
                ("r", "Refresh"),
                ("q", "Quit"),
            ],
            ViewMode::TagSummary => vec![
                ("↑↓/jk", "Navigate"),
                ("Enter", "List Resources"),
                ("/", "Filter"),
                ("c", "Copy"),
                (":", "Command"),
                ("^P", "Go to"),
                ("?", "Help"),
                ("r", "Refresh"),
                ("Esc", "Back"),
                ("q", "Quit"),
            ],
            ViewMode::ResourceTypeSummary => vec![
                ("↑↓/jk", "Navigate"),
                ("Enter", "List Resources"),
                ("/", "Filter"),
                (":", "Command"),
                ("^P", "Go to"),
                ("?", "Help"),
                ("r", "Refresh"),
                ("Esc", "Back"),
                ("q", "Quit"),
            ],
            ViewMode::ResourceDetail => vec![
                ("Tab/1-4", "Tabs"),
                ("↑↓/jk", "Scroll/Select"),
                ("Enter", "Open tag/policy"),
                ("/ n N", "Search"),
                ("c", "Copy"),
                ("y", "Copy ID"),
                ("o", "Portal"),
                ("r", "Refresh"),
                ("Esc", "Back"),
                ("q", "Quit"),
            ],
            _ => vec![
                ("↑↓/jk", "Navigate"),
                ("Enter", "Select"),
                ("/", "Filter"),
                (":", "Command"),
                ("^P", "Go to"),
                ("?", "Help"),
                ("Esc", "Back"),
                ("r", "Refresh"),
                ("q", "Quit"),
            ],
        }
    };

    let spans: Vec<Span> = keybindings
        .iter()
        .flat_map(|(key, desc)| {
            vec![
                Span::styled(format!(" {} ", key), theme().selection()),
                Span::styled(format!(" {} ", desc), Style::default().fg(theme().text)),
            ]
        })
        .collect();

    let footer = Paragraph::new(Line::from(spans)).block(Block::default().borders(Borders::ALL));

    f.render_widget(footer, area);
}

/// Draw error overlay
/// `s`: the view's columns to sort by, the current one marked.
fn draw_sort_picker(f: &mut Frame, app: &TuiApp) {
    let state = app.state();
    let Some(selected) = state.sort_picker else {
        return;
    };
    let t = theme();
    let columns = c9s_core::sort::columns(&state.view);
    let current = state.sort_spec(&state.view);
    let mut entries: Vec<String> = columns
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let mark = match current {
                Some(s) if s.column == i => {
                    if s.descending {
                        "  ↓ (again: ↑)"
                    } else {
                        "  ↑ (again: ↓)"
                    }
                }
                _ => "",
            };
            format!(" {}  {name}{mark}", i + 1)
        })
        .collect();
    entries.push(" 0  Default order".into());
    let height = entries.len() as u16 + 4;
    let full = f.area();
    let width = 44.min(full.width);
    let area = Rect {
        x: full.x + full.width.saturating_sub(width) / 2,
        y: full.y + full.height.saturating_sub(height) / 2,
        width,
        height: height.min(full.height),
    };
    let mut lines: Vec<Line> = entries
        .into_iter()
        .enumerate()
        .map(|(i, text)| {
            let line = Line::from(text);
            if i == selected {
                line.style(t.selection())
            } else {
                line.style(Style::default().fg(t.text))
            }
        })
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " ↑↓ choose · Enter or 1-9 sort · Esc cancel",
        Style::default().fg(t.muted),
    )));
    f.render_widget(ratatui::widgets::Clear, area);
    let popup = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Sort by")
            .style(match t.background {
                Some(bg) => Style::default().bg(bg),
                None => Style::default(),
            }),
    );
    f.render_widget(popup, area);
}

/// `o` with several pages: which one to open.
fn draw_portal_choice(f: &mut Frame, app: &TuiApp) {
    let Some((pages, selected)) = &app.state().portal_choice else {
        return;
    };
    let t = theme();
    let height = pages.len() as u16 + 4;
    let full = f.area();
    let width = 64.min(full.width);
    let area = Rect {
        x: full.x + full.width.saturating_sub(width) / 2,
        y: full.y + full.height.saturating_sub(height) / 2,
        width,
        height: height.min(full.height),
    };
    let mut lines: Vec<Line> = pages
        .iter()
        .enumerate()
        .map(|(i, (name, _))| {
            let line = Line::from(format!(" {}  {name}", i + 1));
            if i == *selected {
                line.style(t.selection())
            } else {
                line.style(Style::default().fg(t.text))
            }
        })
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " ↑↓ choose · Enter or 1-9 open · Esc cancel",
        Style::default().fg(t.muted),
    )));
    f.render_widget(ratatui::widgets::Clear, area);
    let popup = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Open in the Azure Portal")
            .style(match t.background {
                Some(bg) => Style::default().bg(bg),
                None => Style::default(),
            }),
    );
    f.render_widget(popup, area);
}

fn draw_error_overlay(f: &mut Frame, app: &TuiApp) {
    if let Some(error) = &app.state().error {
        let area = centered_rect(60, 20, f.area());

        let error_text = format!("Error\n\n{}\n\nPress Esc to dismiss", error);
        let error_widget = Paragraph::new(error_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Error")
                    .style(Style::default().fg(theme().error)),
            )
            .style(Style::default().fg(theme().text))
            .wrap(Wrap { trim: true });

        f.render_widget(error_widget, area);
    }
}

/// Helper function to create a centered rect
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_ui_module_exists() {
        // Basic test to ensure module compiles
    }

    #[test]
    fn counts_get_thousands_separators() {
        assert_eq!(super::grouped(0), "0");
        assert_eq!(super::grouped(999), "999");
        assert_eq!(super::grouped(1000), "1,000");
        assert_eq!(super::grouped(1234567), "1,234,567");
    }
}
