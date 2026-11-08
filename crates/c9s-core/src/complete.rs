//! Autocomplete for `:` commands. Suggests, for the word being typed:
//!
//! - first word: command names (`mg`, `sub`, `rg`, `res`, `type`, `tag`);
//! - after `:rg`/`:res`/`:type`/`:tag`: `all`, subscription names and the
//!   filter keys (`type=`, `loc=`, `tag=`; `:rg` has no `type=`);
//! - after `type=`: aliases of the types that are loaded (or known aliases);
//! - after `loc=`: locations that are loaded;
//! - after `tag=`: tag keys, then after `tag=key=` that key's values.
//!
//! Values come from what's loaded, so they match what filters will find.

use crate::resource_type::{known_aliases, short_label};
use crate::state::AppState;
use std::collections::BTreeMap;

/// Most suggestions returned.
pub const MAX_SUGGESTIONS: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// Replaces the word being typed.
    pub text: String,
    /// Shown next to it (e.g. what the command does, or how many resources).
    pub detail: String,
}

/// Suggestions for the word at the end of `input`, and where that word
/// starts (byte offset), for [`apply`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Completion {
    pub start: usize,
    pub items: Vec<Suggestion>,
}

const COMMANDS: &[(&str, &str)] = &[
    ("mg", "management group tree"),
    ("sub", "subscriptions"),
    ("rg", "resource groups"),
    ("res", "resources"),
    ("type", "resource counts per type"),
    ("tag", "tags and how many resources carry them"),
    ("policy", "policy assignments and compliance"),
    ("compliance", "subscriptions by non-compliant resources"),
    ("exemption", "policy exemptions and when they expire"),
    ("ra", "role assignments"),
    ("role", "role definitions, built-in and custom"),
    ("access", "everything one user, group or app can do"),
    ("group", "directory groups and their members"),
    ("app", "app registrations and enterprise applications"),
    ("theme", "switch the colour theme"),
    ("ctx", "switch tenant"),
];

pub fn complete(input: &str, state: &AppState) -> Completion {
    let start = input.rfind(' ').map_or(0, |i| i + 1);
    let word = &input[start..];
    let earlier: Vec<&str> = input[..start].split_whitespace().collect();

    let items = match earlier.first() {
        None => prefixed(
            word,
            COMMANDS.iter().map(|(c, d)| (c.to_string(), d.to_string())),
        ),
        Some(cmd) => {
            let cmd = cmd.to_ascii_lowercase();
            if cmd == "ctx" {
                let mut options = vec![(
                    "all".to_string(),
                    "every tenant (default login)".to_string(),
                )];
                options.extend(state.tenant_rows().into_iter().filter_map(|r| {
                    let id = r.id?;
                    Some((r.name, r.domain.unwrap_or(id)))
                }));
                prefixed(word, options.into_iter())
            } else if cmd == "theme" {
                let options = crate::config::ThemeName::ALL
                    .iter()
                    .map(|t| (t.as_str().to_string(), "theme".to_string()));
                prefixed(word, options)
            } else if cmd == "group" {
                let mut options: Vec<(String, String)> = state
                    .groups
                    .iter()
                    .map(|g| (g.display_name.clone(), g.kind_label().to_string()))
                    .collect();
                options.sort();
                options.dedup();
                prefixed(word, options.into_iter())
            } else if cmd == "access" {
                let mut options: Vec<(String, String)> = state
                    .principals
                    .values()
                    .map(|p| (p.display_name.clone(), p.kind.clone()))
                    .collect();
                options.sort();
                options.dedup();
                prefixed(word, options.into_iter())
            } else if cmd == "compliance" || cmd == "exemption" || cmd == "ra" {
                let mut options = vec![("all".to_string(), "whole tenant".to_string())];
                if let Some(tree) = &state.scope_tree {
                    options.extend(
                        tree.groups
                            .iter()
                            .map(|g| (g.display_name.clone(), "management group".into())),
                    );
                }
                if cmd == "exemption" || cmd == "ra" {
                    options.extend(
                        state
                            .subscriptions
                            .iter()
                            .filter(|s| state.in_tenant(s))
                            .map(|s| (s.name.clone(), "subscription".into())),
                    );
                }
                prefixed(word, options.into_iter())
            } else if cmd == "policy" {
                let mut options = vec![("all".to_string(), "whole tenant".to_string())];
                options.extend(
                    state
                        .subscriptions
                        .iter()
                        .filter(|s| state.in_tenant(s))
                        .map(|s| (s.name.clone(), "subscription".into())),
                );
                prefixed(word, options.into_iter())
            } else if !matches!(cmd.as_str(), "rg" | "res" | "type" | "tag") {
                vec![]
            } else if let Some(values) = strip_key(word, "type=") {
                if cmd == "rg" {
                    vec![]
                } else {
                    with_key("type=", values, types(state))
                }
            } else if let Some(values) = strip_key(word, "loc=") {
                with_key("loc=", values, locations(state))
            } else if let Some(values) = strip_key(word, "tag=") {
                tag_values(values, state)
            } else {
                // Scope or a filter key.
                let mut options: Vec<(String, String)> = Vec::new();
                let has_scope = earlier[1..].iter().any(|t| !t.contains('='));
                if !has_scope {
                    options.push(("all".into(), "every subscription".into()));
                    options.extend(
                        state
                            .subscriptions
                            .iter()
                            .filter(|s| state.in_tenant(s))
                            .map(|s| (s.name.clone(), "subscription".into())),
                    );
                }
                if cmd != "rg" {
                    options.push(("type=".into(), "by type or alias, e.g. type=vm".into()));
                }
                options.push(("loc=".into(), "by location".into()));
                options.push(("tag=".into(), "by tag: tag=key or tag=key=value".into()));
                prefixed(word, options.into_iter())
            }
        }
    };
    Completion { start, items }
}

