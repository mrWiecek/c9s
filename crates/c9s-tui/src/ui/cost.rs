//! `:cost`: a bar chart of cost over time above a table of cost per group.

use super::theme::theme;
use super::widgets::{cell, list_item_spans, SelectableList, SelectableListState};
use crate::TuiApp;
use anyhow::Result;
use c9s_core::cost::{bar_column, bar_rows, format_amount, CostReport, Granularity};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, StatefulWidget, Wrap},
    Frame,
};

pub(super) fn draw_cost(f: &mut Frame, area: Rect, app: &mut TuiApp) -> Result<()> {
    let state = app.state();
    let Some(query) = state.cost_query.clone() else {
        return Ok(());
    };
    let settings = state.cost_settings;
    let Some(report) = state.cost_report().cloned() else {
        let message = if state.loading {
            format!(
                "Loading the cost of {} ({})...\n\nCost Management can take a few seconds.",
                query.scope.label(),
                settings.period.label()
            )
        } else {
            "No cost loaded.\n\nPress 'r' to load it again.".to_string()
        };
        let content = Paragraph::new(message)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("Cost · {}", query.scope.label())),
            )
            .style(Style::default().fg(theme().subtle))
            .wrap(Wrap { trim: true });
        f.render_widget(content, area);
        return Ok(());
    };

    // The chart takes up to half the height (at least room for 3 bar rows).
    let chart_height = (area.height / 2).clamp(7, 16).min(area.height);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(chart_height), Constraint::Min(0)])
        .split(area);
    draw_chart(f, chunks[0], app, &report);
    draw_table(f, chunks[1], app, &report);
    Ok(())
}

