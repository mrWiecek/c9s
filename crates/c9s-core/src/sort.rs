//! Sorting lists by a column (`s`): which columns each view offers, and a
//! stable sort by one of them, ascending or descending.

use crate::policy::Expiry;
use crate::state::ViewMode;
use std::cmp::Ordering;

/// What a column sorts by: text (case-insensitive) or a number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SortValue {
    Text(String),
    Num(i64),
}

impl SortValue {
    pub fn text(s: &str) -> Self {
        SortValue::Text(s.to_lowercase())
    }
}

impl PartialOrd for SortValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SortValue {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (SortValue::Num(a), SortValue::Num(b)) => a.cmp(b),
            (SortValue::Text(a), SortValue::Text(b)) => a.cmp(b),
            // Numbers before text (not expected within one column).
            (SortValue::Num(_), SortValue::Text(_)) => Ordering::Less,
            (SortValue::Text(_), SortValue::Num(_)) => Ordering::Greater,
        }
    }
}

/// A sort order and how to read a row's value in a column.
pub type SortBy<'k, T> = (SortSpec, &'k dyn Fn(&T, usize) -> SortValue);

/// How a view is sorted: by which of its columns, which way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortSpec {
    pub column: usize,
    pub descending: bool,
}

impl SortSpec {
    /// For a title: "Name ↑".
    pub fn label(&self, view: &ViewMode) -> String {
        let name = columns(view).get(self.column).copied().unwrap_or("?");
        format!("{name} {}", if self.descending { "↓" } else { "↑" })
    }
}

/// The columns `s` offers in `view` (empty: the view can't be sorted).
pub fn columns(view: &ViewMode) -> &'static [&'static str] {
    match view {
        ViewMode::SubscriptionList => &["Name", "ID", "State"],
        ViewMode::ResourceGroupList => &["Name", "Location", "Subscription"],
        ViewMode::ResourceList => &["Name", "Type", "Resource group", "Location", "Subscription"],
        ViewMode::ResourceTypeSummary => &["Type", "Count"],
        ViewMode::TagSummary => &["Key", "Value", "Count"],
        ViewMode::TenantList => &["Name", "Cloud", "Subscriptions"],
        ViewMode::PolicyAssignments => &["Name", "Non-compliant", "Origin", "Enforcement"],
        ViewMode::PolicyNonCompliant => &["Resource", "Type", "Resource group", "Policies"],
        ViewMode::ComplianceRanking => &["Subscription", "Non-compliant", "Compliant %", "Group"],
        ViewMode::ExemptionList => &["Name", "Expiry", "Category", "Assignment", "Scope"],
        ViewMode::RoleAssignments => &["Role", "Principal", "Type", "Scope", "Origin", "PIM"],
        ViewMode::RoleDefinitions => &["Name", "Kind", "Assignments"],
        ViewMode::GroupList => &["Name", "Kind", "Mail", "Created"],
        ViewMode::GroupMembers | ViewMode::MemberOf => &["Name", "Kind", "Detail"],
        ViewMode::AppList => &["Name", "Kind", "App ID", "Credential expiry"],
        _ => &[],
    }
}

/// An expiry as a number that sorts soonest first (expired before
/// expiring; never last).
pub fn expiry_value(e: Option<Expiry>) -> SortValue {
    SortValue::Num(match e {
        Some(Expiry::Expired(days)) => -days,
        Some(Expiry::In(days)) => days,
        Some(Expiry::Never) | None => i64::MAX,
    })
}

/// The order of `len` rows sorted by `spec`'s column (indices into them):
/// stable, ties keep their order whichever way it sorts.
pub fn order(len: usize, spec: SortSpec, key: impl Fn(usize, usize) -> SortValue) -> Vec<usize> {
    let mut keyed: Vec<(SortValue, usize)> = (0..len).map(|i| (key(i, spec.column), i)).collect();
    keyed.sort_by(|(a, i), (b, j)| {
        let by_key = if spec.descending { b.cmp(a) } else { a.cmp(b) };
        by_key.then(i.cmp(j))
    });
    keyed.into_iter().map(|(_, i)| i).collect()
}

/// Sort `rows` in place by `spec` (unchanged for `None`).
pub fn sort<T>(rows: &mut Vec<T>, spec: Option<SortSpec>, key: impl Fn(&T, usize) -> SortValue) {
    let Some(spec) = spec else {
        return;
    };
    let order = order(rows.len(), spec, |i, c| key(&rows[i], c));
    let mut slots: Vec<Option<T>> = std::mem::take(rows).into_iter().map(Some).collect();
    rows.extend(order.into_iter().filter_map(|i| slots[i].take()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_both_ways_case_insensitive_and_numeric() {
        let mut rows = vec![("b", 2), ("A", 10), ("a", 1), ("c", 2)];
        let key = |r: &(&str, i64), c: usize| match c {
            0 => SortValue::text(r.0),
            _ => SortValue::Num(r.1),
        };
        sort(
            &mut rows,
            Some(SortSpec {
                column: 0,
                descending: false,
            }),
            key,
        );
        assert_eq!(
            rows,
            [("A", 10), ("a", 1), ("b", 2), ("c", 2)],
            "ties keep order"
        );
        sort(
            &mut rows,
            Some(SortSpec {
                column: 1,
                descending: false,
            }),
            key,
        );
        assert_eq!(
            rows,
            [("a", 1), ("b", 2), ("c", 2), ("A", 10)],
            "10 after 2"
        );
        sort(
            &mut rows,
            Some(SortSpec {
                column: 1,
                descending: true,
            }),
            key,
        );
        assert_eq!(
            rows,
            [("A", 10), ("b", 2), ("c", 2), ("a", 1)],
            "ties still in order"
        );
        let before = rows.clone();
        sort(&mut rows, None, key);
        assert_eq!(rows, before);
    }

    #[test]
    fn expiries_sort_soonest_first() {
        let mut v = vec![
            expiry_value(Some(Expiry::Never)),
            expiry_value(Some(Expiry::In(30))),
            expiry_value(Some(Expiry::Expired(3))),
            expiry_value(None),
            expiry_value(Some(Expiry::In(2))),
        ];
        v.sort();
        assert_eq!(
            v,
            [
                SortValue::Num(-3),
                SortValue::Num(2),
                SortValue::Num(30),
                SortValue::Num(i64::MAX),
                SortValue::Num(i64::MAX)
            ]
        );
    }

    #[test]
    fn every_sortable_view_names_its_columns() {
        assert_eq!(columns(&ViewMode::ResourceList)[1], "Type");
        assert!(
            columns(&ViewMode::ManagementGroupTree).is_empty(),
            "a tree keeps its shape"
        );
        let spec = SortSpec {
            column: 1,
            descending: true,
        };
        assert_eq!(spec.label(&ViewMode::ResourceList), "Type ↓");
    }
}