/// `input` with the word starting at `start` replaced by `text`, followed by
/// a space unless more is expected (a `key=` to fill in).
/// What Enter does with the suggestions: the command line with the
/// highlighted one in place of the word being typed, and whether that still
/// needs a value (`type=`, `env=`) rather than running. `None`: run the
/// line as typed (no suggestions; nothing typed and none picked with ↑/↓;
/// or the word already is one of them).
pub fn enter(
    input: &str,
    selection: usize,
    picked: bool,
    state: &AppState,
) -> Option<(String, bool)> {
    let completion = complete(input, state);
    let word = &input[completion.start.min(input.len())..];
    if completion.items.is_empty() {
        return None;
    }
    if !picked
        && (word.is_empty()
            || completion
                .items
                .iter()
                .any(|s| s.text.eq_ignore_ascii_case(word)))
    {
        return None;
    }
    let s = completion
        .items
        .get(selection)
        .or(completion.items.first())?;
    let needs_more = s.text.ends_with('=') || s.text.ends_with(',');
    Some((apply(input, completion.start, &s.text), needs_more))
}

pub fn apply(input: &str, start: usize, text: &str) -> String {
    let start = start.min(input.len());
    let space = if text.ends_with('=') || text.ends_with(',') {
        ""
    } else {
        " "
    };
    format!("{}{text}{space}", &input[..start])
}

/// The values after `key` (case-insensitive), e.g. "vm,dis" for "type=vm,dis".
fn strip_key<'a>(word: &'a str, key: &str) -> Option<&'a str> {
    (word.len() >= key.len() && word[..key.len()].eq_ignore_ascii_case(key))
        .then(|| &word[key.len()..])
}

/// Suggestions for the last comma-separated value after `key`, keeping the
/// earlier values: "type=vm,di" + "disk" → "type=vm,disk".
fn with_key(key: &str, values: &str, options: Vec<(String, String)>) -> Vec<Suggestion> {
    let (done, last) = match values.rfind(',') {
        Some(i) => (&values[..=i], &values[i + 1..]),
        None => ("", values),
    };
    prefixed(last, options.into_iter())
        .into_iter()
        .map(|s| Suggestion {
            text: format!("{key}{done}{}", s.text),
            detail: s.detail,
        })
        .collect()
}

