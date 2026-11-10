//! Cost (`:cost`): what a subscription, resource group or management group
//! cost over a period, per day or month, split by resource group, resource
//! type, service or subscription.
//!
//! One Cost Management query answers a view: cost per (day or month,
//! group). The chart sums the groups per day; the table sums the days per
//! group. Both currencies come back together (the billing currency and
//! USD), so switching currency doesn't ask Azure again.

pub use c9s_azure::cost::CostTable;
use chrono::{Datelike, Months, NaiveDate};
use serde_json::Value;
use std::collections::HashMap;

/// The time range, ending today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CostPeriod {
    #[default]
    Days30,
    /// This month and the two before it.
    Months3,
    /// This month and the five before it.
    Months6,
}

impl CostPeriod {
    pub const ALL: [CostPeriod; 3] = [CostPeriod::Days30, CostPeriod::Months3, CostPeriod::Months6];

    pub fn label(self) -> &'static str {
        match self {
            CostPeriod::Days30 => "last 30 days",
            CostPeriod::Months3 => "last 3 months",
            CostPeriod::Months6 => "last 6 months",
        }
    }

    /// First and last day (both included) when today is `today`.
    pub fn range(self, today: NaiveDate) -> (NaiveDate, NaiveDate) {
        let months = |n: u32| {
            today
                .with_day(1)
                .and_then(|first| first.checked_sub_months(Months::new(n)))
                .unwrap_or(today)
        };
        let from = match self {
            CostPeriod::Days30 => today - chrono::Duration::days(29),
            CostPeriod::Months3 => months(2),
            CostPeriod::Months6 => months(5),
        };
        (from, today)
    }

    pub fn next(self) -> Self {
        next_of(&Self::ALL, self)
    }
}

/// One bar per day or per month.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Granularity {
    #[default]
    Daily,
    Monthly,
}

impl Granularity {
    pub fn label(self) -> &'static str {
        match self {
            Granularity::Daily => "daily",
            Granularity::Monthly => "monthly",
        }
    }

    /// The API's name for it.
    pub fn api(self) -> &'static str {
        match self {
            Granularity::Daily => "Daily",
            Granularity::Monthly => "Monthly",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Granularity::Daily => Granularity::Monthly,
            Granularity::Monthly => Granularity::Daily,
        }
    }

    /// A short label for a bar on the chart's axis ("11-03", "Nov").
    pub fn axis_label(self, date: NaiveDate) -> String {
        match self {
            Granularity::Daily => date.format("%m-%d").to_string(),
            Granularity::Monthly => date.format("%b").to_string(),
        }
    }

    /// The day `date`'s bar starts on.
    pub fn bucket(self, date: NaiveDate) -> NaiveDate {
        match self {
            Granularity::Daily => date,
            Granularity::Monthly => date.with_day(1).unwrap_or(date),
        }
    }
}

/// What the table splits the cost by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CostGroupBy {
    #[default]
    ResourceGroup,
    ResourceType,
    Service,
    Subscription,
}

impl CostGroupBy {
    pub const ALL: [CostGroupBy; 4] = [
        CostGroupBy::ResourceGroup,
        CostGroupBy::ResourceType,
        CostGroupBy::Service,
        CostGroupBy::Subscription,
    ];

    pub fn label(self) -> &'static str {
        match self {
            CostGroupBy::ResourceGroup => "resource group",
            CostGroupBy::ResourceType => "resource type",
            CostGroupBy::Service => "service",
            CostGroupBy::Subscription => "subscription",
        }
    }

    /// The Cost Management dimension.
    pub fn dimension(self) -> &'static str {
        match self {
            CostGroupBy::ResourceGroup => "ResourceGroupName",
            CostGroupBy::ResourceType => "ResourceType",
            CostGroupBy::Service => "ServiceName",
            CostGroupBy::Subscription => "SubscriptionId",
        }
    }

    /// The groupings that mean something at `scope` (a resource group has
    /// one resource group; a subscription one subscription).
    pub fn options(scope: &CostScope) -> &'static [CostGroupBy] {
        match scope {
            CostScope::ManagementGroup { .. } => &Self::ALL,
            CostScope::Subscription { .. } => &Self::ALL[..3],
            CostScope::ResourceGroup { .. } => &Self::ALL[1..3],
        }
    }

    /// The next grouping offered at `scope`.
    pub fn next_at(self, scope: &CostScope) -> Self {
        next_of(Self::options(scope), self)
    }

    /// This grouping if `scope` offers it, else its first.
    pub fn at(self, scope: &CostScope) -> Self {
        let options = Self::options(scope);
        if options.contains(&self) {
            self
        } else {
            options[0]
        }
    }
}

