use crate::desktop::{Desktop, SystemDesktop};
use crate::event::Event;
use crate::ui::{self, search};
use crate::{Command, CommandKind, Scope};
use anyhow::Result;
use c9s_core::complete;
use c9s_core::detail;
use c9s_core::loader::{LoadReceiver, LoadRequest, LoadResult};
use c9s_core::palette::PaletteAction;
use c9s_core::policy::PolicyScope;
use c9s_core::provider::{Feature, Provider};
use c9s_core::query::ResourceQuery;
use c9s_core::scope_tree::TreeRowKind;
use c9s_core::state::{AppTab, DetailTab, NavigationContext, ViewMode};
use c9s_core::{App, AppState};
use std::time::{Duration, Instant};

/// Navigation metadata key marking a view that showed all subscriptions.
const CROSS_SUBSCRIPTION: &str = "cross_subscription";
/// Navigation metadata key: the management group a view was scoped to.
const SCOPE_GROUP: &str = "scope_group";
/// Navigation metadata key: the structured resource filter, as typed.
const QUERY: &str = "query";
/// Navigation metadata key: the row that was selected, restored on going back.
const SELECTED: &str = "selected";
/// Navigation metadata key: the resource list request (JSON) a view showed,
/// reloaded on going back when the list has since been replaced.
const RESOURCES: &str = "resources";
/// Navigation metadata keys: the group whose members a list showed (and
/// "1" for nested members too), and the principal whose groups it showed.
const MEMBERS_GROUP: &str = "members_group";
const MEMBERS_TRANSITIVE: &str = "members_transitive";
const MEMBER_OF: &str = "member_of";
/// Navigation metadata keys: what a role assignment list showed (its scope
/// as `g:<group>` or `s:<subscription>`, role GUID, principal ID).
const RA_SCOPE: &str = "ra_scope";
const RA_ROLE: &str = "ra_role";
const RA_PRINCIPAL: &str = "ra_principal";
/// Navigation metadata key: the resource a detail view showed (JSON), with
/// its tab and cursor, reopened on going back from another detail view.
const DETAIL: &str = "detail";
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use tracing::{debug, info, warn};

/// TUI application state wrapper
///
/// This wraps the core application and adds TUI-specific state like quit flag.
pub struct TuiApp {
    core: App,
    should_quit: bool,
    desktop: Box<dyn Desktop>,
    /// When the current view was last loaded by auto-refresh or `r`
    last_refresh: Instant,
}

impl TuiApp {
    /// Create a new TUI application
    pub fn new(core: App) -> Self {
        Self::with_desktop(core, Box::new(SystemDesktop))
    }

    /// Like [`TuiApp::new`], with a custom clipboard/browser (for tests).
    pub fn with_desktop(core: App, desktop: Box<dyn Desktop>) -> Self {
        Self {
            core,
            should_quit: false,
            desktop,
            last_refresh: Instant::now(),
        }
    }

    /// Get the core application
    pub fn core(&self) -> &App {
        &self.core
    }

    /// Get mutable core application
    pub fn core_mut(&mut self) -> &mut App {
        &mut self.core
    }

    /// Get the application state
    pub fn state(&self) -> &AppState {
        self.core.state()
    }

    /// Get mutable application state
    pub fn state_mut(&mut self) -> &mut AppState {
        self.core.state_mut()
    }

    /// Take the channel on which background load results arrive
    pub fn take_load_receiver(&mut self) -> Option<LoadReceiver> {
        self.core.take_load_receiver()
    }

    /// Apply a finished background load
    pub fn apply_load(&mut self, result: LoadResult) {
        self.core.apply(result);
    }

    /// Check if the application should quit
    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// Handle an event
    ///
    /// Returns `Ok(true)` if the application should continue running,
    /// `Ok(false)` if it should quit.
    pub async fn handle_event(&mut self, event: Event) -> Result<bool> {
        match event {
            Event::Key(key) => self.handle_key(key).await,
            Event::Mouse(mouse) => self.handle_mouse(mouse).await,
            Event::Tick => {
                let state = self.state_mut();
                if state.loading {
                    state.spinner_frame = state.spinner_frame.wrapping_add(1);
                }
                self.tick_at(Instant::now()).await?;
                Ok(true)
            }
            Event::Resize(_, _) => {
                // Terminal resized - UI will automatically adjust
                debug!("Terminal resized");
                Ok(true)
            }
            Event::Error(err) => {
                warn!("Event error: {}", err);
                self.state_mut().set_error(Some(err));
                Ok(true)
            }
        }
    }

    /// Handle mouse input
    async fn handle_mouse(&mut self, mouse: MouseEvent) -> Result<bool> {
        // Don't handle mouse in filter or command mode
        if self.state().filter_mode || self.state().command_mode {
            return Ok(true);
        }

        match mouse.kind {
            MouseEventKind::ScrollDown => {
                self.navigate_down();
                Ok(true)
            }
            MouseEventKind::ScrollUp => {
                self.navigate_up();
                Ok(true)
            }
            _ => Ok(true),
        }
    }

    /// Handle keyboard input
    async fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        // Command mode has special handling
        if self.state().command_mode {
            return self.handle_command_key(key).await;
        }

        // Filter mode has special handling
        if self.state().filter_mode {
            return self.handle_filter_key(key).await;
        }

        // The command palette takes every key while open
        if self.state().palette.is_some() {
            return self.handle_palette_key(key).await;
        }

        // `s`: choosing the column to sort by
        if self.state().sort_picker.is_some() {
            self.handle_sort_key(key);
            return Ok(true);
        }

        // `o` asked which page to open
        if self.state().portal_choice.is_some() {
            self.handle_portal_choice_key(key);
            return Ok(true);
        }

        // Typing a search in the detail view (after `/`)
        if self.state().detail_search_input.is_some() {
            self.handle_search_key(key);
            return Ok(true);
        }

        // Help screen has special handling
        if self.state().help_open {
            return self.handle_help_key(key).await;
        }

