use super::theme::theme;
use c9s_core::state::{format_age, AppState, DataSource};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

/// Braille spinner frames, advanced on each 100 ms tick while loading.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Status bar component
///
/// Displays connection status, context information, and statistics
/// in a compact footer bar, similar to k9s status line.
pub struct StatusBar;

impl StatusBar {
    /// Render the status bar line
    ///
    /// Format: ● Status | Context | Stats | Indicators
    pub fn render(state: &AppState) -> Line<'static> {
        let mut spans = vec![Self::connection_indicator(state), Span::raw(" | ")];

        // The context: its cloud, and the tenant when one is chosen.
        let tenant = state.tenant.as_ref().map(|id| {
            state
                .tenants
                .iter()
                .find(|t| t.id.eq_ignore_ascii_case(id))
                .map(|t| t.name.clone())
                .unwrap_or_else(|| id.clone())
        });
        // Other clouds: the gcloud configuration or AWS profile.
        let tenant = tenant
            .or_else(|| state.gcp_configuration.clone())
            .or_else(|| state.aws_profile.clone());
        spans.push(Span::styled(
            match tenant {
                Some(t) => format!("{} · {} ", state.provider.as_str(), Self::truncate(&t, 24)),
                None => format!("{} ", state.provider.as_str()),
            },
            Style::default()
                .fg(theme().accent)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw("| "));

        // Current subscription context
        if let Some(sub) = &state.selected_subscription {
            spans.push(Span::styled(
                format!(
                    "{}: {} ",
                    state.provider.terms().account_short,
                    Self::truncate(&sub.name, 20)
                ),
                Style::default().fg(theme().highlight),
            ));
            spans.push(Span::raw("| "));
        }

        // Settings worth knowing about at a glance
        if state.config.read_only {
            spans.push(Span::styled(
                "RO ",
                Style::default()
                    .fg(theme().accent)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw("| "));
        }
        if state.config.refresh_interval > 0 {
            spans.push(Span::styled(
                format!("auto ⟳ {}s ", state.config.refresh_interval),
                Style::default().fg(theme().accent),
            ));
            spans.push(Span::raw("| "));
        }

        // Item count in current view
        let count = state.current_list_len();
        if count > 0 {
            spans.push(Span::raw(format!("{} items ", count)));
            spans.push(Span::raw("| "));
        }

        // Sort indicator (s)
        if let Some(spec) = state.sort_spec(&state.view) {
            spans.push(Span::styled(
                format!("sorted by {} ", spec.label(&state.view)),
                Style::default().fg(theme().special),
            ));
            spans.push(Span::raw("| "));
        }

        // Filter indicator
        if !state.filter.is_empty() {
            spans.push(Span::styled(
                "Filtered ",
                Style::default()
                    .fg(theme().special)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw("| "));
        }

        // Loading indicator (animated while background loads run)
        if state.loading {
            let frame = SPINNER[state.spinner_frame % SPINNER.len()];
            spans.push(Span::styled(
                format!("{frame} Loading"),
                Style::default().fg(theme().ok),
            ));
        } else {
            spans.push(Span::styled("Ready", Style::default().fg(theme().ok)));
        }

        // Non-blocking notice, e.g. some subscriptions failed to load
        if let Some(notice) = &state.notice {
            spans.push(Span::raw(" | "));
            spans.push(Span::styled(
                format!("⚠ {notice}"),
                Style::default().fg(theme().highlight),
            ));
        }

        Line::from(spans)
    }

    /// Render connection status indicator
    ///
    /// Returns a colored dot with status text:
    /// - Green: Connected (data from Azure)
    /// - Yellow: Cached (data from local cache)
    /// - Red: Disconnected (no data)
    fn connection_indicator(state: &AppState) -> Span<'static> {
        // Prefer the current view's own freshness ("◐ Cached · 3h ago").
        if let Some(fresh) = state.current_freshness() {
            let age = format_age(fresh.as_of);
            let (text, color) = match fresh.source {
                DataSource::Azure => (format!("● Azure · {age}"), theme().ok),
                DataSource::Cache => (format!("◐ Cached · {age}"), theme().highlight),
                DataSource::None => ("○ Disconnected".to_string(), theme().error),
            };
            return Span::styled(
                text,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            );
        }
        match state.data_source {
            DataSource::Azure => Span::styled(
                "● Azure",
                Style::default().fg(theme().ok).add_modifier(Modifier::BOLD),
            ),
            DataSource::Cache => Span::styled(
                "◐ Cached",
                Style::default()
                    .fg(theme().highlight)
                    .add_modifier(Modifier::BOLD),
            ),
            DataSource::None => Span::styled(
                "○ Disconnected",
                Style::default()
                    .fg(theme().error)
                    .add_modifier(Modifier::BOLD),
            ),
        }
    }

    /// Truncate long strings with ellipsis (character-based, UTF-8 safe)
    fn truncate(s: &str, max_len: usize) -> String {
        super::widgets::truncate_with_ellipsis(s, max_len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use c9s_core::models::Subscription;
    use c9s_core::state::{Freshness, ViewMode};

    #[test]
    fn test_status_bar_basic() {
        let state = AppState::default();
        let line = StatusBar::render(&state);

        let text = format!("{:?}", line);
        assert!(text.contains("Disconnected"));
        assert!(text.contains("Ready"));
    }

    #[test]
    fn test_status_bar_with_subscription() {
        let mut state = AppState::default();
        state.selected_subscription = Some(Subscription {
            id: "sub-123".to_string(),
            name: "Production".to_string(),
            state: "Enabled".to_string(),
            tenant_id: "tenant-123".to_string(),
        });

        let line = StatusBar::render(&state);
        let text = format!("{:?}", line);

        assert!(text.contains("sub:"));
        assert!(text.contains("Production"));
    }

    #[test]
    fn test_status_bar_with_items() {
        let mut state = AppState::default();
        state.subscriptions = vec![
            Subscription {
                id: "sub-1".to_string(),
                name: "Sub 1".to_string(),
                state: "Enabled".to_string(),
                tenant_id: "tenant-1".to_string(),
            },
            Subscription {
                id: "sub-2".to_string(),
                name: "Sub 2".to_string(),
                state: "Enabled".to_string(),
                tenant_id: "tenant-2".to_string(),
            },
        ];

        let line = StatusBar::render(&state);
        let text = format!("{:?}", line);

        assert!(text.contains("2 items"));
    }

    #[test]
    fn test_status_bar_with_filter() {
        let mut state = AppState::default();
        state.filter = "vm".to_string();

        let line = StatusBar::render(&state);
        let text = format!("{:?}", line);

        assert!(text.contains("Filtered"));
    }

    #[test]
    fn test_status_bar_loading() {
        let mut state = AppState::default();
        state.loading = true;

        let line = StatusBar::render(&state);
        let text = format!("{:?}", line);

        assert!(text.contains("Loading"));
        assert!(text.contains(SPINNER[0]));
    }

    #[test]
    fn shows_read_only_and_auto_refresh() {
        let mut state = AppState::default();
        let text = format!("{:?}", StatusBar::render(&state));
        assert!(text.contains("RO "), "read-only is the default");
        assert!(!text.contains("auto"));
        state.config.read_only = false;
        state.config.refresh_interval = 60;
        let text = format!("{:?}", StatusBar::render(&state));
        assert!(
            !text.contains("RO ") && text.contains("auto ⟳ 60s"),
            "{text}"
        );
    }

    #[test]
    fn test_truncate_short_string() {
        let result = StatusBar::truncate("Short", 20);
        assert_eq!(result, "Short");
    }

    #[test]
    fn test_truncate_long_string() {
        let result = StatusBar::truncate("This is a very long subscription name", 15);
        assert_eq!(result, "This is a ve...");
        assert_eq!(result.len(), 15);
    }

    #[test]
    fn test_data_source_azure() {
        let mut state = AppState::default();
        state.set_data_source(DataSource::Azure);

        let line = StatusBar::render(&state);
        let text = format!("{:?}", line);

        assert!(text.contains("Azure"));
    }

    #[test]
    fn test_data_source_cache() {
        let mut state = AppState::default();
        state.set_data_source(DataSource::Cache);

        let line = StatusBar::render(&state);
        let text = format!("{:?}", line);

        assert!(text.contains("Cached"));
    }

    #[test]
    fn test_status_bar_shows_source_and_age_of_current_view() {
        let mut state = AppState::default();
        state.view = ViewMode::ResourceGroupList;
        let now = chrono::Utc::now().timestamp();
        state.freshness.insert(
            ViewMode::ResourceGroupList,
            Freshness {
                source: DataSource::Cache,
                as_of: now - 3 * 3600,
            },
        );
        state.freshness.insert(
            ViewMode::SubscriptionList,
            Freshness {
                source: DataSource::Azure,
                as_of: now,
            },
        );
        let text = format!("{:?}", StatusBar::render(&state));
        assert!(text.contains("Cached · 3h ago"), "{text}");

        state.view = ViewMode::SubscriptionList;
        let text = format!("{:?}", StatusBar::render(&state));
        assert!(text.contains("Azure · just now"), "{text}");
    }

    #[test]
    fn test_long_non_ascii_subscription_name_does_not_panic() {
        let mut state = AppState::default();
        state.selected_subscription = Some(c9s_core::models::Subscription {
            id: "id".into(),
            name: "żółćżółćżółćżółćżółć".into(), // 2-byte chars: byte 17 is mid-character,
            state: "Enabled".into(),
            tenant_id: "t".into(),
        });
        let text = format!("{:?}", StatusBar::render(&state));
        assert!(text.contains("sub:"));
    }
}