fn next_of<T: Copy + PartialEq>(all: &[T], current: T) -> T {
    let at = all.iter().position(|x| *x == current).unwrap_or(0);
    all[(at + 1) % all.len()]
}

/// Where the cost is read.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum CostScope {
    ManagementGroup {
        id: String,
        name: String,
    },
    Subscription {
        id: String,
        name: String,
    },
    ResourceGroup {
        subscription_id: String,
        subscription_name: String,
        name: String,
    },
}

impl CostScope {
    /// The ARM scope.
    pub fn arm_id(&self) -> String {
        match self {
            CostScope::ManagementGroup { id, .. } => {
                format!("/providers/Microsoft.Management/managementGroups/{id}")
            }
            CostScope::Subscription { id, .. } => format!("/subscriptions/{id}"),
            CostScope::ResourceGroup {
                subscription_id,
                name,
                ..
            } => format!("/subscriptions/{subscription_id}/resourceGroups/{name}"),
        }
    }

    /// For titles: "sub-prod", "sub-prod / rg-app", "Platform (group)".
    pub fn label(&self) -> String {
        match self {
            CostScope::ManagementGroup { name, .. } => format!("{name} (management group)"),
            CostScope::Subscription { name, .. } => name.clone(),
            CostScope::ResourceGroup {
                subscription_name,
                name,
                ..
            } => format!("{subscription_name} / {name}"),
        }
    }
}

/// How the cost view is set up: kept across scopes, so moving between
/// subscriptions compares like with like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CostSettings {
    pub period: CostPeriod,
    pub granularity: Granularity,
    pub group_by: CostGroupBy,
    /// Amortized cost (reservations spread over their term) instead of
    /// actual.
    pub amortized: bool,
    /// USD instead of the billing currency.
    pub usd: bool,
}

/// One cost report to load: scope, dates, grouping and cost type. (The
/// currency is not part of it: both come back.)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CostQuery {
    pub scope: CostScope,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub granularity: Granularity,
    pub group_by: CostGroupBy,
    pub amortized: bool,
}

impl CostQuery {
    pub fn new(scope: CostScope, settings: &CostSettings, today: NaiveDate) -> Self {
        let (from, to) = settings.period.range(today);
        Self {
            group_by: settings.group_by.at(&scope),
            scope,
            from,
            to,
            granularity: settings.granularity,
            amortized: settings.amortized,
        }
    }

    /// The API request.
    pub fn api(&self) -> c9s_azure::cost::CostQuery {
        c9s_azure::cost::CostQuery {
            scope: self.scope.arm_id(),
            amortized: self.amortized,
            from: self.from.format("%Y-%m-%d").to_string(),
            to: self.to.format("%Y-%m-%d").to_string(),
            granularity: self.granularity.api(),
            group_by: Some(self.group_by.dimension().to_string()),
        }
    }

    /// Every bar's first day, in order (also those without cost).
    pub fn buckets(&self) -> Vec<NaiveDate> {
        let mut out = Vec::new();
        let mut day = self.granularity.bucket(self.from);
        while day <= self.to {
            out.push(day);
            day = match self.granularity {
                Granularity::Daily => day + chrono::Duration::days(1),
                Granularity::Monthly => match day.checked_add_months(Months::new(1)) {
                    Some(d) => d,
                    None => break,
                },
            };
        }
        out
    }
}