fn draw_chart(f: &mut Frame, area: Rect, app: &mut TuiApp, report: &CostReport) {
    let state = app.state();
    let settings = state.cost_settings;
    let usd = settings.usd;
    let query = &report.query;
    let currency = report.currency_label(usd).to_string();
    let series = report.series(usd);
    let total = report.total(usd);

    let title = format!(
        "Cost · {} · {} ({} – {}) · {} · {} · {currency}",
        query.scope.label(),
        settings.period.label(),
        query.from.format("%Y-%m-%d"),
        query.to.format("%Y-%m-%d"),
        query.granularity.label(),
        if query.amortized {
            "amortized"
        } else {
            "actual"
        },
    );
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 3 || inner.width < 12 {
        return;
    }

    // Left: the scale. Right: the bars.
    let max = series.iter().map(|(_, v)| *v).fold(0.0_f64, f64::max);
    let axis = format_amount(max).chars().count().max(4) + 1;
    let width = (inner.width as usize).saturating_sub(axis);
    let bars_height = (inner.height as usize).saturating_sub(2);
    let values: Vec<f64> = series.iter().map(|(_, v)| *v).collect();
    let (rows, per) = bar_rows(&values, width, bars_height);
    let (column, _) = bar_column(values.len(), width);
    let bars = values.len().div_ceil(per);
    let selected = state
        .cost_bar
        .unwrap_or(values.len().saturating_sub(1))
        .min(values.len().saturating_sub(1));
    let selected_bar = selected / per;

    // The highlighted bar: its date(s) and amount.
    let picked = {
        let start = selected_bar * per;
        let end = (start + per).min(series.len());
        let amount: f64 = series[start..end].iter().map(|(_, v)| v).sum();
        let first = series[start].0;
        let last = series[end - 1].0;
        let when = match query.granularity {
            Granularity::Daily if first == last => first.format("%a %Y-%m-%d").to_string(),
            Granularity::Daily => {
                format!("{} – {}", first.format("%Y-%m-%d"), last.format("%Y-%m-%d"))
            }
            Granularity::Monthly if first == last => first.format("%B %Y").to_string(),
            Granularity::Monthly => format!("{} – {}", first.format("%b %Y"), last.format("%b %Y")),
        };
        format!("{when}: {} {currency}", format_amount(amount))
    };
    let average = match query.granularity {
        Granularity::Daily => "per day",
        Granularity::Monthly => "per month",
    };
    let mut summary = vec![
        Span::styled("Total ", Style::default().fg(theme().muted)),
        Span::styled(
            format!("{} {currency}", format_amount(total)),
            Style::default()
                .fg(theme().highlight)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "  ·  {} {average} on average  ·  ",
                format_amount(total / series.len().max(1) as f64)
            ),
            Style::default().fg(theme().muted),
        ),
        Span::styled(picked, Style::default().fg(theme().special)),
    ];
    if per > 1 {
        summary.push(Span::styled(
            format!(
                "  (each bar: {per} {})",
                match query.granularity {
                    Granularity::Daily => "days",
                    Granularity::Monthly => "months",
                }
            ),
            Style::default().fg(theme().subtle),
        ));
    }
    if usd && !report.has_usd() {
        summary.push(Span::styled(
            "  (no USD amounts: billing currency)",
            Style::default().fg(theme().subtle),
        ));
    }

    let mut lines = vec![Line::from(summary)];
    for (i, row) in rows.iter().enumerate() {
        let label = if i == 0 {
            format_amount(max)
        } else if i + 1 == rows.len() {
            "0".to_string()
        } else {
            String::new()
        };
        let mut spans = vec![Span::styled(
            format!("{label:>w$} ", w = axis - 1),
            Style::default().fg(theme().subtle),
        )];
        let chars: Vec<char> = row.chars().collect();
        for bar in 0..bars {
            let piece: String = chars.iter().skip(bar * column).take(column).collect();
            let color = if bar == selected_bar {
                theme().highlight
            } else {
                theme().accent
            };
            spans.push(Span::styled(piece, Style::default().fg(color)));
        }
        lines.push(Line::from(spans));
    }
    // Dates under the first, highlighted and last bars.
    let mut axis_line = vec![' '; axis + bars * column];
    let label = |d| query.granularity.axis_label(d);
    let mut place = |at: usize, text: &str| {
        for (k, ch) in text.chars().enumerate() {
            if let Some(slot) = axis_line.get_mut(at + k) {
                *slot = ch;
            }
        }
    };
    // The marker first; a date only where it doesn't cover the marker.
    let marker = axis + selected_bar * column;
    place(marker, "▲");
    if let (Some(first), Some(last)) = (series.first(), series.last()) {
        let start = label(first.0);
        if axis + start.chars().count() < marker {
            place(axis, &start);
        }
        let end = label(last.0);
        let at = (axis + bars * column).saturating_sub(end.chars().count());
        if at > marker {
            place(at, &end);
        }
    }
    lines.push(Line::styled(
        axis_line.into_iter().collect::<String>(),
        Style::default().fg(theme().subtle),
    ));
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_table(f: &mut Frame, area: Rect, app: &mut TuiApp, report: &CostReport) {
    let rows = app.state().cost_rows();
    let state = app.state();
    let usd = state.cost_settings.usd;
    let currency = report.currency_label(usd).to_string();
    let group = report.query.group_by.label();
    let drill = match report.query.group_by {
        c9s_core::cost::CostGroupBy::ResourceGroup | c9s_core::cost::CostGroupBy::Subscription => {
            " · Enter: its cost"
        }
        _ => "",
    };
    let title = if state.filter_mode {
        format!(
            "Cost by {group} ({}) - Filter: {}_",
            rows.len(),
            state.filter
        )
    } else {
        format!("Cost by {group} ({}){drill}", rows.len())
    };
    if rows.is_empty() {
        let message = if !state.filter.is_empty() {
            "Nothing matches the filter.\n\nPress Esc to clear it."
        } else {
            "No cost in this period."
        };
        let content = Paragraph::new(message)
            .block(Block::default().borders(Borders::ALL).title(title))
            .style(Style::default().fg(theme().subtle))
            .wrap(Wrap { trim: true });
        f.render_widget(content, area);
        return;
    }
    let inner_width = area.width.saturating_sub(2) as usize;
    let amounts: Vec<String> = rows.iter().map(|r| format_amount(r.cost)).collect();
    let amount_w = amounts.iter().map(|a| a.chars().count()).max().unwrap_or(4) + 1;
    let share_w = 8;
    let bar_w = 20.min(inner_width / 5);
    let name_w = inner_width
        .saturating_sub(amount_w + currency.chars().count() + 2 + share_w + bar_w + 2)
        .clamp(10, 60);
    let viewport = area.height.saturating_sub(2) as usize;
    let mut list_state = state.list_state.clone();
    let visible = list_state.visible_range(rows.len(), viewport);
    let items: Vec<_> = rows[visible.clone()]
        .iter()
        .zip(&amounts[visible.clone()])
        .map(|(r, amount)| {
            let filled = (r.share / 100.0 * bar_w as f64).round() as usize;
            list_item_spans(vec![
                Span::styled(
                    cell(&r.name, name_w),
                    Style::default()
                        .fg(theme().text)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{amount:>amount_w$} {currency}  "),
                    Style::default().fg(theme().highlight),
                ),
                Span::styled(
                    format!("{:>6.1}%  ", r.share),
                    Style::default().fg(theme().muted),
                ),
                Span::styled(
                    "█".repeat(filled.min(bar_w)),
                    Style::default().fg(theme().accent),
                ),
            ])
        })
        .collect();
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