/// Options whose text starts with `word` (ignoring case), then those that
/// contain it.
fn prefixed(word: &str, options: impl Iterator<Item = (String, String)>) -> Vec<Suggestion> {
    let needle = word.to_ascii_lowercase();
    let (mut starts, mut contains): (Vec<_>, Vec<_>) = (vec![], vec![]);
    for (text, detail) in options {
        let lower = text.to_ascii_lowercase();
        if lower.starts_with(&needle) {
            starts.push(Suggestion { text, detail });
        } else if lower.contains(&needle) {
            contains.push(Suggestion { text, detail });
        }
    }
    starts.extend(contains);
    starts.truncate(MAX_SUGGESTIONS);
    starts
}

/// Aliases of the loaded resource types, most common first; the known
/// aliases if nothing is loaded.
fn types(state: &AppState) -> Vec<(String, String)> {
    // From all loaded resources, not the current list's filters.
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for r in &state.resources {
        *counts
            .entry(r.resource_type.to_ascii_lowercase())
            .or_default() += 1;
    }
    let mut summary: Vec<(String, usize)> = counts.into_iter().collect();
    summary.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    if summary.is_empty() {
        return known_aliases()
            .into_iter()
            .map(|(alias, t)| (alias.to_string(), t.to_string()))
            .collect();
    }
    summary
        .into_iter()
        .map(|(t, n)| (short_label(&t), format!("{t} · {n}")))
        .collect()
}

/// Locations of the loaded resources and resource groups, most used first.
fn locations(state: &AppState) -> Vec<(String, String)> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let all = state
        .resources
        .iter()
        .map(|r| &r.location)
        .chain(state.resource_groups.iter().map(|g| &g.location));
    for loc in all.filter(|l| !l.is_empty()) {
        *counts.entry(loc.to_ascii_lowercase()).or_default() += 1;
    }
    let mut rows: Vec<(String, usize)> = counts.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    rows.into_iter().map(|(l, n)| (l, n.to_string())).collect()
}