/// Cost of one group on one day (or month).
#[derive(Debug, Clone, PartialEq)]
pub struct CostEntry {
    pub date: NaiveDate,
    pub group: String,
    pub cost: f64,
    /// The cost in USD, when Azure says it.
    pub cost_usd: Option<f64>,
}

/// A loaded cost report.
#[derive(Debug, Clone, PartialEq)]
pub struct CostReport {
    pub query: CostQuery,
    /// The billing currency ("EUR"), empty if Azure didn't say.
    pub currency: String,
    pub entries: Vec<CostEntry>,
}

/// One row of the table: a group and its cost over the period.
#[derive(Debug, Clone, PartialEq)]
pub struct CostGroupRow {
    /// What Azure calls it (a resource group name, a type, a service...;
    /// for subscriptions, the ID).
    pub key: String,
    /// What to show (a subscription's name).
    pub name: String,
    pub cost: f64,
    /// Share of the total, 0-100.
    pub share: f64,
}

impl CostReport {
    /// The report from Cost Management's table: columns are found by name
    /// (`Cost` or `PreTaxCost`, `CostUSD`, `UsageDate` or `BillingMonth`,
    /// the grouping dimension, `Currency`).
    pub fn from_table(query: CostQuery, table: &CostTable) -> Result<Self, String> {
        let find = |names: &[&str]| {
            table
                .columns
                .iter()
                .position(|c| names.iter().any(|n| c.eq_ignore_ascii_case(n)))
        };
        let cost = find(&["Cost", "PreTaxCost", "totalCost"])
            .ok_or("the cost report has no cost column")?;
        let usd = find(&["CostUSD", "PreTaxCostUSD", "totalCostUSD"]);
        let date = find(&["UsageDate", "BillingMonth", "Date"]);
        let group = find(&[query.group_by.dimension()]);
        let currency = find(&["Currency", "BillingCurrency"]);
        let mut entries = Vec::with_capacity(table.rows.len());
        let mut currencies: HashMap<String, usize> = HashMap::new();
        for row in &table.rows {
            let Some(amount) = row.get(cost).and_then(number) else {
                continue;
            };
            let day = date
                .and_then(|i| row.get(i))
                .and_then(parse_date)
                .unwrap_or(query.from);
            let name = group
                .and_then(|i| row.get(i))
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Null => String::new(),
                    other => other.to_string(),
                })
                .unwrap_or_default();
            if let Some(c) = currency.and_then(|i| row.get(i)).and_then(Value::as_str) {
                *currencies.entry(c.to_string()).or_default() += 1;
            }
            entries.push(CostEntry {
                date: query.granularity.bucket(day),
                group: name,
                cost: amount,
                cost_usd: usd.and_then(|i| row.get(i)).and_then(number),
            });
        }
        let currency = currencies
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
            .map(|(c, _)| c)
            .unwrap_or_default();
        Ok(Self {
            query,
            currency,
            entries,
        })
    }

    /// Whether every amount is known in USD.
    pub fn has_usd(&self) -> bool {
        self.entries.iter().all(|e| e.cost_usd.is_some())
    }

    fn amount(&self, e: &CostEntry, usd: bool) -> f64 {
        if usd {
            e.cost_usd.unwrap_or(e.cost)
        } else {
            e.cost
        }
    }

    /// The currency amounts are in.
    pub fn currency_label(&self, usd: bool) -> &str {
        if usd && self.has_usd() {
            "USD"
        } else if self.currency.is_empty() {
            "(billing currency)"
        } else {
            &self.currency
        }
    }

    pub fn total(&self, usd: bool) -> f64 {
        self.entries.iter().map(|e| self.amount(e, usd)).sum()
    }

    /// Cost per bar, every bar of the period (zero where nothing was
    /// spent), oldest first.
    pub fn series(&self, usd: bool) -> Vec<(NaiveDate, f64)> {
        let mut sums: HashMap<NaiveDate, f64> = HashMap::new();
        for e in &self.entries {
            *sums.entry(e.date).or_default() += self.amount(e, usd);
        }
        self.query
            .buckets()
            .into_iter()
            .map(|d| (d, sums.get(&d).copied().unwrap_or(0.0)))
            .collect()
    }

    /// Cost per group over the period, most expensive first. `name` turns
    /// a key into what to show (subscription IDs into names).
    pub fn groups(&self, usd: bool, name: impl Fn(&str) -> String) -> Vec<CostGroupRow> {
        let mut sums: HashMap<String, (String, f64)> = HashMap::new();
        for e in &self.entries {
            sums.entry(e.group.to_ascii_lowercase())
                .or_insert_with(|| (e.group.clone(), 0.0))
                .1 += self.amount(e, usd);
        }
        let total: f64 = sums.values().map(|v| v.1).sum();
        let mut rows: Vec<CostGroupRow> = sums
            .into_values()
            .map(|(key, cost)| CostGroupRow {
                name: if key.is_empty() {
                    "(none)".to_string()
                } else {
                    name(&key)
                },
                key,
                cost,
                share: if total.abs() > f64::EPSILON {
                    cost / total * 100.0
                } else {
                    0.0
                },
            })
            .collect();
        rows.sort_by(|a, b| {
            b.cost
                .partial_cmp(&a.cost)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        rows
    }
}

fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// `20261101` (UsageDate) or `"2026-11-01T00:00:00"` (BillingMonth).
fn parse_date(v: &Value) -> Option<NaiveDate> {
    let text = match v {
        Value::Number(n) => n.as_i64()?.to_string(),
        Value::String(s) => s.clone(),
        _ => return None,
    };
    let text = text.trim();
    NaiveDate::parse_from_str(text, "%Y%m%d")
        .ok()
        .or_else(|| NaiveDate::parse_from_str(text.get(..10)?, "%Y-%m-%d").ok())
}

/// `1234.5` as "1,234.50"; large amounts without cents ("123,457").
pub fn format_amount(amount: f64) -> String {
    let negative = amount < 0.0;
    let amount = amount.abs();
    let (whole, cents) = if amount >= 100_000.0 {
        (amount.round() as u64, None)
    } else {
        let c = (amount * 100.0).round() as u64;
        (c / 100, Some(c % 100))
    };
    let digits = whole.to_string();
    let mut grouped = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let sign = if negative { "-" } else { "" };
    match cents {
        Some(c) => format!("{sign}{grouped}.{c:02}"),
        None => format!("{sign}{grouped}"),
    }
}

/// Vertical bars, one text line per row (top first), `width` columns wide:
/// each value gets an equal column (wider when there are few), with
/// eighth-block resolution. With more values than columns, neighbours
/// share a column (summed); the result says how many share one.
pub fn bar_rows(values: &[f64], width: usize, height: usize) -> (Vec<String>, usize) {
    if values.is_empty() || width == 0 || height == 0 {
        return (vec![String::new(); height], 1);
    }
    let per = values.len().div_ceil(width).max(1);
    let merged: Vec<f64> = values.chunks(per).map(|c| c.iter().sum()).collect();
    let n = merged.len();
    let column = (width / n).clamp(1, 8);
    // A gap between bars once they are wide enough to spare one.
    let (bar, gap) = if column >= 3 {
        (column - 1, 1)
    } else {
        (column, 0)
    };
    let max = merged.iter().cloned().fold(0.0_f64, f64::max);
    const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let eighths: Vec<usize> = merged
        .iter()
        .map(|v| {
            if max <= 0.0 || *v <= 0.0 {
                0
            } else {
                // Anything spent shows at least a sliver.
                ((v / max) * (height * 8) as f64).round().max(1.0) as usize
            }
        })
        .collect();
    let mut rows = Vec::with_capacity(height);
    for level in (0..height).rev() {
        let mut line = String::new();
        for e in &eighths {
            let fill = e.saturating_sub(level * 8).min(8);
            let ch = if fill == 0 { ' ' } else { BLOCKS[fill - 1] };
            for _ in 0..bar {
                line.push(ch);
            }
            for _ in 0..gap {
                line.push(' ');
            }
        }
        rows.push(line);
    }
    (rows, per)
}

/// The column width [`bar_rows`] gives each bar (gap included), so a
/// caller can place labels and a cursor under the bars.
pub fn bar_column(count: usize, width: usize) -> (usize, usize) {
    if count == 0 || width == 0 {
        return (1, 1);
    }
    let per = count.div_ceil(width).max(1);
    let n = count.div_ceil(per);
    ((width / n).clamp(1, 8), per)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn sub() -> CostScope {
        CostScope::Subscription {
            id: "s1".into(),
            name: "sub-prod".into(),
        }
    }

    #[test]
    fn periods_end_today() {
        let today = day("2026-11-10");
        assert_eq!(CostPeriod::Days30.range(today), (day("2026-10-12"), today));
        assert_eq!(CostPeriod::Months3.range(today), (day("2026-09-01"), today));
        assert_eq!(CostPeriod::Months6.range(today), (day("2026-06-01"), today));
        // Across a year boundary.
        assert_eq!(
            CostPeriod::Months3.range(day("2027-01-31")).0,
            day("2026-11-01")
        );
    }

    #[test]
    fn buckets_cover_the_period() {
        let settings = CostSettings::default();
        let q = CostQuery::new(sub(), &settings, day("2026-11-10"));
        let b = q.buckets();
        assert_eq!(b.len(), 30);
        assert_eq!(b[0], day("2026-10-12"));
        let monthly = CostQuery::new(
            sub(),
            &CostSettings {
                period: CostPeriod::Months6,
                granularity: Granularity::Monthly,
                ..settings
            },
            day("2026-11-10"),
        );
        assert_eq!(
            monthly.buckets(),
            [
                "2026-06-01",
                "2026-07-01",
                "2026-08-01",
                "2026-09-01",
                "2026-10-01",
                "2026-11-01"
            ]
            .map(day)
        );
    }

    #[test]
    fn groupings_fit_the_scope() {
        let rg = CostScope::ResourceGroup {
            subscription_id: "s1".into(),
            subscription_name: "sub".into(),
            name: "rg".into(),
        };
        // A resource group isn't split by resource group.
        assert_eq!(
            CostGroupBy::ResourceGroup.at(&rg),
            CostGroupBy::ResourceType
        );
        assert_eq!(CostGroupBy::Service.next_at(&rg), CostGroupBy::ResourceType);
        assert_eq!(
            CostGroupBy::Service.next_at(&sub()),
            CostGroupBy::ResourceGroup
        );
        assert_eq!(
            CostGroupBy::Subscription.at(&sub()),
            CostGroupBy::ResourceGroup
        );
        assert_eq!(rg.arm_id(), "/subscriptions/s1/resourceGroups/rg");
    }

    fn report() -> CostReport {
        let q = CostQuery::new(sub(), &CostSettings::default(), day("2026-11-10"));
        let table = CostTable {
            columns: [
                "Cost",
                "CostUSD",
                "UsageDate",
                "ResourceGroupName",
                "Currency",
            ]
            .map(String::from)
            .to_vec(),
            rows: vec![
                vec![
                    json!(10.0),
                    json!(11.0),
                    json!(20261101),
                    json!("rg-a"),
                    json!("EUR"),
                ],
                vec![
                    json!(5.5),
                    json!(6.0),
                    json!(20261101),
                    json!("RG-B"),
                    json!("EUR"),
                ],
                vec![
                    json!(2.0),
                    json!(2.2),
                    json!(20261103),
                    json!("rg-a"),
                    json!("EUR"),
                ],
                vec![
                    json!("1.5"),
                    json!(1.6),
                    json!(20261103),
                    json!("rg-b"),
                    json!("EUR"),
                ],
                vec![
                    json!(0.5),
                    json!(0.6),
                    json!(20261104),
                    json!(""),
                    json!("EUR"),
                ],
            ],
        };
        CostReport::from_table(q, &table).unwrap()
    }

    #[test]
    fn a_report_sums_per_day_and_per_group() {
        let r = report();
        assert_eq!(r.currency, "EUR");
        assert!(r.has_usd());
        assert!((r.total(false) - 19.5).abs() < 1e-9);
        assert!((r.total(true) - 21.4).abs() < 1e-9);
        assert_eq!(r.currency_label(true), "USD");

        let series = r.series(false);
        assert_eq!(series.len(), 30, "every day, also those without cost");
        let on = |d: &str| series.iter().find(|(x, _)| *x == day(d)).unwrap().1;
        assert!((on("2026-11-01") - 15.5).abs() < 1e-9);
        assert_eq!(on("2026-11-02"), 0.0);

        // Group names differ in case only: one group.
        let groups = r.groups(false, |k| k.to_string());
        let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["rg-a", "RG-B", "(none)"]);
        assert!((groups[1].cost - 7.0).abs() < 1e-9);
        assert!((groups.iter().map(|g| g.share).sum::<f64>() - 100.0).abs() < 1e-6);
    }

    #[test]
    fn monthly_reports_read_billing_months() {
        let settings = CostSettings {
            granularity: Granularity::Monthly,
            period: CostPeriod::Months3,
            ..Default::default()
        };
        let q = CostQuery::new(sub(), &settings, day("2026-11-10"));
        let table = CostTable {
            columns: [
                "PreTaxCost",
                "BillingMonth",
                "ResourceGroupName",
                "Currency",
            ]
            .map(String::from)
            .to_vec(),
            rows: vec![
                vec![
                    json!(100.0),
                    json!("2026-09-01T00:00:00"),
                    json!("a"),
                    json!("USD"),
                ],
                vec![
                    json!(50.0),
                    json!("2026-11-01T00:00:00"),
                    json!("a"),
                    json!("USD"),
                ],
            ],
        };
        let r = CostReport::from_table(q, &table).unwrap();
        assert!(!r.has_usd());
        // Without USD amounts, USD shows the billing currency.
        assert_eq!(r.currency_label(true), "USD");
        let s = r.series(false);
        assert_eq!(
            s.iter().map(|x| x.1).collect::<Vec<_>>(),
            [100.0, 0.0, 50.0]
        );
    }

    #[test]
    fn a_table_without_cost_is_an_error() {
        let q = CostQuery::new(sub(), &CostSettings::default(), day("2026-11-10"));
        let table = CostTable {
            columns: vec!["Nothing".into()],
            rows: vec![],
        };
        assert!(CostReport::from_table(q, &table).is_err());
    }

    #[test]
    fn amounts_are_grouped() {
        assert_eq!(format_amount(0.0), "0.00");
        assert_eq!(format_amount(1234.5), "1,234.50");
        assert_eq!(format_amount(99_999.994), "99,999.99");
        assert_eq!(format_amount(1_234_567.8), "1,234,568");
        assert_eq!(format_amount(-12.345), "-12.35");
    }

    #[test]
    fn bars_scale_to_the_largest() {
        let (rows, per) = bar_rows(&[0.0, 1.0, 2.0], 9, 2);
        assert_eq!(per, 1);
        assert_eq!(rows.len(), 2);
        // Columns of 3: 2-wide bars and a gap.
        assert_eq!(rows[0], "      ██ ");
        assert_eq!(rows[1], "   ██ ██ ");
        // More values than columns: neighbours are summed.
        let (rows, per) = bar_rows(&[1.0; 10], 4, 1);
        assert_eq!(per, 3);
        assert_eq!(rows[0].chars().count(), 4);
        assert_eq!(bar_column(10, 4), (1, 3));
        // A tiny amount still shows.
        let (rows, _) = bar_rows(&[1000.0, 0.001], 2, 1);
        assert_eq!(rows[0], "█▁");
    }
}
