use super::theme::theme;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

/// The help text. Kept in one place so it can be tested for coverage.
pub fn help_lines() -> Vec<Line<'static>> {
    let heading = |text: &'static str| {
        Line::from(Span::styled(
            text,
            Style::default()
                .fg(theme().highlight)
                .add_modifier(Modifier::BOLD),
        ))
    };
    let dim =
        |text: &'static str| Line::from(Span::styled(text, Style::default().fg(theme().muted)));
    let l = Line::from;
    vec![
        Line::from(Span::styled(
            "c9s help  ·  j/k or ↑/↓ scroll  ·  Esc, q or ? close",
            Style::default()
                .fg(theme().accent)
                .add_modifier(Modifier::BOLD),
        )),
        l(""),
        heading("Clouds (:ctx)"),
        l("  One context at a time; the status bar names it (azure · Contoso, gcp · work,"),
        l("  aws · prod-admin). :ctx lists every context, Enter switches, :ctx all is Azure."),
        l("  Azure    tenants (az login): everything below"),
        l("  GCP      gcloud configurations: folders (:mg), projects, resources from Cloud"),
        l("           Asset Inventory (location as the group), documents, o for the console"),
        l("  AWS      CLI profiles: the account, or the organization's accounts and units"),
        l("           (:mg), resources from Resource Explorer (region as the group), o"),
        l("  Policy, role assignments, groups and apps are Azure only and say so."),
        dim("  Setup and permissions per cloud: docs/CLOUDS.md"),
        l(""),
        heading("Go to anything (Ctrl+P)"),
        l("  Ctrl+P    fuzzy search commands, subscriptions, management groups, and the"),
        l("            loaded resource groups and resources: type a few letters (kvprod"),
        l("            finds kv-prod-weu), ↑/↓ choose, Enter goes there, Esc closes."),
        l("            Only loaded data is searched: after :res all, every resource."),
        l(""),
        heading("Commands (type : then the command, Enter runs it)"),
        l("  Suggestions appear as you type: commands, scopes, filter keys, and values"),
        l("  from what's loaded (type=, loc=, tag=key=…). Tab completes, ↑/↓ choose."),
        l("  :policy [all|<sub>]                Policy assignments that apply there, with compliance"),
        l("  :compliance [all|<mg>]             Subscriptions ranked by non-compliant resources"),
        l("  :exemption [all|<mg>|<sub>]        Policy exemptions, soonest to expire first"),
        l("  :ra [all|<mg>|<sub>]               Role assignments: inherited, here, below"),
        l("  :role                              Role definitions, built-in and custom"),
        l("  :access <principal>                Everything one user, group or app can do"),
        l("  :group [name]                      Directory groups and their members"),
        l("  :app [name]                        App registrations and enterprise applications"),
        l("  :cost [all|<mg>|<sub>]             What a subscription or management group cost ($)"),
        l("  :theme [name]                      Switch colours: default, light, mono, high-contrast,"),
        l("                                     dracula, nord, gruvbox, catppuccin, tokyo-night,"),
        l("                                     solarized-dark, solarized-light (config: theme = …)"),
        l("  :ctx  [all|<context>]              Contexts: Azure tenants, gcloud configurations and"),
        l("                                     AWS profiles; list them, or switch by name"),
        l("  :mg                                The tree: management groups (the start view),"),
        l("                                     GCP folders, AWS organizational units"),
        l("  :sub                               Subscriptions"),
        l("  :rg   [scope] [loc=…] [tag=…]      Resource groups"),
        l("  :res  [scope] [type=…] [loc=…] [tag=…]   Resources"),
        l("  :type [scope] [type=…] [loc=…] [tag=…]   How many resources of each type"),
        l("  :tag  [scope] [type=…] [loc=…] [tag=…]   Tags: key, value, how many resources"),
        l(""),
        l("  scope   (nothing)   where you are: the highlighted or current subscription"),
        l("                      or management group; all subscriptions if neither"),
        l("          all         every subscription"),
        l("          <name|ID>   one subscription, e.g. :rg prod-connectivity"),
        l(""),
        heading("Filters (after :rg, :res or :type; space-separated)"),
        l("  type=vm          a short alias (vm, vnet, nsg, aks, keyvault or key-vault…), the last"),
        l("                   part (virtualMachines), the full type, or a provider (Network,"),
        l("                   Microsoft.Network)"),
        l("  loc=westeurope   location"),
        l("  tag=env=prod     tag with that value (tag=env:prod also works)"),
        l("  tag=env          tag with any value"),
        l("  a,b              either value: type=vm,disk   loc=westeurope,northeurope"),
        l("  Several filters must all match. Case doesn't matter. :rg has no type=."),
        l("  Active filters show in the list title; a new command without them clears them."),
        l(""),
        heading("Examples"),
        l("  :res all type=vm loc=westeurope     VMs in West Europe, every subscription"),
        l("  :res type=vnet                      VNets where you are"),
        l("  :res all type=keyvault tag=environment=prod"),
        l("  :res all tag=env=prod,env=production   either value"),
        l("  :rg all loc=northeurope             Resource groups in North Europe"),
        l("  :rg tag=owner                       Resource groups with an owner tag"),
        l("  :type all                           What exists, most common type first"),
        l("  :type all loc=westeurope            ...only in West Europe; Enter lists a type"),
        l("  :tag all                            Every tag in use; Enter lists its resources"),
        l("  :tag all type=keyvault              Tags on key vaults"),
        l(""),
        heading("Text filter"),
        l("  /text     in a list: narrow it by name, type (or alias) or location"),
        l("            in the resource detail: search, see Resource detail below"),
        l("            Enter keeps it, Esc clears it; combines with the filters above"),
        l(""),
        heading("Keys"),
        l("  j/k ↑/↓   move          g/G   first/last        Ctrl+d/u, PgDn/PgUp   page"),
        l("  Enter     open          Esc   back / clear filter / close popup"),
        l("  r         refresh       ?     this help         q, Ctrl+c   quit"),
        l("  s         sort the list by a column (again: the other way; 0: default order)"),
        l("  y         copy the selected item's ID (Azure ID, GCP resource name, AWS ARN)"),
        l("  c         copy the current view (see below)"),
        l("  o         open the selected item in the Azure Portal (GCP, AWS: their console)"),
        l("            (no browser, e.g. WSL without wslview: the link is copied instead)"),
        l(""),
        heading("Copy the view (c)"),
        l("  Lists      every row that passes the filters, tab-separated with a header"),
        l("             (pastes into a spreadsheet), not only the rows on screen"),
        l("  Detail     the current tab: Overview, Tags, Policy or JSON"),
        l("  Tree, :type, :tag  their rows"),
        dim("  Copying goes through the terminal (OSC 52); in tmux enable set-clipboard."),
        l(""),
        heading("Management group tree"),
        l("  Space     open/close a group     l/→ open     h/← close (on a closed row: parent)"),
        l("  Enter     resource groups of the group (all its subscriptions) or subscription"),
        l(""),
        heading("Resource detail"),
        l("  Tab/l, Shift+Tab/h, or 1-4   Overview, Tags, Policy, JSON   j/k, g/G   scroll"),
        l("  Policy    every policy that evaluates the resource, non-compliant first:"),
        l("            its name, initiative or direct, assignment, where it is assigned"),
        l("  Tags and Policy tabs: j/k select a row, Enter opens it:"),
        l("            a tag: every resource in the tenant with it (Esc comes back)"),
        l("            a policy: its definition (effect, category, rule in JSON)"),
        l("  /text     search the current tab, like Vim: jumps as you type, all matches"),
        l("            highlighted; Enter keeps it, Esc cancels. n / N next / previous"),
        l("            match (wraps around). Lowercase ignores case; any capital: exact case."),
        l("            Esc again clears the highlights."),
        l("  VNets list their subnets: prefixes, delegations, NSG, route table, NAT gateway"),
        l(""),
        heading("Policy (:policy)"),
        l("  Assignments that apply at the scope: assigned there or inherited from"),
        l("  management groups above it (\"from Platform\"). Each shows non-compliant out of"),
        l("  evaluated resources, worst first; I = initiative, P = single policy."),
        l("  :policy uses where you are (highlighted subscription or management group);"),
        l("  :policy all the whole tenant."),
        l("  First line: the scope's compliance: % of evaluated resources compliant with"),
        l("            every policy, how many are not, subscriptions and assignments affected"),
        l("  Enter     the assignment's non-compliant resources in that scope, with the"),
        l("            failing policies; Enter on one opens its detail (Policy tab: 3)"),
        l("  d         the assignment itself (Overview, JSON)"),
        l(""),
        heading("Compliance (:compliance, and the tree)"),
        l("  The tree shows each management group's and subscription's compliance:"),
        l("            % of evaluated resources compliant with every policy, and how many"),
        l("            are not (groups add up their subscriptions). Loads in the background."),
        l("  :compliance [all|<mg>]   subscriptions ranked by non-compliant resources,"),
        l("            worst first; Enter opens :policy for the subscription"),
        l(""),
        heading("Exemptions (:exemption)"),
        l("  :exemption [all|<mg>|<sub>]   exemptions at or below the scope: category,"),
        l("            expiry (red: expired, yellow: within 30 days), assignment, scope,"),
        l("            reason; soonest to expire first. Enter opens one."),
        l("  In a resource's Policy tab, Exempt rows name their exemption; Enter opens it."),
        l(""),
        heading("Access (:ra, :role)"),
        l("  :ra [all|<mg>|<sub>]   role assignments that matter where you are: inherited"),
        l("            from management groups above, at the scope, and below it (resource"),
        l("            groups, resources). C custom role, B built-in. Enter opens the role;"),
        l("            d the assignment. / filters by role, principal, type or scope."),
        l("            PIM eligible assignments show \"eligible\" (until when); /eligible lists"),
        l("            only them, /active the rest. They count in p and :access too."),
        l("  p         (in :ra) everything the row's principal can do, across the tenant:"),
        l("            its own assignments and its groups' (nested too), \"via <group>\""),
        l("  :access <name|sign-in|app id|object id>   the same for any principal"),
        l("  :role      role definitions, custom first, with how many assignments each has;"),
        l("            Enter lists the role's assignments, d opens its permissions"),
        l(""),
        heading("Cost (:cost, $)"),
        l("  :cost [all|<mg>|<sub>]   cost where you are: the highlighted subscription,"),
        l("            resource group or management group. $ in the subscriptions, resource"),
        l("            groups or tree shows the highlighted one's cost (Esc comes back)."),
        l("  Chart     total, average, and a bar per day or month; ←/→ (h/l) pick a bar"),
        l("  Table     cost per resource group, resource type, service or (management"),
        l("            group) subscription, most expensive first; Enter opens a resource"),
        l("            group's or subscription's own cost; s sorts, / filters, c copies"),
        l("  p         period: last 30 days, last 3 months, last 6 months"),
        l("  m         daily or monthly bars          b   group by (cycles)"),
        l("  a         amortized or actual cost (reservations spread over their term)"),
        l("  u         USD or the billing currency    o   cost analysis in the Portal"),
        dim("  Needs Cost Management Reader (or Reader) on the scope. Azure only."),
        l(""),
        heading("Groups (:group)"),
        l("  :group [name]   directory groups: kind (security, Microsoft 365, distribution),"),
        l("            dynamic or assigned, R role-assignable. Enter lists the members."),
        l("  Members   t switches between direct and nested members. Enter on a group opens"),
        l("            its members; on a user or app, the groups it belongs to (nested too)."),
        l("  p         the group's or member's Azure access; o its Entra page in the Portal"),
        l(""),
        heading("Applications (:app)"),
        l("  :app [name]  one row per app: its registration and enterprise application,"),
        l("            joined by app ID. registered, external, user-assigned MI, system-assigned"),
        l("            MI (/managed finds both); m shows Microsoft's own apps. Soonest credential"),
        l("            expiry: red expired, yellow 30 days"),
        l("  Enter     tabs (1-7): Overview, Permissions (requested vs granted, consent),"),
        l("            Exposed API, Owners, Credentials, Assigned users and groups, JSON"),
        l("  Overview  ↑↓/jk select a field; y or c copies it (app ID first), C the whole tab"),
        l("  /text     search any tab (JSON too), n / N next / previous; Esc clears it"),
        l("  o         open in the Portal: with several pages, choose the app registration,"),
        l("            the enterprise application or (managed identity) its Azure resource;"),
        l("            the selected field's page is offered first"),
        l("  p         the enterprise app's Azure access"),
        l("  Managed identities: Overview shows the identity's Azure resource (user-assigned)"),
        l("            or the resource it belongs to (system-assigned)"),
        l(""),
        heading("Tenants (:ctx)"),
        l("  :ctx        list the tenants you can use (● current); Enter switches"),
        l("  :ctx Contoso   switch by name, domain or ID;  :ctx all   back to the default"),
        l("  After a switch, subscriptions, `all`, the tree, the palette and suggestions"),
        l("  cover that tenant only. Tokens come from `az account get-access-token"),
        l("  --tenant`; if it asks, run `az login --tenant <id>` once."),
        l(""),
        heading("Config file (optional)"),
        l("  c9s --init-config writes one with the defaults; c9s --help shows where."),
        l("  start = \"mg\" | \"sub\"        first view"),
        l("  default_subscription = \"…\"   scope for :rg/:res/:type/:tag without context"),
        l("  refresh_interval = 120       auto-refresh every N seconds (min 30; 0 off)"),
        l("  tenant = \"<tenant-id>\"       start in this tenant (like :ctx)"),
        l("  theme = \"default\"            \"light\" for light terminals, \"mono\" for no colours"),
        l("  read_only = true             status bar shows RO; actions (M4) disabled"),
        l(""),
        heading("Status bar"),
        l("  ● Azure · 2m ago    fresh from Azure"),
        l("  ◐ Cached · 3h ago   from the local cache while Azure is asked"),
    ]
}

