use super::theme::theme;
use c9s_core::state::{AppState, ViewMode};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

/// Breadcrumb navigation component
///
/// Renders a breadcrumb trail showing the current navigation path,
/// similar to k9s context display.
pub struct Breadcrumb;

impl Breadcrumb {
    /// Render the breadcrumb navigation line
    ///
    /// Format: c9s > [Subscription] > [View] [| Filter: xxx]
    pub fn render(state: &AppState) -> Line<'static> {
        let mut spans = vec![Span::styled(
            "c9s",
            Style::default()
                .fg(theme().accent)
                .add_modifier(Modifier::BOLD),
        )];

        // The tenant, once one is chosen with :ctx
        if let Some(tenant) = state.tenant_label() {
            spans.push(Span::raw(" > "));
            spans.push(Span::styled(
                format!("[{}]", Self::truncate(&tenant, 30)),
                Style::default().fg(theme().special),
            ));
        }

        // Add the scope: a management group (cross-subscription lists opened
        // from the tree) or the selected subscription.
        let group = state
            .scope_group
            .as_deref()
            .filter(|_| state.cross_subscription_mode)
            .and_then(|g| state.scope_tree.as_ref()?.label_of(g));
        if let Some(group) = group {
            spans.push(Span::raw(" > "));
            spans.push(Span::styled(
                Self::truncate(group, 30),
                Style::default().fg(theme().highlight),
            ));
        } else if let Some(sub) = &state.selected_subscription {
            spans.push(Span::raw(" > "));
            spans.push(Span::styled(
                Self::truncate(&sub.name, 30),
                Style::default().fg(theme().highlight),
            ));
        }

        // Add current view
        spans.push(Span::raw(" > "));
        spans.push(Span::styled(
            Self::view_name(&state.view, state.provider.terms()),
            Style::default().fg(theme().text),
        ));

        // Add filter indicator if active
        if !state.filter.is_empty() {
            spans.push(Span::raw(" | "));
            spans.push(Span::styled(
                format!("Filter: {}", state.filter),
                Style::default()
                    .fg(theme().special)
                    .add_modifier(Modifier::ITALIC),
            ));
        }

        // Add loading indicator
        if state.loading {
            spans.push(Span::raw("  "));
            spans.push(Span::styled("⟳", Style::default().fg(theme().ok)));
        }

        Line::from(spans)
    }

    /// Get human-readable view name
    fn view_name(view: &ViewMode, terms: c9s_core::provider::Terms) -> &'static str {
        match view {
            ViewMode::ManagementGroupTree => terms.tree,
            ViewMode::SubscriptionList => terms.accounts,
            ViewMode::ResourceGroupList => terms.containers,
            ViewMode::ResourceList => "Resources",
            ViewMode::ResourceTypeSummary => "Resource Types",
            ViewMode::TagSummary => "Tags",
            ViewMode::TenantList => "Contexts",
            ViewMode::PolicyAssignments => "Policy",
            ViewMode::PolicyNonCompliant => "Non-compliant",
            ViewMode::ComplianceRanking => "Compliance",
            ViewMode::ExemptionList => "Exemptions",
            ViewMode::RoleAssignments => "Role assignments",
            ViewMode::RoleDefinitions => "Roles",
            ViewMode::GroupList => "Groups",
            ViewMode::AppList => "Applications",
            ViewMode::AppDetail => "Application",
            ViewMode::GroupMembers => "Members",
            ViewMode::MemberOf => "Member of",
            ViewMode::ResourceDetail => "Resource Detail",
            ViewMode::Logs => "Logs",
            ViewMode::Cost => "Cost",
        }
    }

    /// Truncate long strings with ellipsis
    ///
    /// This ensures breadcrumbs don't overflow on narrow terminals
    fn truncate(s: &str, max_len: usize) -> String {
        // Character-based: byte slicing panics on non-ASCII names.
        super::widgets::truncate_with_ellipsis(s, max_len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use c9s_core::models::Subscription;

    #[test]
    fn test_breadcrumb_basic() {
        let state = AppState::default();
        let line = Breadcrumb::render(&state);

        // Should contain c9s and Subscriptions
        let text = format!("{:?}", line);
        assert!(text.contains("c9s"));
        assert!(text.contains("Subscriptions"));
    }

    #[test]
    fn test_breadcrumb_with_subscription() {
        let mut state = AppState::default();
        state.selected_subscription = Some(Subscription {
            id: "sub-123".to_string(),
            name: "Production".to_string(),
            state: "Enabled".to_string(),
            tenant_id: "tenant-123".to_string(),
        });

        let line = Breadcrumb::render(&state);
        let text = format!("{:?}", line);

        assert!(text.contains("Production"));
    }

    #[test]
    fn test_breadcrumb_truncation() {
        let mut state = AppState::default();
        state.selected_subscription = Some(Subscription {
            id: "sub-123".to_string(),
            name: "Very Long Subscription Name That Exceeds The Maximum Length Allowed".to_string(),
            state: "Enabled".to_string(),
            tenant_id: "tenant-123".to_string(),
        });

        let line = Breadcrumb::render(&state);
        let text = format!("{:?}", line);

        // Should be truncated with ellipsis
        assert!(text.contains("..."));
    }

    #[test]
    fn test_breadcrumb_with_filter() {
        let mut state = AppState::default();
        state.filter = "vm".to_string();

        let line = Breadcrumb::render(&state);
        let text = format!("{:?}", line);

        assert!(text.contains("Filter"));
        assert!(text.contains("vm"));
    }

    #[test]
    fn test_breadcrumb_loading() {
        let mut state = AppState::default();
        state.loading = true;

        let line = Breadcrumb::render(&state);
        let text = format!("{:?}", line);

        assert!(text.contains("⟳"));
    }

    #[test]
    fn test_truncate_non_ascii_does_not_panic() {
        let result = Breadcrumb::truncate("Zarządzanie-grupą-główną-ąęółżź", 10);
        assert_eq!(result.chars().count(), 10);
    }

    #[test]
    fn test_truncate_short_string() {
        let result = Breadcrumb::truncate("Short", 20);
        assert_eq!(result, "Short");
    }

    #[test]
    fn test_truncate_long_string() {
        let result = Breadcrumb::truncate("This is a very long string", 10);
        assert_eq!(result, "This is...");
        assert_eq!(result.len(), 10);
    }

    #[test]
    fn test_view_names() {
        assert_eq!(
            Breadcrumb::view_name(
                &ViewMode::SubscriptionList,
                c9s_core::provider::Provider::Azure.terms()
            ),
            "Subscriptions"
        );
        assert_eq!(
            Breadcrumb::view_name(
                &ViewMode::ResourceList,
                c9s_core::provider::Provider::Azure.terms()
            ),
            "Resources"
        );
        assert_eq!(
            Breadcrumb::view_name(
                &ViewMode::ResourceGroupList,
                c9s_core::provider::Provider::Azure.terms()
            ),
            "Resource Groups"
        );
        assert_eq!(
            Breadcrumb::view_name(
                &ViewMode::ResourceDetail,
                c9s_core::provider::Provider::Azure.terms()
            ),
            "Resource Detail"
        );
        assert_eq!(
            Breadcrumb::view_name(&ViewMode::Logs, c9s_core::provider::Provider::Azure.terms()),
            "Logs"
        );
    }
}