        // Global keybindings
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('c') | KeyCode::Char('q') => {
                    info!("Quit requested");
                    self.should_quit = true;
                    return Ok(false);
                }
                KeyCode::Char('p') => {
                    debug!("Open the command palette");
                    let palette = c9s_core::palette::Palette::open(self.state());
                    self.state_mut().palette = Some(palette);
                    return Ok(true);
                }
                KeyCode::Char('r') => {
                    debug!("Force refresh requested");
                    self.handle_refresh().await?;
                    return Ok(true);
                }
                KeyCode::Char('d') => {
                    // Ctrl+D for page down
                    self.navigate_page_down();
                    return Ok(true);
                }
                KeyCode::Char('u') => {
                    // Ctrl+U for page up
                    self.navigate_page_up();
                    return Ok(true);
                }
                _ => {}
            }
        }

        // View-specific keybindings
        match key.code {
            KeyCode::Char('q') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                info!("Quit requested");
                self.should_quit = true;
                Ok(false)
            }
            KeyCode::Char(':') => {
                debug!("Enter command mode");
                self.state_mut().enter_command_mode();
                Ok(true)
            }
            KeyCode::Char('?') => {
                debug!("Show help");
                self.state_mut().toggle_help();
                Ok(true)
            }
            KeyCode::Esc => {
                let state = self.state_mut();
                if state.error.is_some() {
                    state.clear_error();
                } else if !state.filter.is_empty() {
                    state.clear_filter();
                } else if matches!(state.view, ViewMode::ResourceDetail | ViewMode::AppDetail)
                    && !state.detail_search.is_empty()
                {
                    // First Esc clears the search highlights, the next goes back.
                    state.clear_detail_search();
                    state.notice = None;
                } else {
                    self.navigate_back().await?;
                }
                Ok(true)
            }
            KeyCode::Char('/')
                if matches!(
                    self.state().view,
                    ViewMode::ResourceDetail | ViewMode::AppDetail
                ) =>
            {
                debug!("Start search");
                let state = self.state_mut();
                state.detail_search_origin = (state.detail_scroll, state.detail_match);
                state.detail_search_input = Some(String::new());
                Ok(true)
            }
            KeyCode::Char('/') => {
                debug!("Start filter");
                self.state_mut().enter_filter_mode();
                Ok(true)
            }
            KeyCode::Char('r') => {
                debug!("Refresh requested");
                self.handle_refresh().await?;
                Ok(true)
            }
            // Navigation keys
            KeyCode::Down | KeyCode::Char('j') => {
                self.navigate_down();
                Ok(true)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.navigate_up();
                Ok(true)
            }
            KeyCode::Char('g') => {
                self.navigate_first();
                Ok(true)
            }
            KeyCode::Char('G') => {
                self.navigate_last();
                Ok(true)
            }
            KeyCode::PageDown => {
                self.navigate_page_down();
                Ok(true)
            }
            KeyCode::PageUp => {
                self.navigate_page_up();
                Ok(true)
            }
            KeyCode::Enter => self.handle_select().await,
            KeyCode::Backspace => {
                self.navigate_back().await?;
                Ok(true)
            }
            _ => {
                // Pass to view-specific handlers
                self.handle_view_key(key).await
            }
        }
    }

    /// Handle keyboard input in command mode
    async fn handle_command_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc => {
                debug!("Exit command mode");
                self.state_mut().exit_command_mode();
                Ok(true)
            }
            KeyCode::Enter => {
                // A suggestion is on offer for the word being typed: Enter
                // takes it (as Tab would) and runs the command, unless it
                // still needs a value (`type=`).
                let state = self.state();
                if let Some((input, needs_more)) = complete::enter(
                    &state.command_input,
                    state.command_selection,
                    state.command_picked,
                    state,
                ) {
                    let state = self.state_mut();
                    state.command_input = input;
                    state.command_selection = 0;
                    state.command_picked = false;
                    if needs_more {
                        return Ok(true);
                    }
                }
                debug!("Execute command");
                let command_input = self.state().command_input.trim_end().to_string();
                self.state_mut().exit_command_mode();
                self.execute_command(&command_input).await?;
                Ok(true)
            }
            KeyCode::Backspace => {
                self.state_mut().command_pop();
                Ok(true)
            }
            // Autocomplete: Tab takes the highlighted suggestion; ↑/↓ pick.
            KeyCode::Tab => {
                let state = self.state();
                let completion = complete::complete(&state.command_input, state);
                if let Some(s) = completion
                    .items
                    .get(state.command_selection)
                    .or(completion.items.first())
                {
                    let input = complete::apply(&state.command_input, completion.start, &s.text);
                    let state = self.state_mut();
                    state.command_input = input;
                    state.command_selection = 0;
                    state.command_picked = false;
                }
                Ok(true)
            }
            KeyCode::Down | KeyCode::Up => {
                let state = self.state();
                let n = complete::complete(&state.command_input, state).items.len();
                if n > 0 {
                    let step = if key.code == KeyCode::Down { 1 } else { n - 1 };
                    let state = self.state_mut();
                    state.command_selection = (state.command_selection + step) % n;
                    state.command_picked = true;
                }
                Ok(true)
            }
            KeyCode::Char(c) => {
                self.state_mut().command_push(c);
                Ok(true)
            }
            _ => Ok(true),
        }
    }

    /// Keys while the palette is open: type to search, ↑/↓ (or Ctrl+P/N)
    /// to choose, Enter to go, Esc to close.
    async fn handle_palette_key(&mut self, key: KeyEvent) -> Result<bool> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(palette) = self.state_mut().palette.as_mut() else {
            return Ok(true);
        };
        match key.code {
            KeyCode::Esc => self.state_mut().palette = None,
            KeyCode::Enter => {
                let action = palette.chosen().map(|r| r.item.action.clone());
                self.state_mut().palette = None;
                if let Some(action) = action {
                    self.run_palette_action(action).await?;
                }
            }
            KeyCode::Down => palette.select(1),
            KeyCode::Up => palette.select(-1),
            KeyCode::Char('n') if ctrl => palette.select(1),
            KeyCode::Char('p') if ctrl => palette.select(-1),
            KeyCode::Char('c') if ctrl => {
                self.should_quit = true;
                return Ok(false);
            }
            KeyCode::Backspace => {
                let mut query = palette.query.clone();
                query.pop();
                palette.set_query(query);
            }
            KeyCode::Char(c) if !ctrl => {
                let query = format!("{}{c}", palette.query);
                palette.set_query(query);
            }
            _ => {}
        }
        Ok(true)
    }

    /// Go where a palette item points. Esc afterwards returns to where the
    /// palette was opened.
    async fn run_palette_action(&mut self, action: PaletteAction) -> Result<()> {
        match action {
            PaletteAction::Command(command) => self.execute_command(&command).await?,
            PaletteAction::Help => self.state_mut().toggle_help(),
            PaletteAction::Subscription(id) => {
                self.push_current_context(None);
                let sub = self
                    .state()
                    .subscriptions
                    .iter()
                    .find(|s| s.id == id)
                    .cloned();
                let state = self.state_mut();
                state.selected_subscription = sub;
                state.scope_group = None;
                self.show_resource_groups(vec![id], false);
            }
            PaletteAction::ManagementGroup(group) => {
                let ids = self.enabled_subscriptions_under(Some(&group));
                if ids.is_empty() {
                    self.state_mut().notice =
                        Some("No enabled subscriptions under that management group".into());
                    return Ok(());
                }
                self.push_current_context(None);
                let state = self.state_mut();
                state.scope_group = Some(group);
                state.selected_subscription = None;
                self.show_resource_groups(ids, true);
            }
            PaletteAction::ResourceGroup {
                subscription_id,
                name,
            } => {
                self.push_current_context(None);
                let sub = self
                    .state()
                    .subscriptions
                    .iter()
                    .find(|s| s.id.eq_ignore_ascii_case(&subscription_id))
                    .cloned();
                let state = self.state_mut();
                state.selected_subscription = sub;
                state.scope_group = None;
                self.show_resources(vec![subscription_id], Some(name), false);
            }
            PaletteAction::Resource(id) => {
                let resource = self.state().resources.iter().find(|r| r.id == id).cloned();
                if let Some(resource) = resource {
                    self.push_current_context(Some(resource.resource_group.clone()));
                    self.core.open_resource_detail(resource);
                }
            }
        }
        Ok(())
    }

    /// Execute a parsed command
    async fn execute_command(&mut self, input: &str) -> Result<()> {
        let command = Command::parse(input);

        if !command.is_valid() {
            if let Some(error_msg) = command.error_message() {
                self.state_mut().set_error(Some(error_msg));
            }
            return Ok(());
        }

        // Azure-only features say so under another cloud.
        if let Some(why) = command
            .kind
            .feature()
            .and_then(|f| self.state().provider.unsupported(f))
        {
            self.state_mut().set_error(Some(why));
            return Ok(());
        }

        // A command without a scope means "where I am": decide that before
        // the command resets the navigation.
        let context = self.current_context();
        // Commands always start a new, unscoped navigation.
        self.state_mut().scope_group = None;
        let mut command = command;
        match &mut command.kind {
            CommandKind::ResourceGroups { scope, .. }
            | CommandKind::Resources { scope, .. }
            | CommandKind::ResourceTypes { scope, .. }
            | CommandKind::Tags { scope, .. }
            | CommandKind::Policy { scope }
            | CommandKind::Compliance { scope }
            | CommandKind::Exemption { scope }
            | CommandKind::RoleAssignments { scope }
                if *scope == Scope::Current =>
            {
                *scope = context;
            }
            _ => {}
        }
        match command.kind {
            CommandKind::Policy { scope } => {
                debug!("Command: policy, scope {:?}", scope);
                if let Some(policy_scope) = self.policy_scope(&scope) {
                    self.state_mut().clear_navigation();
                    self.core.open_policy(policy_scope);
                }
            }
            CommandKind::RoleAssignments { scope } => {
                debug!("Command: role assignments, scope {:?}", scope);
                self.open_role_assignments(scope);
            }
            CommandKind::Access(query) => {
                debug!("Command: access {}", query);
                match self.find_principal(&query) {
                    Some(id) => {
                        let state = self.state_mut();
                        state.clear_navigation();
                        state.filter.clear();
                        self.core.open_access(&id);
                    }
                    None => self.state_mut().set_error(Some(format!(
                        "No principal named {query} among those c9s knows.\n\n\
                         Give the principal's object ID, or open :ra all first so \
                         names are loaded, then press p on one of its rows."
                    ))),
                }
            }
            CommandKind::Theme(name) => {
                use c9s_core::config::ThemeName;
                let names = ThemeName::ALL
                    .iter()
                    .map(|t| t.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                match name.as_deref().map(ThemeName::parse) {
                    None => {
                        let current = self.state().config.theme.as_str();
                        self.state_mut().notice = Some(format!("Theme {current}. Themes: {names}"));
                    }
                    Some(Some(theme)) => {
                        crate::ui::theme::set_theme(theme);
                        let state = self.state_mut();
                        state.config.theme = theme;
                        state.notice = Some(format!(
                            "Theme {} (to keep it, set theme = \"{}\" in the config file)",
                            theme.as_str(),
                            theme.as_str()
                        ));
                    }
                    Some(None) => self.state_mut().set_error(Some(format!(
                        "No theme named {}.\n\nThemes: {names}",
                        name.unwrap_or_default()
                    ))),
                }
            }
            CommandKind::Apps(name) => {
                debug!("Command: applications {:?}", name);
                let state = self.state_mut();
                state.clear_navigation();
                state.filter = name.unwrap_or_default();
                self.core.open_apps();
            }
            CommandKind::Groups(name) => {
                debug!("Command: groups {:?}", name);
                let state = self.state_mut();
                state.clear_navigation();
                state.filter = name.unwrap_or_default();
                self.core.open_groups();
            }
            CommandKind::RoleDefinitions => {
                debug!("Command: role definitions");
                let state = self.state_mut();
                state.clear_navigation();
                state.filter.clear();
                self.core.open_role_definitions();
            }
            CommandKind::Exemption { scope } => {
                debug!("Command: exemption, scope {:?}", scope);
                self.open_exemptions(scope);
            }
            CommandKind::Compliance { scope } => {
                debug!("Command: compliance, scope {:?}", scope);
                self.open_compliance(scope);
            }
            CommandKind::Context(None) => {
                debug!("Command: tenants");
                let state = self.state_mut();
                state.clear_navigation();
                state.view = ViewMode::TenantList;
                state.list_state.reset();
                // Azure tenants (from Azure), and gcloud's configurations.
                if self.state().provider == Provider::Azure {
                    self.core.request(LoadRequest::Tenants);
                }
                self.core.request(LoadRequest::CloudContexts);
            }
            CommandKind::Context(Some(arg)) => {
                if arg.eq_ignore_ascii_case("all") {
                    self.core.switch_tenant(None);
                } else {
                    let gcp = self
                        .state()
                        .gcp_configurations
                        .iter()
                        .find(|c| c.name == arg)
                        .map(|c| c.name.clone());
                    // AWS profiles are read from the CLI's files, so they
                    // work before :ctx has listed them.
                    let aws = c9s_core::aws::profiles()
                        .into_iter()
                        .chain(self.state().aws_profiles.clone())
                        .find(|p| p.name == arg)
                        .map(|p| p.name);
                    match (gcp, aws, self.find_tenant(&arg)) {
                        (Some(configuration), _, _) => self.core.switch_to_gcp(Some(configuration)),
                        (None, Some(profile), _) => self.core.switch_to_aws(profile),
                        (None, None, Some(id)) => self.core.switch_tenant(Some(id)),
                        (None, None, None) => self.state_mut().set_error(Some(format!(
                            "Context not found: {arg}\n\nType :ctx to list the Azure tenants, \
                             gcloud configurations and AWS profiles you can use."
                        ))),
                    }
                }
            }
            CommandKind::ManagementGroups => {
                debug!("Command: management group tree");
                self.state_mut().clear_navigation();
                self.core.open_scope_tree();
            }
            CommandKind::Subscriptions => {
                debug!("Command: Navigate to subscriptions");
                let state = self.state_mut();
                state.view = ViewMode::SubscriptionList;
                state.list_state.reset();
                state.clear_navigation();
            }
            CommandKind::ResourceGroups { scope, query } => {
                debug!("Command: resource groups, scope {:?}", scope);
                if let Some(subscription_ids) = self.resolve_scope(&scope) {
                    self.state_mut().clear_navigation();
                    self.show_resource_groups(subscription_ids, scope.is_cross_subscription());
                    self.state_mut().resource_query = query;
                }
            }
            CommandKind::Resources { scope, query } => {
                debug!("Command: resources, scope {:?}, filter {}", scope, query);
                if let Some(subscription_ids) = self.resolve_scope(&scope) {
                    self.state_mut().clear_navigation();
                    self.show_resources(subscription_ids, None, scope.is_cross_subscription());
                    self.state_mut().resource_query = query;
                }
            }
            CommandKind::ResourceTypes { scope, query } => {
                debug!(
                    "Command: resource types, scope {:?}, filter {}",
                    scope, query
                );
                if let Some(subscription_ids) = self.resolve_scope(&scope) {
                    self.state_mut().clear_navigation();
                    self.show_resources(subscription_ids, None, scope.is_cross_subscription());
                    let state = self.state_mut();
                    state.resource_query = query;
                    state.view = ViewMode::ResourceTypeSummary;
                }
            }
            CommandKind::Tags { scope, query } => {
                debug!("Command: tags, scope {:?}, filter {}", scope, query);
                if let Some(subscription_ids) = self.resolve_scope(&scope) {
                    self.state_mut().clear_navigation();
                    self.show_resources(subscription_ids, None, scope.is_cross_subscription());
                    let state = self.state_mut();
                    state.resource_query = query;
                    state.view = ViewMode::TagSummary;
                }
            }
            CommandKind::Invalid(_) => {
                // Already handled above
            }
            CommandKind::Unknown(_) => {
                // Already handled above
            }
        }

        Ok(())
    }

    /// Turn a command scope into subscription IDs. For a single subscription,
    /// also makes it the selected one. Shows an error and returns `None` if the
    /// subscription is unknown.
    fn resolve_scope(&mut self, scope: &Scope) -> Option<Vec<String>> {
        match scope {
            Scope::All | Scope::Current => Some(self.enabled_subscription_ids()),
            Scope::Group(group) => {
                let ids = self.enabled_subscriptions_under(Some(group));
                if ids.is_empty() {
                    self.state_mut()
                        .set_error(Some(format!("No enabled subscriptions under {group}")));
                    return None;
                }
                let state = self.state_mut();
                state.scope_group = Some(group.clone());
                state.selected_subscription = None;
                Some(ids)
            }
            Scope::Subscription(name) => match self.find_subscription(name).cloned() {
                Some(sub) => {
                    let id = sub.id.clone();
                    self.state_mut().selected_subscription = Some(sub);
                    Some(vec![id])
                }
                None => {
                    self.state_mut()
                        .set_error(Some(format!("Subscription not found: {}", name)));
                    None
                }
            },
        }
    }

    /// The policy scope for a command scope: one subscription, a management
    /// group, or (for `all`) the tenant root group. Shows an error and
    /// returns `None` when that can't be determined.
    fn policy_scope(&mut self, scope: &Scope) -> Option<PolicyScope> {
        match scope {
            Scope::Group(group) => Some(PolicyScope::Group(group.clone())),
            Scope::Subscription(name) => match self.find_subscription(name).cloned() {
                Some(sub) => {
                    let id = sub.id.clone();
                    self.state_mut().selected_subscription = Some(sub);
                    Some(PolicyScope::Subscription(id))
                }
                None => {
                    self.state_mut()
                        .set_error(Some(format!("Subscription not found: {name}")));
                    None
                }
            },
            Scope::All | Scope::Current => match &self.state().scope_tree {
                Some(tree) => Some(PolicyScope::Group(tree.root.clone())),
                None => {
                    self.state_mut().set_error(Some(
                        "Policy for all subscriptions needs the management group tree, \
                         which isn't loaded (see :mg). Name a subscription instead: \
                         :policy <subscription>"
                            .into(),
                    ));
                    None
                }
            },
        }
    }

    /// `:compliance`: the subscriptions of a management group (or the whole
    /// tenant) ranked by non-compliant resources. A subscription scope ranks
    /// the whole tenant with that subscription selected.
    fn open_compliance(&mut self, scope: Scope) {
        let (group, select) = match scope {
            Scope::All | Scope::Current => (None, None),
            Scope::Group(group) => (Some(group), None),
            Scope::Subscription(name) => match self.find_group(&name) {
                Some(group) => (Some(group), None),
                None => match self.find_subscription(&name) {
                    Some(sub) => (None, Some(sub.id.to_ascii_lowercase())),
                    None => {
                        self.state_mut().set_error(Some(format!(
                            "No management group or subscription named {name}"
                        )));
                        return;
                    }
                },
            },
        };
        let state = self.state_mut();
        state.clear_navigation();
        // The tenant root group is the whole tenant.
        let root = state.scope_tree.as_ref().map(|t| t.root.clone());
        state.compliance_group = group.filter(|g| Some(g) != root.as_ref());
        state.filter.clear();
        state.view = ViewMode::ComplianceRanking;
        state.list_state.reset();
        self.core.load_compliance_rollup();
        if let Some(id) = select {
            self.state_mut().select_id(&format!("/subscriptions/{id}"));
        }
    }

    /// A principal's ID (lowercase) by its name, sign-in name, app ID or
    /// object ID: exact matches first, then a unique partial match. An ID
    /// that looks like a GUID is taken as is.
    fn find_principal(&self, query: &str) -> Option<String> {
        let q = query.trim().to_ascii_lowercase();
        if q.is_empty() {
            return None;
        }
        let principals = &self.state().principals;
        let fields = |p: &c9s_core::rbac::Principal| {
            [
                p.id.to_ascii_lowercase(),
                p.display_name.to_ascii_lowercase(),
                p.detail.to_ascii_lowercase(),
            ]
        };
        if let Some(p) = principals.values().find(|p| fields(p).contains(&q)) {
            return Some(p.id.to_ascii_lowercase());
        }
        let partial: Vec<&c9s_core::rbac::Principal> = principals
            .values()
            .filter(|p| fields(p).iter().any(|f| f.contains(&q)))
            .collect();
        if let [only] = partial.as_slice() {
            return Some(only.id.to_ascii_lowercase());
        }
        let is_guid = q.len() == 36 && q.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
        is_guid.then_some(q)
    }

    /// A management group by name or display name (case-insensitive).
    fn find_group(&self, name: &str) -> Option<String> {
        let tree = self.state().scope_tree.as_ref()?;
        if tree.root.eq_ignore_ascii_case(name) || tree.root_display_name.eq_ignore_ascii_case(name)
        {
            return Some(tree.root.clone());
        }
        tree.groups
            .iter()
            .find(|g| {
                g.name.eq_ignore_ascii_case(name) || g.display_name.eq_ignore_ascii_case(name)
            })
            .map(|g| g.name.clone())
    }

    /// What a command without a scope applies to: the highlighted
    /// subscription or management group in the list/tree, else the
    /// management group or subscription the current view is scoped to, else
    /// all subscriptions.
    fn current_context(&self) -> Scope {
        let state = self.state();
        match state.view {
            ViewMode::SubscriptionList => {
                if let Some(sub) = state.selected_subscription() {
                    return Scope::Subscription(sub.id.clone());
                }
            }
            ViewMode::ManagementGroupTree => {
                if let Some(row) = state.selected_tree_row() {
                    return match row.kind {
                        TreeRowKind::Subscription => Scope::Subscription(row.key),
                        TreeRowKind::ManagementGroup => Scope::Group(row.key),
                        TreeRowKind::Unplaced => Scope::All,
                    };
                }
            }
            _ => {}
        }
        if state.cross_subscription_mode {
            return match &state.scope_group {
                Some(group) => Scope::Group(group.clone()),
                None => Scope::All,
            };
        }
        match (
            &state.selected_subscription,
            &state.config.default_subscription,
        ) {
            (Some(sub), _) => Scope::Subscription(sub.id.clone()),
            // No context: the config's default subscription, if any.
            (None, Some(name)) => Scope::Subscription(name.clone()),
            (None, None) => Scope::All,
        }
    }

    /// A tenant by ID, name or domain (ignoring case), among the tenants
    /// known from Azure and from subscriptions. A GUID-looking ID is taken
    /// as is, so a tenant can be used before its name is known.
    fn find_tenant(&self, wanted: &str) -> Option<String> {
        let found = self.state().tenant_rows().into_iter().find_map(|row| {
            let id = row.id?;
            let hit = id.eq_ignore_ascii_case(wanted)
                || row.name.eq_ignore_ascii_case(wanted)
                || row
                    .domain
                    .as_deref()
                    .is_some_and(|d| d.eq_ignore_ascii_case(wanted));
            hit.then_some(id)
        });
        found.or_else(|| {
            let guid =
                wanted.len() == 36 && wanted.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
            guid.then(|| wanted.to_ascii_lowercase())
        })
    }

    /// IDs of all subscriptions that can be queried in the current tenant
    /// (disabled ones always fail).
    fn enabled_subscription_ids(&self) -> Vec<String> {
        let state = self.state();
        state
            .subscriptions
            .iter()
            .filter(|s| state.in_tenant(s))
            .filter(|s| !s.state.eq_ignore_ascii_case("Disabled"))
            .map(|s| s.id.clone())
            .collect()
    }

    /// IDs of the enabled subscriptions a cross-subscription view covers: all,
    /// or those below the management group the view was opened from.
    fn scope_subscription_ids(&self) -> Vec<String> {
        self.enabled_subscriptions_under(self.state().scope_group.as_deref())
    }

    /// Enabled subscriptions below `group` (all of them for `None`).
    fn enabled_subscriptions_under(&self, group: Option<&str>) -> Vec<String> {
        let state = self.state();
        let (Some(group), Some(tree)) = (group, &state.scope_tree) else {
            return self.enabled_subscription_ids();
        };
        let disabled: std::collections::HashSet<String> = state
            .subscriptions
            .iter()
            .filter(|s| s.state.eq_ignore_ascii_case("Disabled"))
            .map(|s| s.id.to_ascii_lowercase())
            .collect();
        tree.subscriptions_under(group, &state.subscriptions)
            .into_iter()
            .filter(|id| !disabled.contains(id))
            .collect()
    }

    /// Switch to the resource group list now and load it in the background.
    fn show_resource_groups(&mut self, subscription_ids: Vec<String>, all: bool) {
        let state = self.state_mut();
        state.view = ViewMode::ResourceGroupList;
        state.cross_subscription_mode = all;
        state.resource_query = Default::default();
        state.resource_groups.clear();
        state.list_state.reset();
        self.core
            .request(LoadRequest::ResourceGroups { subscription_ids });
    }

    /// Switch to the resource list now and load it in the background.
    fn show_resources(
        &mut self,
        subscription_ids: Vec<String>,
        resource_group: Option<String>,
        all: bool,
    ) {
        let state = self.state_mut();
        state.view = ViewMode::ResourceList;
        state.cross_subscription_mode = all;
        state.current_resource_group = resource_group.clone();
        state.resource_query = Default::default();
        state.resources.clear();
        state.list_state.reset();
        self.core.request(LoadRequest::Resources {
            subscription_ids,
            resource_group,
        });
    }

    /// Find a subscription by name or ID
    fn find_subscription(&self, name_or_id: &str) -> Option<&c9s_core::models::Subscription> {
        let search = name_or_id.to_lowercase();
        self.state()
            .subscriptions
            .iter()
            .find(|s| s.name.to_lowercase() == search || s.id.to_lowercase().contains(&search))
    }

    /// Handle keyboard input in filter mode
    async fn handle_filter_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc => {
                debug!("Exit filter mode");
                self.state_mut().exit_filter_mode();
                Ok(true)
            }
            KeyCode::Enter => {
                debug!("Apply filter and exit filter mode");
                self.state_mut().filter_mode = false;
                Ok(true)
            }
            KeyCode::Backspace => {
                self.state_mut().filter_pop();
                Ok(true)
            }
            KeyCode::Char(c) => {
                self.state_mut().filter_push(c);
                Ok(true)
            }
            _ => Ok(true),
        }
    }

    /// Handle keyboard input in help screen
    async fn handle_help_key(&mut self, key: KeyEvent) -> Result<bool> {
        let state = self.state_mut();
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                debug!("Close help");
                state.toggle_help();
                state.help_scroll = 0;
            }
            // Scrolling; the drawing clamps the end.
            KeyCode::Down | KeyCode::Char('j') => state.help_scroll += 1,
            KeyCode::Up | KeyCode::Char('k') => {
                state.help_scroll = state.help_scroll.saturating_sub(1)
            }
            KeyCode::PageDown => state.help_scroll += 10,
            KeyCode::Char('d') if ctrl => state.help_scroll += 10,
            KeyCode::PageUp => state.help_scroll = state.help_scroll.saturating_sub(10),
            KeyCode::Char('u') if ctrl => state.help_scroll = state.help_scroll.saturating_sub(10),
            KeyCode::Char('g') => state.help_scroll = 0,
            KeyCode::Char('G') => state.help_scroll = usize::MAX / 2,
            _ => {}
        }
        Ok(true)
    }

    /// Auto-refresh (`refresh_interval` in the config): reload the current
    /// view when the interval has passed since the last load, unless a load
    /// is running or the user is typing, searching or reading help.
    pub async fn tick_at(&mut self, now: Instant) -> Result<()> {
        let state = self.state();
        let interval = state.config.refresh_interval;
        let busy = state.loading
            || state.command_mode
            || state.filter_mode
            || state.help_open
            || state.palette.is_some()
            || state.detail_search_input.is_some()
            || state.error.is_some();
        let refreshable = matches!(
            state.view,
            ViewMode::ManagementGroupTree
                | ViewMode::SubscriptionList
                | ViewMode::ResourceGroupList
                | ViewMode::ResourceList
                | ViewMode::ResourceTypeSummary
                | ViewMode::TagSummary
                | ViewMode::ResourceDetail
        );
        if interval > 0
            && !busy
            && refreshable
            && now.duration_since(self.last_refresh) >= Duration::from_secs(interval)
        {
            debug!("Auto-refresh after {interval}s");
            self.handle_refresh().await?;
        }
        Ok(())
    }

    /// Reload the current view in the background
    async fn handle_refresh(&mut self) -> Result<()> {
        self.last_refresh = Instant::now();
        let state = self.state();
        let scope_ids = if state.cross_subscription_mode {
            Some(self.scope_subscription_ids())
        } else {
            state
                .selected_subscription
                .as_ref()
                .map(|s| vec![s.id.clone()])
        };

        match (state.view.clone(), scope_ids) {
            (ViewMode::SubscriptionList, _) => {
                info!("Refreshing subscriptions");
                self.core.request(LoadRequest::Subscriptions);
            }
            (ViewMode::ResourceDetail, _) => {
                self.core.refresh_resource_detail();
                if self.state().detail_tab == DetailTab::Policy {
                    self.core.load_resource_policy(true);
                }
            }
            (ViewMode::PolicyNonCompliant, _) => {
                if let Some(a) = self.state().policy_states_assignment.clone() {
                    self.core.open_non_compliant(a);
                }
            }
            (ViewMode::TenantList, _) => self.core.request(LoadRequest::Tenants),
            (ViewMode::PolicyAssignments, _) => {
                if let Some(scope) = self.state().policy_scope.clone() {
                    self.core.open_policy(scope);
                }
            }
            (ViewMode::ManagementGroupTree, _) => {
                info!("Refreshing the management group tree");
                self.core.request(LoadRequest::ScopeTree);
                self.core.request(LoadRequest::ComplianceRollup);
            }
            (ViewMode::GroupList, _) => self.core.request(LoadRequest::Groups),
            (ViewMode::AppList, _) => self.core.request(LoadRequest::Applications),
            (ViewMode::AppDetail, _) => self.core.refresh_app(),
            (ViewMode::GroupMembers, _) => {
                let state = self.state();
                if let Some(group) = state.members_group.clone() {
                    let transitive = state.members_transitive;
                    self.core.open_group_members(&group, transitive);
                }
            }
            (ViewMode::MemberOf, _) => {
                if let Some(principal) = self.state().member_of.clone() {
                    self.core.open_member_of(&principal);
                }
            }
            (ViewMode::RoleAssignments | ViewMode::RoleDefinitions, _) => {
                self.core.load_rbac();
            }
            (ViewMode::ExemptionList, _) => {
                self.core.request(LoadRequest::PolicyExemptions);
            }
            (ViewMode::ComplianceRanking, _) => {
                self.core.request(LoadRequest::ComplianceRollup);
            }
            (ViewMode::ResourceGroupList, Some(subscription_ids)) => {
                self.core
                    .request(LoadRequest::ResourceGroups { subscription_ids });
            }
            (
                ViewMode::ResourceList | ViewMode::ResourceTypeSummary | ViewMode::TagSummary,
                Some(subscription_ids),
            ) => {
                let resource_group = self.state().current_resource_group.clone();
                self.core.request(LoadRequest::Resources {
                    subscription_ids,
                    resource_group,
                });
            }
            _ => debug!("Refresh not available for current view"),
        }
        Ok(())
    }

    /// Navigate back in the navigation stack
    async fn navigate_back(&mut self) -> Result<()> {
        let state = self.state_mut();
        if let Some(ctx) = state.pop_navigation() {
            debug!("Navigate back to {:?}", ctx.view);
            state.view = ctx.view;
            state.list_state.reset();
            state.cross_subscription_mode = ctx.metadata.contains_key(CROSS_SUBSCRIPTION);
            state.scope_group = ctx.metadata.get(SCOPE_GROUP).cloned();
            // A group's members, or a principal's groups: whose.
            if state.view == ViewMode::GroupMembers {
                state.members_group = ctx.metadata.get(MEMBERS_GROUP).cloned();
                state.members_transitive = ctx.metadata.contains_key(MEMBERS_TRANSITIVE);
            }
            if state.view == ViewMode::MemberOf {
                state.member_of = ctx.metadata.get(MEMBER_OF).cloned();
            }
            // A role assignment list: the scope, role and principal it showed.
            if state.view == ViewMode::RoleAssignments {
                state.ra_scope =
                    ctx.metadata
                        .get(RA_SCOPE)
                        .and_then(|s| match s.split_once(':')? {
                            ("g", g) => Some(PolicyScope::Group(g.to_string())),
                            ("s", s) => Some(PolicyScope::Subscription(s.to_string())),
                            _ => None,
                        });
                state.ra_role = ctx.metadata.get(RA_ROLE).cloned();
                state.ra_principal = ctx.metadata.get(RA_PRINCIPAL).cloned();
            }
            // Back to the policy view keeps its scope (still in state).
            state.resource_query = ctx
                .metadata
                .get(QUERY)
                .and_then(|q| ResourceQuery::parse(q.split_whitespace()).ok())
                .unwrap_or_default();
            // The list was replaced since (a tag's resources from a detail
            // view): load what it showed, from the cache at once.
            let reload = ctx
                .metadata
                .get(RESOURCES)
                .and_then(|r| serde_json::from_str::<(Vec<String>, Option<String>)>(r).ok())
                .map(
                    |(subscription_ids, resource_group)| LoadRequest::Resources {
                        subscription_ids,
                        resource_group,
                    },
                )
                .filter(|r| self.core.resources_request() != Some(r));
            if let Some(request) = reload {
                if let LoadRequest::Resources { resource_group, .. } = &request {
                    let state = self.state_mut();
                    state.current_resource_group = resource_group.clone();
                    state.resources.clear();
                }
                self.core.request(request);
            }
            // Back from another detail view (a policy definition): reopen
            // the resource, on the tab and row it was left at.
            let detail = ctx.metadata.get(DETAIL).and_then(|d| {
                serde_json::from_str::<(c9s_core::models::Resource, usize, usize)>(d).ok()
            });
            if let Some((resource, tab, cursor)) = detail {
                self.core.open_resource_detail(resource);
                let state = self.state_mut();
                state.detail_tab = DetailTab::ALL.get(tab).copied().unwrap_or_default();
                state.detail_cursor = cursor;
                state.detail_scroll_to = Some(ui::detail_row_line(state.detail_tab, cursor));
                if state.detail_tab == DetailTab::Policy {
                    self.core.load_resource_policy(false);
                }
            }
            let state = self.state_mut();
            if let Some(id) = ctx.metadata.get(SELECTED) {
                state.select_id(id);
            }
        }
        Ok(())
    }

    /// Navigate down in the current list
    fn navigate_down(&mut self) {
        if matches!(
            self.state().view,
            ViewMode::ResourceDetail | ViewMode::AppDetail
        ) {
            if !self.move_detail_cursor(|c, _| c + 1) {
                self.scroll_detail(1);
            }
            return;
        }
        let state = self.state_mut();
        let list_len = state.current_list_len();
        if list_len > 0 {
            state.list_state.select_next(list_len);
        }
    }

    /// Navigate up in the current list
    fn navigate_up(&mut self) {
        if matches!(
            self.state().view,
            ViewMode::ResourceDetail | ViewMode::AppDetail
        ) {
            if !self.move_detail_cursor(|c, _| c.saturating_sub(1)) {
                self.scroll_detail(-1);
            }
            return;
        }
        let state = self.state_mut();
        if state.current_list_len() > 0 {
            state.list_state.select_previous();
        }
    }

    /// Navigate to first item
    fn navigate_first(&mut self) {
        if matches!(
            self.state().view,
            ViewMode::ResourceDetail | ViewMode::AppDetail
        ) {
            if !self.move_detail_cursor(|_, _| 0) {
                self.state_mut().detail_scroll = 0;
            }
            return;
        }
        let state = self.state_mut();
        if state.current_list_len() > 0 {
            state.list_state.select_first();
        }
    }

    /// Navigate to last item
    fn navigate_last(&mut self) {
        if matches!(
            self.state().view,
            ViewMode::ResourceDetail | ViewMode::AppDetail
        ) {
            if !self.move_detail_cursor(|_, n| n) {
                self.state_mut().detail_scroll = usize::MAX;
            }
            return;
        }
        let state = self.state_mut();
        let list_len = state.current_list_len();
        if list_len > 0 {
            state.list_state.select_last(list_len);
        }
    }

    /// Navigate page down (viewport-aware)
    fn navigate_page_down(&mut self) {
        if matches!(
            self.state().view,
            ViewMode::ResourceDetail | ViewMode::AppDetail
        ) {
            self.scroll_detail(10);
            return;
        }
        let state = self.state_mut();
        let list_len = state.current_list_len();
        if list_len > 0 {
            // Jump by 10 items or to the end
            let target = (state.list_state.selected + 10).min(list_len - 1);
            state.list_state.selected = target;
        }
    }

    /// Navigate page up (viewport-aware)
    fn navigate_page_up(&mut self) {
        if matches!(
            self.state().view,
            ViewMode::ResourceDetail | ViewMode::AppDetail
        ) {
            self.scroll_detail(-10);
            return;
        }
        let state = self.state_mut();
        if state.current_list_len() > 0 {
            // Jump by 10 items or to the start
            state.list_state.selected = state.list_state.selected.saturating_sub(10);
        }
    }

    /// Scroll the detail view by `lines` (the drawing clamps the end).
    /// Move the cursor of a detail tab with rows (Tags, Policy) and keep it
    /// on screen. `false` when the tab has no rows (the caller scrolls).
    fn move_detail_cursor(&mut self, to: impl Fn(usize, usize) -> usize) -> bool {
        // An application's Overview: the cursor walks its fields.
        if self.state().view == ViewMode::AppDetail
            && self.state().app_tab == c9s_core::state::AppTab::Overview
        {
            let fields = c9s_core::app_view::overview_fields(self.state());
            if fields.is_empty() {
                return false;
            }
            let state = self.state_mut();
            state.app_field = to(state.app_field, fields.len()).min(fields.len() - 1);
            state.detail_scroll_to = Some(fields[state.app_field].line);
            return true;
        }
        let state = self.state_mut();
        let Some(n) = state.detail_rows().filter(|n| *n > 0) else {
            return false;
        };
        state.detail_cursor = to(state.detail_cursor, n).min(n - 1);
        state.detail_scroll_to = Some(ui::detail_row_line(state.detail_tab, state.detail_cursor));
        true
    }

    /// `Enter` in the detail view: the tag's resources across the tenant, or
    /// the policy's definition.
    fn open_detail_row(&mut self) {
        if let Some((key, value)) = self.state().selected_detail_tag() {
            self.open_tag_resources(key, value);
        } else if let Some(policy) = self.state().selected_detail_policy() {
            // An exempt policy: the exemption explains it.
            match self.state().exemption_for(&policy).cloned() {
                Some(exemption) if policy.state.eq_ignore_ascii_case("Exempt") => {
                    self.open_from_detail(exemption_resource(&exemption))
                }
                _ => self.open_policy_definition(policy),
            }
        }
    }

    /// Every resource in the tenant with `key=value` (from a detail view's
    /// Tags tab). The lists further back show other resources: they are
    /// reloaded on the way back.
    fn open_tag_resources(&mut self, key: String, value: String) {
        if let Some(LoadRequest::Resources {
            subscription_ids,
            resource_group,
        }) = self.core.resources_request().cloned()
        {
            if let Ok(encoded) = serde_json::to_string(&(subscription_ids, resource_group)) {
                for ctx in self.state_mut().navigation_stack.iter_mut() {
                    let shows_resources = matches!(
                        ctx.view,
                        ViewMode::ResourceList
                            | ViewMode::ResourceTypeSummary
                            | ViewMode::TagSummary
                    );
                    if shows_resources && !ctx.metadata.contains_key(RESOURCES) {
                        ctx.metadata.insert(RESOURCES.into(), encoded.clone());
                    }
                }
            }
        }
        self.push_current_context(None);
        let ids = self.enabled_subscription_ids();
        self.show_resources(ids, None, true);
        let state = self.state_mut();
        state.resource_query.tags = vec![(key.clone(), Some(value.clone()))];
        state.notice = Some(format!(
            "Resources tagged {key}={value} in every subscription"
        ));
    }

    /// The definition of a policy from the detail view's Policy tab.
    fn open_policy_definition(&mut self, policy: c9s_core::policy::PolicyState) {
        if policy.definition_id.is_empty() {
            self.state_mut().notice = Some("Azure did not report this policy's definition".into());
            return;
        }
        let set = policy
            .definition_id
            .to_ascii_lowercase()
            .contains("/policysetdefinitions/");
        self.open_from_detail(c9s_core::models::Resource {
            name: policy.policy_label().to_string(),
            id: policy.definition_id,
            resource_type: if set {
                "Microsoft.Authorization/policySetDefinitions"
            } else {
                "Microsoft.Authorization/policyDefinitions"
            }
            .into(),
            location: String::new(),
            resource_group: String::new(),
            // Definitions live at the tenant or a management group.
            subscription_id: String::new(),
            tags: Default::default(),
        });
    }

    /// Open another object's detail from the detail view; Esc comes back to
    /// this resource, on the same tab and row.
    fn open_from_detail(&mut self, resource: c9s_core::models::Resource) {
        let state = self.state();
        let back = state.selected_resource_detail.as_ref().and_then(|r| {
            serde_json::to_string(&(r, state.detail_tab.index(), state.detail_cursor)).ok()
        });
        self.push_current_context(None);
        if let (Some(back), Some(ctx)) = (back, self.state_mut().navigation_stack.last_mut()) {
            ctx.metadata.insert(DETAIL.into(), back);
        }
        self.core.open_resource_detail(resource);
    }

    /// `:exemption`: the exemptions at or below a scope (a management group
    /// or subscription by name; `all` or no context: the whole tenant).
    fn open_exemptions(&mut self, scope: Scope) {
        let Some(scope) = self.listing_scope(scope) else {
            return;
        };
        let state = self.state_mut();
        state.clear_navigation();
        state.filter.clear();
        self.core.open_exemptions(scope);
    }

    /// `:ra`: the role assignments that matter at a scope (a management
    /// group or subscription by name; `all` or no context: the tenant).
    fn open_role_assignments(&mut self, scope: Scope) {
        let Some(scope) = self.listing_scope(scope) else {
            return;
        };
        let state = self.state_mut();
        state.clear_navigation();
        state.filter.clear();
        self.core.open_role_assignments(scope, None);
    }

    /// The scope a tenant-wide list narrows to: `Ok(None)` for the whole
    /// tenant (also the tenant root group). `None` when the name matched
    /// nothing (the error is shown).
    fn listing_scope(&mut self, scope: Scope) -> Option<Option<PolicyScope>> {
        let scope = match scope {
            Scope::All => None,
            Scope::Current => None,
            Scope::Group(group) => Some(PolicyScope::Group(group)),
            Scope::Subscription(name) => match self.find_group(&name) {
                Some(group) => Some(PolicyScope::Group(group)),
                None => match self.find_subscription(&name) {
                    Some(sub) => Some(PolicyScope::Subscription(sub.id.clone())),
                    None => {
                        self.state_mut().set_error(Some(format!(
                            "No management group or subscription named {name}"
                        )));
                        return None;
                    }
                },
            },
        };
        // The tenant root group is the whole tenant.
        let root = self.state().scope_tree.as_ref().map(|t| t.root.clone());
        Some(scope.filter(|s| !matches!(s, PolicyScope::Group(g) if Some(g) == root.as_ref())))
    }

    /// A role definition as a detail-view item: the loaded definition (whose
    /// ID Resource Graph knows), else the ID the assignment names.
    fn role_definition_resource(
        &self,
        guid: &str,
        fallback_id: &str,
    ) -> c9s_core::models::Resource {
        let (id, name) = match self.state().role_definition(guid) {
            Some(d) => (d.id.clone(), d.role_name.clone()),
            None => (fallback_id.to_string(), guid.to_string()),
        };
        c9s_core::models::Resource {
            id,
            name,
            resource_type: "Microsoft.Authorization/roleDefinitions".into(),
            location: String::new(),
            resource_group: String::new(),
            // Definitions are found across the tenant.
            subscription_id: String::new(),
            tags: Default::default(),
        }
    }

    /// `d` in `:ra` (the assignment itself) and `:role` (the definition).
    fn open_rbac_detail(&mut self) {
        // A PIM eligibility isn't a role assignment resource: say what it
        // is instead of failing to load it.
        if let Some(row) = self.state().selected_role_assignment() {
            if row.assignment.eligible {
                let ends = match row.assignment.ends.get(..10) {
                    Some(day) => format!("until {day}"),
                    None => "permanently".into(),
                };
                self.state_mut().notice = Some(format!(
                    "PIM eligible {ends}: {} can activate {} at {} (Entra PIM in the Portal)",
                    row.principal_name, row.role_name, row.scope_label
                ));
                return;
            }
        }
        let resource = match self.state().view {
            ViewMode::RoleAssignments => self.state().selected_role_assignment().map(|r| {
                let a = r.assignment;
                let scope = a.scope.to_ascii_lowercase();
                c9s_core::models::Resource {
                    name: format!("{} for {}", r.role_name, a.principal_id),
                    subscription_id: scope
                        .strip_prefix("/subscriptions/")
                        .map(|s| s.split('/').next().unwrap_or(s).to_string())
                        .unwrap_or_default(),
                    id: a.id,
                    resource_type: "Microsoft.Authorization/roleAssignments".into(),
                    location: String::new(),
                    resource_group: String::new(),
                    tags: Default::default(),
                }
            }),
            ViewMode::RoleDefinitions => self
                .state()
                .selected_role_definition()
                .map(|r| self.role_definition_resource(&r.definition.guid(), &r.definition.id)),
            _ => None,
        };
        if let Some(resource) = resource {
            self.push_current_context(None);
            self.core.open_resource_detail(resource);
        }
    }

    fn scroll_detail(&mut self, lines: isize) {
        let state = self.state_mut();
        state.detail_scroll = state.detail_scroll.saturating_add_signed(lines);
    }

    /// Copy the selected item's ARM ID to the clipboard (`y`).
    fn copy_selected_id(&mut self) {
        if self.copy_app_field() {
            return;
        }
        let Some(id) = self.state().selected_arm_id() else {
            return;
        };
        // An ID is pasted into shells: no control characters (a newline
        // would run what follows it).
        let id = c9s_core::safe_text::clean(&id).into_owned();
        let notice = match self.desktop.copy(&id) {
            Ok(()) => format!("Copied {id}"),
            Err(e) => format!("Could not copy ({e}): {id}"),
        };
        self.state_mut().notice = Some(notice);
    }

    /// On an application's Overview, copy the selected field (`y`, `c`).
    /// Returns whether it did.
    fn copy_app_field(&mut self) -> bool {
        if self.state().view != ViewMode::AppDetail
            || self.state().app_tab != c9s_core::state::AppTab::Overview
        {
            return false;
        }
        let fields = c9s_core::app_view::overview_fields(self.state());
        let Some(field) = fields.get(self.state().app_field.min(fields.len().saturating_sub(1)))
        else {
            return false;
        };
        let value = c9s_core::safe_text::clean(&field.value).into_owned();
        let what = if field.label.is_empty() {
            "it".to_string()
        } else {
            field.label.to_lowercase()
        };
        let notice = match self.desktop.copy(&value) {
            Ok(()) => format!("Copied {what}: {value}"),
            Err(e) => format!("Could not copy ({e}): {value}"),
        };
        self.state_mut().notice = Some(notice);
        true
    }

    /// The pages `o` can open for an application: its registration, its
    /// enterprise application and, for a managed identity, its Azure
    /// resource; with the section each belongs to.
    fn app_pages(
        &self,
        row: &c9s_core::apps::AppRow,
    ) -> Vec<(c9s_core::app_view::Section, String, String)> {
        use c9s_core::app_view::Section;
        let tenant = self.state().portal_tenant();
        let mut pages = Vec::new();
        if row.app.is_some() {
            if let Some(url) =
                detail::entra_portal_url("application", &row.app_id, &row.app_id, tenant.as_deref())
            {
                pages.push((Section::Registration, "App registration".to_string(), url));
            }
        }
        if let Some(sp) = &row.sp {
            if let Some(url) =
                detail::entra_portal_url("servicePrincipal", &sp.id, &row.app_id, tenant.as_deref())
            {
                pages.push((
                    Section::EnterpriseApp,
                    "Enterprise application".to_string(),
                    url,
                ));
            }
        }
        if let Some(url) = c9s_core::app_view::managed_identity_url(row, self.state()) {
            let what = match row.managed_identity().and_then(|m| m.user_assigned) {
                Some(true) => "Azure resource (the user-assigned identity)",
                Some(false) => "Azure resource (what the identity belongs to)",
                None => "Azure resource",
            };
            pages.push((Section::Resource, what.to_string(), url));
        }
        pages
    }

    /// Keys while `s` asks which column to sort by: ↑/↓ or j/k choose,
    /// Enter or 1-9 sort (the same column again: the other way), 0 the
    /// view's own order, Esc cancels.
    fn handle_sort_key(&mut self, key: KeyEvent) {
        let view = self.state().view.clone();
        let columns = c9s_core::sort::columns(&view);
        let Some(selected) = self.state().sort_picker else {
            return;
        };
        // The last entry is "Default order".
        let entries = columns.len() + 1;
        let chosen = match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.state_mut().sort_picker = None;
                return;
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                self.state_mut().sort_picker = Some((selected + 1) % entries);
                return;
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                self.state_mut().sort_picker = Some((selected + entries - 1) % entries);
                return;
            }
            KeyCode::Enter | KeyCode::Char('s') => selected,
            KeyCode::Char('0') => columns.len(),
            KeyCode::Char(c @ '1'..='9') => {
                let i = c as usize - '1' as usize;
                if i >= columns.len() {
                    return;
                }
                i
            }
            _ => return,
        };
        // Keep the highlighted item highlighted after the rows move.
        let keep = self.state().selected_id();
        let state = self.state_mut();
        state.sort_picker = None;
        if chosen >= columns.len() {
            state.sorts.remove(&view);
            state.notice = Some("Default order".into());
        } else {
            let descending = matches!(
                state.sort_spec(&view),
                Some(s) if s.column == chosen && !s.descending
            );
            let spec = c9s_core::sort::SortSpec {
                column: chosen,
                descending,
            };
            state.notice = Some(format!("Sorted by {}", spec.label(&view)));
            state.sorts.insert(view, spec);
        }
        state.list_state.reset();
        if let Some(id) = keep {
            state.select_id(&id);
        }
    }

    /// Keys while `o` asks which page to open: ↑/↓ or j/k choose, 1-9 or
    /// Enter open, Esc cancels.
    fn handle_portal_choice_key(&mut self, key: KeyEvent) {
        let Some((pages, selected)) = self.state().portal_choice.clone() else {
            return;
        };
        let chosen = match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.state_mut().portal_choice = None;
                return;
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                self.state_mut().portal_choice =
                    Some((pages.clone(), (selected + 1) % pages.len()));
                return;
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                let up = (selected + pages.len() - 1) % pages.len();
                self.state_mut().portal_choice = Some((pages.clone(), up));
                return;
            }
            KeyCode::Enter | KeyCode::Char('o') => selected,
            KeyCode::Char(c @ '1'..='9') => {
                let i = c as usize - '1' as usize;
                if i >= pages.len() {
                    return;
                }
                i
            }
            _ => return,
        };
        self.state_mut().portal_choice = None;
        let (name, url) = pages[chosen].clone();
        self.open_url(&url, &format!("the Azure Portal ({name})"));
    }

    /// The detail view's current lines as plain text, for searching.
    fn detail_texts(&self) -> Vec<String> {
        if self.state().view == ViewMode::AppDetail {
            return c9s_core::app_view::app_lines(self.state())
                .iter()
                .map(|l| l.iter().map(|(t, _)| t.as_str()).collect())
                .collect();
        }
        crate::ui::detail_lines(self.state())
            .iter()
            .map(|l| l.to_string())
            .collect()
    }

    /// Go to the match of `pattern` after (or before) `from`, scroll to it
    /// and say where it is. Returns whether one was found.
    fn search_jump(
        &mut self,
        pattern: &str,
        from: (usize, usize),
        forward: bool,
        inclusive: bool,
    ) -> bool {
        let matches = search::find_matches(&self.detail_texts(), pattern);
        let state = self.state_mut();
        match search::next_match(&matches, from, forward, inclusive) {
            Some((i, wrapped)) => {
                let m = matches[i];
                state.detail_match = Some((m.line, m.start));
                state.detail_scroll_to = Some(m.line);
                let position = format!("/{pattern}  match {} of {}", i + 1, matches.len());
                state.notice = Some(match (wrapped, forward) {
                    (true, true) => format!("search hit BOTTOM, continuing at TOP · {position}"),
                    (true, false) => format!("search hit TOP, continuing at BOTTOM · {position}"),
                    _ => position,
                });
                true
            }
            None => {
                state.detail_match = None;
                state.notice = Some(format!("Pattern not found: {pattern}"));
                false
            }
        }
    }

    /// `n` / `N`: next or previous match of the committed search.
    fn search_step(&mut self, forward: bool) {
        let pattern = self.state().detail_search.clone();
        if pattern.is_empty() {
            self.state_mut().notice = Some("No previous search (type / to search)".into());
            return;
        }
        let from = self
            .state()
            .detail_match
            .unwrap_or((self.state().detail_scroll, 0));
        // Without a current match, the first match at the top line counts.
        let inclusive = self.state().detail_match.is_none();
        self.search_jump(&pattern, from, forward, inclusive);
    }

    /// Keys while typing a search after `/`: incremental, like Vim's
    /// incsearch. Enter keeps the search, Esc cancels it.
    fn handle_search_key(&mut self, key: KeyEvent) {
        let state = self.state_mut();
        let Some(input) = state.detail_search_input.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Enter => {
                let pattern = state.detail_search_input.take().unwrap_or_default();
                if pattern.is_empty() {
                    // Like Vim: an empty pattern repeats the last search.
                    self.search_step(true);
                } else {
                    state.detail_search = pattern;
                }
                return;
            }
            KeyCode::Esc => {
                let (scroll, current) = state.detail_search_origin;
                state.detail_search_input = None;
                state.detail_scroll = scroll;
                state.detail_match = current;
                state.notice = None;
                return;
            }
            KeyCode::Backspace => {
                if input.pop().is_none() {
                    // Backspace on an empty search leaves search mode.
                    state.detail_search_input = None;
                    return;
                }
            }
            KeyCode::Char(c) => input.push(c),
            _ => return,
        }
        // Incremental: from where the search started, as the pattern grows.
        let pattern = input.clone();
        let (origin_scroll, _) = state.detail_search_origin;
        if pattern.is_empty() {
            state.detail_match = None;
            state.notice = None;
            return;
        }
        self.search_jump(&pattern, (origin_scroll, 0), true, true);
    }

    /// Copy what the current view shows (`c`): every row passing the
    /// filters, as tab-separated text, or the detail view's current tab.
    fn copy_view(&mut self) {
        let Some((what, text)) = c9s_core::export::view_text(self.state()) else {
            self.state_mut().notice = Some("Nothing to copy here".into());
            return;
        };
        // Cloud text can carry control characters that would act when
        // pasted into a shell or terminal.
        let text = c9s_core::safe_text::clean_lines(&text);
        let size = if text.len() >= 1024 {
            format!("{} KB", text.len() / 1024)
        } else {
            format!("{} bytes", text.len())
        };
        let notice = match self.desktop.copy(&text) {
            Ok(()) => format!("Copied {what} ({size})"),
            Err(e) => format!("Could not copy ({e})"),
        };
        self.state_mut().notice = Some(notice);
    }

    /// Open the selected item in the Azure Portal (`o`). The status bar
    /// shows the link either way, e.g. for SSH sessions without a browser.
    fn open_in_portal(&mut self) {
        let Some(id) = self.state().selected_arm_id() else {
            return;
        };
        match self.state().provider {
            Provider::Azure => {}
            Provider::Gcp => {
                // Buckets' names carry no project: take the row's.
                let state = self.state();
                let project = match state.view {
                    ViewMode::ResourceList => {
                        state.selected_resource().map(|r| r.subscription_id.clone())
                    }
                    ViewMode::ResourceDetail => state
                        .selected_resource_detail
                        .as_ref()
                        .map(|r| r.subscription_id.clone()),
                    _ => None,
                };
                let url = detail::gcp_console_url(&id, project.as_deref());
                self.open_url(&url, "the Google Cloud console");
                return;
            }
            Provider::Aws => {
                let url = detail::aws_console_url(&id);
                self.open_url(&url, "the AWS console");
                return;
            }
        }
        let tenant = self.state().portal_tenant();
        // Directory objects open on their Entra page, not as a resource.
        let app = match self.state().view {
            ViewMode::AppList => self.state().selected_app(),
            ViewMode::AppDetail => self.state().app_open.clone(),
            _ => None,
        };
        // An application: its registration, its enterprise application
        // or, for a managed identity, its Azure resource. With more than
        // one, ask which (the selected field's page first).
        if let Some(row) = &app {
            let pages = self.app_pages(row);
            match pages.len() {
                0 => {}
                1 => {
                    let (_, name, url) = &pages[0];
                    self.open_url(url, &format!("the Azure Portal ({name})"));
                    return;
                }
                _ => {
                    let preferred = if self.state().view == ViewMode::AppDetail
                        && self.state().app_tab == c9s_core::state::AppTab::Overview
                    {
                        let fields = c9s_core::app_view::overview_fields(self.state());
                        fields
                            .get(self.state().app_field)
                            .and_then(|f| pages.iter().position(|(s, _, _)| *s == f.section))
                            .unwrap_or(0)
                    } else {
                        0
                    };
                    let choices = pages.into_iter().map(|(_, n, u)| (n, u)).collect();
                    self.state_mut().portal_choice = Some((choices, preferred));
                    return;
                }
            }
        }
        let directory = match self.state().view {
            ViewMode::AppList | ViewMode::AppDetail => app
                .map(|a| match a.sp {
                    // The enterprise app (by object ID), else the registration.
                    Some(sp) => ("servicePrincipal".to_string(), a.app_id.clone(), sp.id),
                    None => ("application".to_string(), a.app_id.clone(), a.app_id),
                })
                .map(|(kind, app_id, _)| (kind, app_id)),
            ViewMode::GroupList => Some(("group".to_string(), String::new())),
            ViewMode::GroupMembers => self.state().selected_member().map(|m| (m.kind, m.detail)),
            ViewMode::MemberOf => Some(("group".to_string(), String::new())),
            _ => None,
        };
        // Applications: the enterprise app's page needs its object ID.
        let id = match self.state().view {
            ViewMode::AppList | ViewMode::AppDetail => match self.state().view {
                ViewMode::AppList => self.state().selected_app(),
                _ => self.state().app_open.clone(),
            }
            .and_then(|a| a.sp.map(|s| s.id))
            .unwrap_or(id),
            _ => id,
        };
        let url = match directory {
            Some((kind, detail)) => {
                match detail::entra_portal_url(&kind, &id, &detail, tenant.as_deref()) {
                    Some(url) => url,
                    None => {
                        self.state_mut().notice = Some(format!("No Portal page for this {kind}"));
                        return;
                    }
                }
            }
            None => detail::portal_url(&id, tenant.as_deref()),
        };
        self.open_url(&url, "the Azure Portal");
    }

    /// Open `url` in a browser; without one, copy it to the clipboard.
    fn open_url(&mut self, url: &str, place: &str) {
        let notice = match self.desktop.open(url) {
            Ok(()) => format!("Opened in {place}"),
            // The link is too long for the status bar: put it on the
            // clipboard instead, so it can be pasted into a browser.
            Err(e) => match self.desktop.copy(url) {
                Ok(()) => format!("No browser could be started; link copied ({e})"),
                Err(_) => format!("No browser could be started ({e}): {url}"),
            },
        };
        self.state_mut().notice = Some(notice);
    }

    /// `d` in the policy view: the selected assignment's own details.
    fn open_assignment_detail(&mut self) {
        let Some(row) = self.state().selected_policy_row() else {
            return;
        };
        self.push_current_context(None);
        let a = row.assignment;
        let subscription_id = a
            .scope
            .to_ascii_lowercase()
            .strip_prefix("/subscriptions/")
            .map(|s| s.split('/').next().unwrap_or(s).to_string())
            .unwrap_or_default();
        self.core.open_resource_detail(c9s_core::models::Resource {
            id: a.id,
            name: a.display_name,
            resource_type: "Microsoft.Authorization/policyAssignments".into(),
            location: String::new(),
            resource_group: String::new(),
            subscription_id,
            tags: Default::default(),
        });
    }

    /// Handle selection (Enter key)
    async fn handle_select(&mut self) -> Result<bool> {
        match self.state().view {
            ViewMode::ManagementGroupTree => self.handle_tree_select(),
            ViewMode::SubscriptionList => self.handle_subscription_select().await,
            ViewMode::ResourceTypeSummary => {
                self.handle_type_select();
                Ok(true)
            }
            ViewMode::TagSummary => {
                self.handle_tag_select();
                Ok(true)
            }
            ViewMode::PolicyAssignments => {
                if let Some(row) = self.state().selected_policy_row() {
                    self.push_current_context(None);
                    self.core.open_non_compliant(row.assignment);
                }
                Ok(true)
            }
            ViewMode::RoleAssignments => {
                // The role the assignment grants.
                if let Some(row) = self.state().selected_role_assignment() {
                    let a = &row.assignment;
                    let resource =
                        self.role_definition_resource(&a.role_guid(), &a.role_definition_id);
                    self.push_current_context(None);
                    self.core.open_resource_detail(resource);
                }
                Ok(true)
            }
            ViewMode::AppList => {
                if let Some(row) = self.state().selected_app() {
                    self.push_current_context(None);
                    self.core.open_app(row);
                }
                Ok(true)
            }
            ViewMode::AppDetail => Ok(true),
            ViewMode::GroupList => {
                if let Some(group) = self.state().selected_group() {
                    self.push_current_context(None);
                    self.state_mut().filter.clear();
                    self.core.open_group_members(&group.id, false);
                }
                Ok(true)
            }
            ViewMode::GroupMembers | ViewMode::MemberOf => {
                // A group: its members. Anyone else: their groups.
                let selected = match self.state().view {
                    ViewMode::GroupMembers => self.state().selected_member(),
                    _ => self.state().selected_member_of(),
                };
                if let Some(p) = selected {
                    self.push_current_context(None);
                    self.state_mut().filter.clear();
                    if p.kind == "group" {
                        self.core.open_group_members(&p.id, false);
                    } else {
                        self.core.open_member_of(&p.id);
                    }
                }
                Ok(true)
            }
            ViewMode::RoleDefinitions => {
                // Everyone who has the role, across the tenant.
                if let Some(row) = self.state().selected_role_definition() {
                    self.push_current_context(None);
                    let state = self.state_mut();
                    state.filter.clear();
                    self.core
                        .open_role_assignments(None, Some(row.definition.guid()));
                }
                Ok(true)
            }
            ViewMode::ExemptionList => {
                if let Some(row) = self.state().selected_exemption() {
                    self.push_current_context(None);
                    self.core
                        .open_resource_detail(exemption_resource(&row.exemption));
                }
                Ok(true)
            }
            ViewMode::ComplianceRanking => {
                if let Some(row) = self.state().selected_compliance_row() {
                    self.push_current_context(None);
                    if let Some(sub) = self.find_subscription(&row.subscription_id).cloned() {
                        self.state_mut().selected_subscription = Some(sub);
                    }
                    self.core
                        .open_policy(PolicyScope::Subscription(row.subscription_id));
                }
                Ok(true)
            }
            ViewMode::PolicyNonCompliant => {
                if let Some(row) = self.state().selected_non_compliant() {
                    self.push_current_context(None);
                    let s = row.state;
                    self.core.open_resource_detail(c9s_core::models::Resource {
                        name: s.resource_name().to_string(),
                        id: s.resource_id,
                        resource_type: s.resource_type,
                        location: s.location,
                        resource_group: s.resource_group,
                        subscription_id: s.subscription_id,
                        tags: Default::default(),
                    });
                }
                Ok(true)
            }
            ViewMode::TenantList => {
                if let Some(row) = self.state().selected_tenant_row() {
                    match row.provider {
                        Provider::Azure => self.core.switch_tenant(row.id),
                        Provider::Gcp => self.core.switch_to_gcp(row.id),
                        Provider::Aws => {
                            if let Some(profile) = row.id {
                                self.core.switch_to_aws(profile)
                            }
                        }
                    }
                }
                Ok(true)
            }
            ViewMode::ResourceGroupList => self.handle_resource_group_select().await,
            ViewMode::ResourceList => self.handle_resource_select().await,
            ViewMode::ResourceDetail => {
                self.open_detail_row();
                Ok(true)
            }
            ViewMode::Logs => Ok(true),
        }
    }

    /// Enter on the tree: a subscription opens its resource groups; a group
    /// opens the resource groups of every enabled subscription below it.
    /// Under GCP, a project or folder opens its resources.
    fn handle_tree_select(&mut self) -> Result<bool> {
        let Some(row) = self.state().selected_tree_row() else {
            return Ok(true);
        };
        // Clouds without containers (GCP) open straight onto resources.
        let containers = self.state().provider.supports(Feature::Containers);
        if row.kind == TreeRowKind::Subscription {
            let sub = self
                .state()
                .subscriptions
                .iter()
                .find(|s| s.id.eq_ignore_ascii_case(&row.key))
                .cloned()
                .unwrap_or_else(|| c9s_core::models::Subscription {
                    id: row.key.clone(),
                    name: row.label.clone(),
                    state: "Enabled".into(),
                    tenant_id: String::new(),
                });
            self.push_current_context(None);
            let id = sub.id.clone();
            let state = self.state_mut();
            state.selected_subscription = Some(sub);
            state.scope_group = None;
            if containers {
                self.show_resource_groups(vec![id], false);
            } else {
                self.show_resources(vec![id], None, false);
            }
            return Ok(true);
        }

        let ids = self.enabled_subscriptions_under(Some(&row.key));
        if ids.is_empty() {
            let accounts = self.state().provider.terms().accounts.to_lowercase();
            self.state_mut().notice = Some(format!("No enabled {accounts} under {}", row.label));
            return Ok(true);
        }
        self.push_current_context(None);
        let state = self.state_mut();
        state.scope_group = Some(row.key.clone());
        state.selected_subscription = None;
        if containers {
            self.show_resource_groups(ids, true);
        } else {
            self.show_resources(ids, None, true);
        }
        Ok(true)
    }

    /// Open (`open = true`) or close the selected group in the tree. Closing
    /// a row that is already closed (or a subscription) moves to its parent.
    fn tree_expand(&mut self, open: Option<bool>) {
        let rows = self.state().tree_rows();
        let selected = self.state().list_state.selected;
        let Some(row) = rows.get(selected) else {
            return;
        };
        let open = open.unwrap_or(!row.expanded);
        if row.is_group() && row.has_children && row.expanded != open {
            self.state_mut().set_tree_expanded(&row.key, open);
        } else if !open && row.depth > 0 {
            if let Some(parent) = rows[..selected].iter().rposition(|r| r.depth < row.depth) {
                self.state_mut().list_state.selected = parent;
            }
        }
    }

    /// Enter on the type summary: the resource list, narrowed to that type
    /// (on top of any other filters). The rows are already loaded.
    fn handle_type_select(&mut self) {
        let Some(resource_type) = self.state().selected_type() else {
            return;
        };
        self.push_current_context(None);
        let state = self.state_mut();
        state.resource_query.types = vec![resource_type];
        state.filter.clear();
        state.view = ViewMode::ResourceList;
        state.list_state.reset();
    }

    /// Enter on the tags view: the resources with that tag (on top of any
    /// other filters). The rows are already loaded.
    fn handle_tag_select(&mut self) {
        let Some(tag) = self.state().selected_tag() else {
            return;
        };
        self.push_current_context(None);
        let state = self.state_mut();
        state.resource_query.tags = vec![(tag.key, Some(tag.value))];
        state.filter.clear();
        state.view = ViewMode::ResourceList;
        state.list_state.reset();
    }

    /// Handle subscription selection
    async fn handle_subscription_select(&mut self) -> Result<bool> {
        if let Some(subscription) = self.state().selected_subscription().cloned() {
            info!("Selected subscription: {}", subscription.name);
            self.push_current_context(None);
            let id = subscription.id.clone();
            self.state_mut().selected_subscription = Some(subscription);
            // Clouds without a level between accounts and resources (GCP)
            // open straight onto the resources.
            if self.state().provider.supports(Feature::Containers) {
                self.show_resource_groups(vec![id], false);
            } else {
                self.show_resources(vec![id], None, false);
            }
        }
        Ok(true)
    }

    /// Handle resource group selection: show only that group's resources
    async fn handle_resource_group_select(&mut self) -> Result<bool> {
        if let Some(resource_group) = self.state().selected_resource_group().cloned() {
            info!("Selected resource group: {}", resource_group.name);
            self.push_current_context(None);
            // Use the group's own subscription: in "all subscriptions" mode the
            // selected subscription is not the group's.
            if let Some(sub) = self
                .state()
                .subscriptions
                .iter()
                .find(|s| s.id == resource_group.subscription_id)
                .cloned()
            {
                self.state_mut().selected_subscription = Some(sub);
            }
            self.show_resources(
                vec![resource_group.subscription_id.clone()],
                Some(resource_group.name.clone()),
                false,
            );
        }
        Ok(true)
    }

    /// Remember the current view so Esc/Backspace can return to it
    fn push_current_context(&mut self, resource_group: Option<String>) {
        let state = self.state_mut();
        let mut metadata = std::collections::HashMap::new();
        if state.cross_subscription_mode {
            metadata.insert(CROSS_SUBSCRIPTION.to_string(), String::new());
        }
        if let Some(group) = &state.scope_group {
            metadata.insert(SCOPE_GROUP.to_string(), group.clone());
        }
        if !state.resource_query.is_empty() {
            metadata.insert(QUERY.to_string(), state.resource_query.to_string());
        }
        if let Some(id) = state.selected_id() {
            metadata.insert(SELECTED.to_string(), id);
        }
        if state.view == ViewMode::GroupMembers {
            if let Some(group) = &state.members_group {
                metadata.insert(MEMBERS_GROUP.to_string(), group.clone());
            }
            if state.members_transitive {
                metadata.insert(MEMBERS_TRANSITIVE.to_string(), "1".into());
            }
        }
        if state.view == ViewMode::MemberOf {
            if let Some(principal) = &state.member_of {
                metadata.insert(MEMBER_OF.to_string(), principal.clone());
            }
        }
        if state.view == ViewMode::RoleAssignments {
            if let Some(scope) = &state.ra_scope {
                let encoded = match scope {
                    PolicyScope::Group(g) => format!("g:{g}"),
                    PolicyScope::Subscription(s) => format!("s:{s}"),
                };
                metadata.insert(RA_SCOPE.to_string(), encoded);
            }
            if let Some(role) = &state.ra_role {
                metadata.insert(RA_ROLE.to_string(), role.clone());
            }
            if let Some(principal) = &state.ra_principal {
                metadata.insert(RA_PRINCIPAL.to_string(), principal.clone());
            }
        }
        let ctx = NavigationContext {
            view: state.view.clone(),
            subscription_id: state.selected_subscription.as_ref().map(|s| s.id.clone()),
            resource_group,
            resource_id: None,
            metadata,
        };
        state.push_navigation(ctx);
    }

    /// Handle resource selection
    async fn handle_resource_select(&mut self) -> Result<bool> {
        if let Some(resource) = self.state().selected_resource().cloned() {
            info!("Selected resource: {}", resource.name);
            self.push_current_context(Some(resource.resource_group.clone()));
            self.core.open_resource_detail(resource);
        }
        Ok(true)
    }

    /// Handle view-specific keys
    async fn handle_view_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Char('y') => {
                self.copy_selected_id();
                return Ok(true);
            }
            KeyCode::Char('o') => {
                self.open_in_portal();
                return Ok(true);
            }
            KeyCode::Char('c') => {
                // An application's Overview: the selected field.
                if !self.copy_app_field() {
                    self.copy_view();
                }
                return Ok(true);
            }
            KeyCode::Char('C') => {
                self.copy_view();
                return Ok(true);
            }
            KeyCode::Char('s') if !c9s_core::sort::columns(&self.state().view).is_empty() => {
                // Start on the column the view is sorted by, else the first.
                let state = self.state_mut();
                let at = state.sort_spec(&state.view).map_or(0, |s| s.column);
                state.sort_picker = Some(at);
                return Ok(true);
            }
            _ => {}
        }
        if self.state().view == ViewMode::ResourceDetail {
            let tab = self.state().detail_tab;
            let next = match key.code {
                KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => Some(tab.cycle(1)),
                KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => Some(tab.cycle(-1)),
                KeyCode::Char(c @ '1'..='4') => Some(DetailTab::ALL[c as usize - '1' as usize]),
                _ => None,
            };
            if let Some(next) = next.filter(|n| *n != tab) {
                let state = self.state_mut();
                state.detail_tab = next;
                state.detail_scroll = 0;
                state.detail_cursor = 0;
                state.detail_match = None;
                if next == DetailTab::Policy {
                    self.core.load_resource_policy(false);
                }
            }
            match key.code {
                KeyCode::Char('n') => self.search_step(true),
                KeyCode::Char('N') => self.search_step(false),
                _ => {}
            }
            return Ok(true);
        }
        // Applications: `m` Microsoft's apps on/off; tabs; `p` Azure access.
        if self.state().view == ViewMode::AppList && key.code == KeyCode::Char('m') {
            let state = self.state_mut();
            state.show_microsoft_apps = !state.show_microsoft_apps;
            state.list_state.reset();
            return Ok(true);
        }
        if self.state().view == ViewMode::AppDetail {
            let tab = self.state().app_tab;
            let next = match key.code {
                KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => Some(tab.cycle(1)),
                KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => Some(tab.cycle(-1)),
                KeyCode::Char(c @ '1'..='7') => Some(AppTab::ALL[c as usize - '1' as usize]),
                _ => None,
            };
            if let Some(next) = next.filter(|n| *n != tab) {
                let state = self.state_mut();
                state.app_tab = next;
                state.detail_scroll = 0;
                state.detail_match = None;
                return Ok(true);
            }
            match key.code {
                KeyCode::Char('n') => {
                    self.search_step(true);
                    return Ok(true);
                }
                KeyCode::Char('N') => {
                    self.search_step(false);
                    return Ok(true);
                }
                _ => {}
            }
        }
        if matches!(self.state().view, ViewMode::AppList | ViewMode::AppDetail)
            && key.code == KeyCode::Char('p')
        {
            let row = match self.state().view {
                ViewMode::AppList => self.state().selected_app(),
                _ => self.state().app_open.clone(),
            };
            match row.and_then(|r| r.sp) {
                Some(sp) => {
                    self.push_current_context(None);
                    self.state_mut().filter.clear();
                    self.core.open_access(&sp.id);
                }
                None => {
                    self.state_mut().notice = Some(
                        "No enterprise application in this tenant: it can't hold Azure roles"
                            .into(),
                    )
                }
            }
            return Ok(true);
        }
        // Directory views: `t` nested members on/off, `p` Azure access.
        let directory_principal = match self.state().view {
            ViewMode::GroupList => self.state().selected_group().map(|g| g.id),
            ViewMode::GroupMembers => self.state().selected_member().map(|m| m.id),
            ViewMode::MemberOf => self.state().selected_member_of().map(|m| m.id),
            _ => None,
        };
        if self.state().view == ViewMode::GroupMembers && key.code == KeyCode::Char('t') {
            let state = self.state();
            if let Some(group) = state.members_group.clone() {
                let transitive = !state.members_transitive;
                self.core.open_group_members(&group, transitive);
            }
            return Ok(true);
        }
        if let (Some(id), KeyCode::Char('p')) = (directory_principal, key.code) {
            self.push_current_context(None);
            self.state_mut().filter.clear();
            self.core.open_access(&id);
            return Ok(true);
        }
        if self.state().view == ViewMode::RoleAssignments && key.code == KeyCode::Char('p') {
            // Everything the row's principal (or group) has, everywhere.
            if let Some(row) = self.state().selected_role_assignment() {
                self.push_current_context(None);
                self.state_mut().filter.clear();
                self.core.open_access(&row.assignment.principal_id);
            }
            return Ok(true);
        }
        if matches!(
            self.state().view,
            ViewMode::RoleAssignments | ViewMode::RoleDefinitions
        ) && key.code == KeyCode::Char('d')
        {
            self.open_rbac_detail();
            return Ok(true);
        }
        if self.state().view == ViewMode::PolicyAssignments && key.code == KeyCode::Char('d') {
            self.open_assignment_detail();
            return Ok(true);
        }
        if self.state().view == ViewMode::ManagementGroupTree {
            match key.code {
                KeyCode::Char(' ') => self.tree_expand(None),
                KeyCode::Right | KeyCode::Char('l') => self.tree_expand(Some(true)),
                KeyCode::Left | KeyCode::Char('h') => self.tree_expand(Some(false)),
                _ => {}
            }
        }
        Ok(true)
    }
}

/// An exemption as a detail-view item (found by ID across the tenant).
fn exemption_resource(e: &c9s_core::policy::PolicyExemption) -> c9s_core::models::Resource {
    let scope = e.scope().to_ascii_lowercase();
    let subscription_id = scope
        .strip_prefix("/subscriptions/")
        .map(|s| s.split('/').next().unwrap_or(s).to_string())
        .unwrap_or_default();
    c9s_core::models::Resource {
        id: e.id.clone(),
        name: e.display_name.clone(),
        resource_type: "Microsoft.Authorization/policyExemptions".into(),
        location: String::new(),
        resource_group: String::new(),
        subscription_id,
        tags: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_tui_app_creation() {
        // This test requires async runtime, so we'll keep it simple
        // Just verify the module compiles
    }

    #[test]
    fn test_navigation_empty_list() {
        // Test that navigation on empty lists doesn't panic
        // This would require setting up a full app, so we'll skip for now
    }
}