/// Render the help screen as a modal overlay, scrolled by `scroll` lines
/// (clamped and written back).
pub fn render_help(f: &mut Frame, area: Rect, scroll: &mut usize) {
    let modal_area = centered_rect(90, 90, area);
    super::theme::clear(f, modal_area);

    let lines = help_lines();
    let viewport = modal_area.height.saturating_sub(2) as usize;
    // Long lines wrap, so count the rows they take, not the lines.
    let inner_width = modal_area.width.saturating_sub(2).max(1) as usize;
    let rows: usize = lines
        .iter()
        .map(|l| {
            let rows = l.width().max(1).div_ceil(inner_width);
            // Word wrapping can need one more row than the width suggests.
            if rows > 1 {
                rows + 1
            } else {
                rows
            }
        })
        .sum();
    *scroll = (*scroll).min(rows.saturating_sub(viewport));
    let help_paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .title(" Help ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme().accent)),
        )
        .wrap(Wrap { trim: false })
        .alignment(Alignment::Left)
        .scroll((*scroll as u16, 0));

    f.render_widget(help_paragraph, modal_area);
}

/// Helper function to create a centered rectangle
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
    use super::*;

    #[test]
    fn help_documents_every_command_filter_and_key() {
        let text: String = help_lines().iter().map(|l| l.to_string() + "\n").collect();
        for needle in [
            ":mg",
            ":sub",
            ":rg",
            ":res",
            ":type",
            ":tag",
            "type=",
            "loc=",
            "tag=",
            "Examples",
            "Ctrl+P",
            "Tab completes",
            "  y ",
            "  c ",
            "  o ",
            "/text",
        ] {
            assert!(text.contains(needle), "help lacks {needle:?}");
        }
    }

    #[test]
    fn test_centered_rect() {
        let area = Rect::new(0, 0, 100, 100);
        let centered = centered_rect(80, 80, area);

        // Should be centered
        assert!(centered.x > 0);
        assert!(centered.y > 0);
        assert!(centered.width < area.width);
        assert!(centered.height < area.height);
    }

    #[test]
    fn test_centered_rect_full_size() {
        let area = Rect::new(0, 0, 100, 100);
        let centered = centered_rect(100, 100, area);

        // Should be same as original
        assert_eq!(centered.width, area.width);
        assert_eq!(centered.height, area.height);
    }
}