/// After `tag=`: keys (completing to `tag=key=`), or after `tag=key=` (or
/// `tag=key:`) that key's values.
fn tag_values(values: &str, state: &AppState) -> Vec<Suggestion> {
    // (key as first seen, value, count) over all loaded resources.
    let mut counts: BTreeMap<(String, String), (String, usize)> = BTreeMap::new();
    for r in &state.resources {
        for (k, v) in &r.tags {
            counts
                .entry((k.to_lowercase(), v.clone()))
                .or_insert_with(|| (k.clone(), 0))
                .1 += 1;
        }
    }
    let mut summary: Vec<crate::state::TagCount> = counts
        .into_iter()
        .map(|((_, value), (key, count))| crate::state::TagCount { key, value, count })
        .collect();
    summary.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
    match values.split_once(['=', ':']) {
        Some((key, value)) => {
            let sep = &values[key.len()..key.len() + 1];
            let options: Vec<(String, String)> = summary
                .iter()
                .filter(|t| t.key.eq_ignore_ascii_case(key))
                .map(|t| (t.value.clone(), t.count.to_string()))
                .collect();
            prefixed(value, options.into_iter())
                .into_iter()
                .map(|s| Suggestion {
                    text: format!("tag={key}{sep}{}", s.text),
                    detail: s.detail,
                })
                .collect()
        }
        None => {
            let mut keys: BTreeMap<String, (String, usize)> = BTreeMap::new();
            for t in &summary {
                keys.entry(t.key.to_lowercase())
                    .or_insert_with(|| (t.key.clone(), 0))
                    .1 += t.count;
            }
            let mut rows: Vec<(String, usize)> = keys.into_values().collect();
            rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            prefixed(
                values,
                rows.into_iter()
                    .map(|(k, n)| (format!("{k}="), n.to_string())),
            )
            .into_iter()
            .map(|s| Suggestion {
                text: format!("tag={}", s.text),
                detail: s.detail,
            })
            .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Resource, Subscription};
    use std::collections::HashMap;

    fn state() -> AppState {
        let mut state = AppState::new();
        state.subscriptions = vec![Subscription {
            id: "s1".into(),
            name: "prod-connectivity".into(),
            state: "Enabled".into(),
            tenant_id: "t".into(),
        }];
        let res = |name: &str, t: &str, loc: &str, env: &str| Resource {
            id: format!("/x/{name}"),
            name: name.into(),
            resource_type: t.into(),
            location: loc.into(),
            resource_group: "rg".into(),
            subscription_id: "s1".into(),
            tags: HashMap::from([("env".to_string(), env.to_string())]),
        };
        state.resources = vec![
            res(
                "vm-1",
                "Microsoft.Compute/virtualMachines",
                "westeurope",
                "prod",
            ),
            res(
                "vm-2",
                "Microsoft.Compute/virtualMachines",
                "northeurope",
                "dev",
            ),
            res("kv", "Microsoft.KeyVault/vaults", "westeurope", "prod"),
        ];
        state
    }

    fn texts(input: &str) -> Vec<String> {
        complete(input, &state())
            .items
            .into_iter()
            .map(|s| s.text)
            .collect()
    }

    #[test]
    fn command_names_first() {
        assert_eq!(texts("r"), ["rg", "res", "ra", "role", "group"]);
        assert_eq!(
            texts("t"),
            ["type", "tag", "theme", "exemption", "ctx"],
            "starting with t, then containing t"
        );
        assert_eq!(texts("").len(), 16);
        assert_eq!(texts("comp"), ["compliance"]);
        assert!(texts("xyz").is_empty());
    }

    #[test]
    fn scope_and_filter_keys_after_a_command() {
        assert_eq!(
            texts("res "),
            ["all", "prod-connectivity", "type=", "loc=", "tag="]
        );
        assert_eq!(
            texts("rg "),
            ["all", "prod-connectivity", "loc=", "tag="],
            "no type= for :rg"
        );
        assert_eq!(texts("res pro"), ["prod-connectivity"]);
        assert_eq!(
            texts("res all "),
            ["type=", "loc=", "tag="],
            "scope already given"
        );
        assert!(texts("sub ").is_empty());
    }

    #[test]
    fn values_come_from_what_is_loaded() {
        assert_eq!(
            texts("res type="),
            ["type=vm", "type=key-vault"],
            "most common first"
        );
        assert_eq!(texts("res type=vm,k"), ["type=vm,key-vault"]);
        assert_eq!(texts("res loc="), ["loc=westeurope", "loc=northeurope"]);
        assert_eq!(texts("res all tag="), ["tag=env="]);
        assert_eq!(texts("res tag=env="), ["tag=env=prod", "tag=env=dev"]);
        assert_eq!(
            texts("res tag=env:d"),
            ["tag=env:dev", "tag=env:prod"],
            "starting with d first, then containing d"
        );
    }

    #[test]
    fn suggestions_ignore_the_current_lists_filters() {
        let mut state = state();
        state.resource_query = crate::query::ResourceQuery::parse(["type=vm"]).unwrap();
        state.filter = "zzz".into();
        let items = complete("res type=", &state).items;
        assert_eq!(items.len(), 2, "{items:?}");
    }

    #[test]
    fn known_aliases_when_nothing_is_loaded() {
        let empty = AppState::new();
        let items = complete("res type=vn", &empty).items;
        assert!(items.iter().any(|s| s.text == "type=vnet"), "{items:?}");
    }

    #[test]
    fn applying_replaces_the_word() {
        let c = complete("res all ty", &state());
        assert_eq!(
            apply("res all ty", c.start, &c.items[0].text),
            "res all type="
        );
        assert_eq!(apply("r", 0, "rg"), "rg ");
        assert_eq!(apply("res tag=", 4, "tag=env="), "res tag=env=");
        assert_eq!(
            apply("res tag=env=", 4, "tag=env=prod"),
            "res tag=env=prod "
        );
    }
}
