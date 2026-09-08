use crate::domain::{ProxySnapshot, RunMode};
use std::collections::HashSet;

use crate::appearance::metrics;

pub const GROUP_HEIGHT: f32 = metrics::PROXY_ITEM + metrics::ITEM_GAP;
pub const NODES_HEIGHT: f32 = metrics::PROXY_ITEM + metrics::ITEM_GAP;

#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Group(usize),
    Nodes { group: usize, members: Vec<usize> },
}
impl Row {
    pub fn height(&self) -> f32 {
        match self {
            Self::Group(_) => GROUP_HEIGHT,
            Self::Nodes { .. } => NODES_HEIGHT,
        }
    }
}

pub fn rows(
    snapshot: &ProxySnapshot,
    mode: Option<RunMode>,
    expanded: &HashSet<String>,
    query: &str,
    filter_collapsed: &HashSet<String>,
    columns: usize,
) -> Vec<Row> {
    let query = query.trim().to_lowercase();
    let mut rows = Vec::new();
    for (index, group) in snapshot.groups.iter().enumerate() {
        let global = mode == Some(RunMode::Global);
        if mode == Some(RunMode::Direct) || (global != (group.name == "GLOBAL")) {
            continue;
        }
        if !global && snapshot.proxies.get(&group.name).is_some_and(|p| p.hidden) {
            continue;
        }
        if !global && query.is_empty() && !expanded.contains(&group.name) {
            rows.push(Row::Group(index));
            continue;
        }
        let group_matches = !global && group.name.to_lowercase().contains(&query);
        let members: Vec<_> = group
            .members
            .iter()
            .enumerate()
            .filter_map(|(ix, name)| {
                let matches = query.is_empty()
                    || group_matches
                    || name.to_lowercase().contains(&query)
                    || snapshot
                        .proxies
                        .get(name)
                        .is_some_and(|p| p.kind.to_lowercase().contains(&query));
                matches.then_some(ix)
            })
            .collect();
        if !query.is_empty() && members.is_empty() && !group_matches {
            continue;
        }
        if !global {
            rows.push(Row::Group(index));
        }
        if global
            || (if query.is_empty() {
                expanded.contains(&group.name)
            } else {
                !filter_collapsed.contains(&group.name)
            })
        {
            rows.extend(members.chunks(columns.max(1)).map(|chunk| Row::Nodes {
                group: index,
                members: chunk.to_vec(),
            }));
        }
    }
    rows
}

pub fn selected_row(snapshot: &ProxySnapshot, rows: &[Row], group: usize) -> Option<usize> {
    let selected = snapshot.groups.get(group)?.selected.as_ref()?;
    rows.iter().position(|row| match row {
        Row::Nodes {
            group: index,
            members,
        } if *index == group => members
            .iter()
            .any(|&m| &snapshot.groups[group].members[m] == selected),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ProxyGroup;
    fn snapshot() -> ProxySnapshot {
        ProxySnapshot {
            groups: vec![
                ProxyGroup {
                    name: "Route".into(),
                    kind: "Selector".into(),
                    selected: Some("Japan".into()),
                    members: vec!["Hong Kong".into(), "Japan".into()],
                },
                ProxyGroup {
                    name: "GLOBAL".into(),
                    kind: "Selector".into(),
                    selected: Some("Japan".into()),
                    members: vec![
                        "DIRECT".into(),
                        "Route".into(),
                        "Hong Kong".into(),
                        "Japan".into(),
                    ],
                },
            ],
            ..Default::default()
        }
    }
    #[test]
    fn rules_start_collapsed_global_is_flat_and_direct_is_empty() {
        let s = snapshot();
        let closed = HashSet::new();
        assert_eq!(
            rows(&s, Some(RunMode::Rule), &closed, "", &HashSet::new(), 2),
            [Row::Group(0)]
        );
        let global = rows(&s, Some(RunMode::Global), &closed, "", &HashSet::new(), 2);
        assert_eq!(global.len(), 2);
        assert!(
            global
                .iter()
                .all(|r| matches!(r, Row::Nodes { group: 1, .. }))
        );
        assert!(rows(&s, Some(RunMode::Direct), &closed, "", &HashSet::new(), 2).is_empty());
    }
    #[test]
    fn search_reveals_matches_without_changing_expansion_and_locate_uses_visible_rows() {
        let s = snapshot();
        let mut expanded = HashSet::new();
        let filtered = rows(
            &s,
            Some(RunMode::Rule),
            &expanded,
            "JAPAN",
            &HashSet::new(),
            2,
        );
        assert_eq!(
            filtered,
            [
                Row::Group(0),
                Row::Nodes {
                    group: 0,
                    members: vec![1]
                }
            ]
        );
        assert_eq!(selected_row(&s, &filtered, 0), Some(1));
        assert_eq!(
            rows(&s, Some(RunMode::Rule), &expanded, "", &HashSet::new(), 2),
            [Row::Group(0)]
        );
        expanded.insert("Route".into());
        assert_eq!(
            rows(&s, Some(RunMode::Rule), &expanded, "", &HashSet::new(), 1).len(),
            3
        );
        assert!(
            rows(
                &s,
                Some(RunMode::Rule),
                &expanded,
                "missing",
                &HashSet::new(),
                2
            )
            .is_empty()
        );
    }
}
