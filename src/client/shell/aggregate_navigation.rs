//! Endpoint-qualified rows shared by aggregate navigation surfaces.

use super::*;
use crate::protocol::ClientShellAgent;

#[derive(Clone, Copy)]
pub(super) struct CachedEndpointSnapshot<'a> {
    pub(super) endpoint_index: usize,
    pub(super) endpoint_id: &'a ClientEndpointId,
    pub(super) label: &'a str,
    pub(super) status: ClientEndpointStatus,
    pub(super) snapshot: &'a ClientShellSnapshot,
    pub(super) agent_recency: &'a HashMap<String, u64>,
    pub(super) agent_presentation: &'a super::endpoint_agent_state::EndpointAgentPresentation,
}

impl CachedEndpointSnapshot<'_> {
    pub(super) fn stale(self) -> bool {
        self.status != ClientEndpointStatus::Online
    }
}

pub(super) fn cached_endpoint_snapshots(
    endpoints: &[ClientShellEndpoint],
) -> impl Iterator<Item = CachedEndpointSnapshot<'_>> {
    endpoints
        .iter()
        .enumerate()
        .filter_map(|(endpoint_index, endpoint)| {
            endpoint
                .snapshot
                .as_deref()
                .map(|snapshot| CachedEndpointSnapshot {
                    endpoint_index,
                    endpoint_id: &endpoint.endpoint_id,
                    label: &endpoint.label,
                    status: endpoint.status,
                    snapshot,
                    agent_recency: &endpoint.agent_recency,
                    agent_presentation: &endpoint.agent_presentation,
                })
        })
}

pub(super) struct AggregateAgentRow<'a> {
    pub(super) endpoint: CachedEndpointSnapshot<'a>,
    pub(super) agent: &'a ClientShellAgent,
    pub(super) recency: u64,
}

pub(super) struct AggregateAgentTarget {
    pub(super) endpoint_id: ClientEndpointId,
    pub(super) pane_id: String,
}

pub(super) fn aggregate_agent_rows<'a>(
    endpoints: &'a [ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    order: super::agent_sidebar::AgentListOrder,
) -> Vec<AggregateAgentRow<'a>> {
    let mut rows = unscoped_aggregate_agent_rows(endpoints, active_endpoint_id, order.sort);
    if order.follows_workspace {
        rows.retain(|row| {
            row.endpoint.endpoint_id == active_endpoint_id
                && Some(row.agent.workspace_id.as_str())
                    == row.endpoint.snapshot.focused_workspace_id.as_deref()
        });
    }
    rows
}

fn unscoped_aggregate_agent_rows<'a>(
    endpoints: &'a [ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    sort: crate::config::AgentPanelSortConfig,
) -> Vec<AggregateAgentRow<'a>> {
    let active_index = endpoints
        .iter()
        .position(|endpoint| &endpoint.endpoint_id == active_endpoint_id);
    let active_view = active_index.and_then(|index| {
        let endpoint = &endpoints[index];
        if !endpoint.agent_view_projection_supported {
            return None;
        }
        match ClientShellState::endpoint_agent_view(endpoint) {
            Some(Ok(view)) => Some(Ok(view.as_ref())),
            Some(Err(())) => Some(Err(())),
            None if endpoint
                .snapshot
                .as_deref()
                .is_some_and(|snapshot| snapshot.agent_view_label.is_none()) =>
            {
                Some(Ok(None))
            }
            None => Some(Err(())),
        }
    });

    if let Some(Ok(view)) = active_view {
        let tree_order = view.is_none() && sort == crate::config::AgentPanelSortConfig::Spaces;
        let mut rows = cached_endpoint_snapshots(endpoints)
            .flat_map(|endpoint| {
                let mut agents = endpoint.snapshot.agents.iter().collect::<Vec<_>>();
                if tree_order {
                    super::agent_sidebar::sort_agents_by_tab_tree(
                        &mut agents,
                        &endpoint.snapshot.tabs,
                    );
                }
                agents.into_iter().map(move |agent| AggregateAgentRow {
                    recency: endpoint
                        .agent_recency
                        .get(&agent.pane_id)
                        .copied()
                        .unwrap_or_default(),
                    endpoint,
                    agent,
                })
            })
            .collect::<Vec<_>>();
        if let Some(view) = view {
            let context = active_index
                .and_then(|index| endpoints[index].snapshot.as_deref())
                .map(|snapshot| crate::agent_view_eval::AgentViewContext {
                    scope: active_index.unwrap_or_default(),
                    workspace_id: snapshot.focused_workspace_id.clone(),
                    tab_id: snapshot.focused_tab_id.clone(),
                });
            if let (Some(context), Some(filter)) = (context.as_ref(), view.filter.as_ref()) {
                rows.retain(|row| {
                    crate::agent_view_eval::matches_filter(
                        context,
                        &ClientAgentViewEntry::new(row),
                        filter,
                    )
                });
            }
            if !view.sort.is_empty() {
                rows.sort_by(|left, right| {
                    crate::agent_view_eval::compare_entries(
                        &ClientAgentViewEntry::new(left),
                        &ClientAgentViewEntry::new(right),
                        &view.sort,
                    )
                });
                return rows;
            }
        }
        sort_aggregate_rows(&mut rows, sort);
        return rows;
    }

    let mut rows = cached_endpoint_snapshots(endpoints)
        .flat_map(|endpoint| {
            super::agent_sidebar::unscoped_agent_pane_ids(endpoint.snapshot, sort)
                .into_iter()
                .filter_map(move |pane_id| {
                    let agent = endpoint
                        .snapshot
                        .agents
                        .iter()
                        .find(|agent| agent.pane_id == pane_id)?;
                    Some(AggregateAgentRow {
                        recency: endpoint
                            .agent_recency
                            .get(&pane_id)
                            .copied()
                            .unwrap_or_default(),
                        endpoint,
                        agent,
                    })
                })
        })
        .collect::<Vec<_>>();
    sort_aggregate_rows(&mut rows, sort);
    rows
}

fn sort_aggregate_rows(
    rows: &mut [AggregateAgentRow<'_>],
    sort: crate::config::AgentPanelSortConfig,
) {
    if sort == crate::config::AgentPanelSortConfig::Priority {
        rows.sort_by_key(|row| {
            (
                row.endpoint.stale(),
                std::cmp::Reverse(status_priority(row.agent.agent_status)),
                std::cmp::Reverse(row.recency),
            )
        });
    }
}

struct ClientAgentViewEntry<'a> {
    endpoint_index: usize,
    snapshot: &'a ClientShellSnapshot,
    agent: &'a ClientShellAgent,
    seen: bool,
}

impl<'a> ClientAgentViewEntry<'a> {
    fn new(row: &AggregateAgentRow<'a>) -> Self {
        Self {
            endpoint_index: row.endpoint.endpoint_index,
            snapshot: row.endpoint.snapshot,
            agent: row.agent,
            seen: row.endpoint.agent_presentation.seen(row.agent),
        }
    }
}

impl crate::agent_view_eval::AgentViewEntry for ClientAgentViewEntry<'_> {
    fn scope(&self) -> usize {
        self.endpoint_index
    }

    fn status(&self) -> &'static str {
        status_text(self.agent.agent_status)
    }

    fn workspace_id(&self) -> Option<std::borrow::Cow<'_, str>> {
        Some(std::borrow::Cow::Borrowed(&self.agent.workspace_id))
    }

    fn tab_id(&self) -> Option<std::borrow::Cow<'_, str>> {
        Some(std::borrow::Cow::Borrowed(&self.agent.tab_id))
    }

    fn pane_id(&self) -> Option<std::borrow::Cow<'_, str>> {
        Some(std::borrow::Cow::Borrowed(&self.agent.pane_id))
    }

    fn agent(&self) -> Option<&str> {
        self.agent.agent.as_deref()
    }

    fn seen(&self) -> bool {
        self.seen
    }

    fn state_change_seq(&self) -> Option<u64> {
        Some(self.agent.state_change_seq)
    }

    fn token(&self, token: &str) -> Option<&str> {
        self.agent
            .tokens
            .iter()
            .find(|(name, _)| name == token)
            .map(|(_, value)| value.as_str())
    }

    fn workspace_order(&self) -> Option<u64> {
        self.snapshot
            .workspaces
            .iter()
            .position(|workspace| workspace.workspace_id == self.agent.workspace_id)
            .map(|index| index as u64)
    }

    fn tab_order(&self) -> Option<u64> {
        self.snapshot
            .tabs
            .iter()
            .find(|tab| tab.tab_id == self.agent.tab_id)
            .map(|tab| tab.number as u64)
    }

    fn pane_order(&self) -> Option<u64> {
        let suffix = self
            .agent
            .pane_id
            .strip_prefix(&self.agent.workspace_id)?
            .strip_prefix(":p")?;
        crate::workspace::decode_public_number(suffix).map(|number| number as u64)
    }

    fn attention(&self) -> u64 {
        u64::from(status_priority(self.agent.agent_status))
    }
}

fn is_agent_pane(agent: Option<&crate::protocol::ClientShellAgent>) -> bool {
    agent.is_some_and(|agent| agent.agent.is_some())
}

fn pane_label(
    pane: &crate::protocol::ClientShellPane,
    agent: Option<&crate::protocol::ClientShellAgent>,
    index: usize,
) -> String {
    pane.label
        .clone()
        .or_else(|| agent.and_then(|agent| agent.name.clone()))
        .or_else(|| agent.and_then(|agent| agent.title.clone()))
        .or_else(|| agent.and_then(|agent| agent.terminal_title_stripped.clone()))
        .or_else(|| agent.and_then(|agent| agent.display_agent.clone()))
        .unwrap_or_else(|| format!("pane {}", index + 1))
}

fn status_filter_matches(
    filter: Option<ClientNavigatorFilter>,
    status: crate::api::schema::AgentStatus,
) -> bool {
    match filter {
        Some(ClientNavigatorFilter::Blocked) => status == crate::api::schema::AgentStatus::Blocked,
        Some(ClientNavigatorFilter::Working) => status == crate::api::schema::AgentStatus::Working,
        Some(ClientNavigatorFilter::Idle) => status == crate::api::schema::AgentStatus::Idle,
        Some(ClientNavigatorFilter::Done) => status == crate::api::schema::AgentStatus::Done,
        None => true,
    }
}

/// The status goto lists a pane with, or `None` when agents-only or the
/// status filter hides it. Every goto view counts and lists panes through this
/// one predicate so their pane counts agree.
fn visible_pane_status(
    agent: Option<&ClientShellAgent>,
    agents_only: bool,
    filter: Option<ClientNavigatorFilter>,
) -> Option<crate::api::schema::AgentStatus> {
    if agents_only && !is_agent_pane(agent) {
        return None;
    }
    let status = agent.map_or(crate::api::schema::AgentStatus::Unknown, |agent| {
        agent.agent_status
    });
    status_filter_matches(filter, status).then_some(status)
}

pub(super) fn online_agent_targets(
    endpoints: &[ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    order: super::agent_sidebar::AgentListOrder,
) -> Vec<AggregateAgentTarget> {
    aggregate_agent_rows(endpoints, active_endpoint_id, order)
        .into_iter()
        .filter(|row| !row.endpoint.stale())
        .map(|row| AggregateAgentTarget {
            endpoint_id: row.endpoint.endpoint_id.clone(),
            pane_id: row.agent.pane_id.clone(),
        })
        .collect()
}

pub(super) fn navigator_rows(
    endpoints: &[ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    navigator: &ClientNavigatorOverlay,
) -> Vec<ClientNavigatorRow> {
    let query = navigator.query.trim().to_lowercase();
    let filter = |status| status_filter_matches(navigator.filter, status);
    let text = |value: &str| crate::fuzzy::fuzzy_score(&query, value).is_some();
    if !query.is_empty() {
        return flat_query_rows(endpoints, active_endpoint_id, navigator, &query);
    }
    let filtering = navigator.filter.is_some() || !query.is_empty();
    let federated = endpoints.len() > 1;
    let depth_offset = u8::from(federated);
    let mut rows = Vec::new();

    for endpoint in endpoints {
        let stale = endpoint.status != ClientEndpointStatus::Online;
        let endpoint_query_matches = !query.is_empty() && text(&endpoint.label);
        let mut endpoint_rows = Vec::new();
        if let Some(snapshot) = endpoint.snapshot.as_deref() {
            let agents = snapshot
                .agents
                .iter()
                .map(|agent| (agent.pane_id.as_str(), agent))
                .collect::<HashMap<_, _>>();
            // Build endpoint-local indexes once. Walk each bucket in snapshot
            // order so interleaved input and overlapping IDs on other endpoints
            // retain their existing navigation order and targets.
            let mut tabs_by_workspace = HashMap::new();
            for tab in &snapshot.tabs {
                tabs_by_workspace
                    .entry(tab.workspace_id.as_str())
                    .or_insert_with(Vec::new)
                    .push(tab);
            }
            let mut panes_by_tab = HashMap::new();
            for pane in &snapshot.panes {
                panes_by_tab
                    .entry(pane.tab_id.as_str())
                    .or_insert_with(Vec::new)
                    .push(pane);
            }
            for workspace in &snapshot.workspaces {
                let workspace_meta = workspace.branch.clone().unwrap_or_default();
                let mut children = Vec::new();
                let workspace_tabs = tabs_by_workspace
                    .get(workspace.workspace_id.as_str())
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                for tab in workspace_tabs {
                    let tab_panes = panes_by_tab
                        .get(tab.tab_id.as_str())
                        .map(Vec::as_slice)
                        .unwrap_or_default();
                    let mut panes = Vec::new();
                    for (index, pane) in tab_panes.iter().enumerate() {
                        let agent = agents.get(pane.pane_id.as_str()).copied();
                        let Some(status) =
                            visible_pane_status(agent, navigator.agents_only, navigator.filter)
                        else {
                            continue;
                        };
                        let label = pane_label(pane, agent, index);
                        let meta = pane
                            .foreground_cwd
                            .clone()
                            .or_else(|| pane.cwd.clone())
                            .unwrap_or_default();
                        panes.push(ClientNavigatorRow {
                            depth: 2 + depth_offset,
                            title: format!("{} › {}", tab.label, label),
                            label,
                            meta: String::new(),
                            cwd: meta,
                            label_hits: Vec::new(),
                            meta_hits: Vec::new(),
                            status: Some(status),
                            stale,
                            current: endpoint.endpoint_id == *active_endpoint_id
                                && snapshot.focused_pane_id.as_deref() == Some(&pane.pane_id),
                            target: ClientNavigatorTarget::Pane {
                                endpoint_id: endpoint.endpoint_id.clone(),
                                pane_id: pane.pane_id.clone(),
                            },
                        });
                    }
                    let keep_tab = if navigator.agents_only {
                        !panes.is_empty()
                    } else {
                        !filtering
                            || filter(tab.agent_status)
                                && (endpoint_query_matches || text(&tab.label))
                            || !panes.is_empty()
                    };
                    if keep_tab {
                        if panes.len() == 1 {
                            let mut row = panes.remove(0);
                            row.depth = 1 + depth_offset;
                            row.label = tab.label.clone();
                            row.title = tab.label.clone();
                            children.push(row);
                        } else {
                            children.push(ClientNavigatorRow {
                                depth: 1 + depth_offset,
                                label: tab.label.clone(),
                                title: String::new(),
                                meta: if panes.is_empty() {
                                    String::new()
                                } else {
                                    format!("{} panes", panes.len())
                                },
                                cwd: String::new(),
                                label_hits: Vec::new(),
                                meta_hits: Vec::new(),
                                status: None,
                                stale,
                                current: false,
                                target: ClientNavigatorTarget::Tab {
                                    endpoint_id: endpoint.endpoint_id.clone(),
                                    tab_id: tab.tab_id.clone(),
                                },
                            });
                            children.extend(panes);
                        }
                    }
                }
                let workspace_matches = filter(workspace.agent_status)
                    && (endpoint_query_matches || text(&workspace.label) || text(&workspace_meta));
                let keep_workspace = if navigator.agents_only {
                    !children.is_empty()
                } else {
                    !filtering || workspace_matches || !children.is_empty()
                };
                if keep_workspace {
                    let key = (endpoint.endpoint_id.clone(), workspace.workspace_id.clone());
                    endpoint_rows.push(ClientNavigatorRow {
                        depth: depth_offset,
                        label: workspace.label.clone(),
                        title: String::new(),
                        meta: workspace_meta,
                        cwd: String::new(),
                        label_hits: Vec::new(),
                        meta_hits: Vec::new(),
                        status: None,
                        stale,
                        current: false,
                        target: ClientNavigatorTarget::Workspace {
                            endpoint_id: endpoint.endpoint_id.clone(),
                            workspace_id: workspace.workspace_id.clone(),
                        },
                    });
                    if navigator.expanded_workspaces.contains(&key) || filtering {
                        endpoint_rows.extend(children);
                    }
                }
            }
        }
        let keep_endpoint = if navigator.agents_only {
            !endpoint_rows.is_empty() || endpoint.endpoint_id == *active_endpoint_id
        } else {
            !filtering || endpoint_query_matches || !endpoint_rows.is_empty()
        };
        if keep_endpoint {
            if federated {
                rows.push(ClientNavigatorRow {
                    depth: 0,
                    label: endpoint.label.to_owned(),
                    title: String::new(),
                    meta: String::new(),
                    cwd: String::new(),
                    label_hits: Vec::new(),
                    meta_hits: Vec::new(),
                    status: None,
                    stale,
                    current: false,
                    target: ClientNavigatorTarget::Machine {
                        endpoint_id: endpoint.endpoint_id.clone(),
                    },
                });
            }
            rows.extend(endpoint_rows);
        }
    }
    rows
}

/// A text query drops the tree so every matching pane becomes one row. The
/// row is labelled with its tab name, plus the pane name when the tab has more
/// than one visible pane, and `meta` carries the workspace (and machine when
/// federated). Each whitespace-separated term must fuzzy-match some field of
/// the pane, and each term may match a different one. Tab, pane and agent
/// names are primary fields; workspace, branch, machine, cwd and pane id are
/// secondary. A row with any primary match ranks above rows with only
/// secondary matches, and within each group the summed term scores decide.
/// `label_hits` and `meta_hits` record where the terms land in the drawn text.
fn flat_query_rows(
    endpoints: &[ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    navigator: &ClientNavigatorOverlay,
    query: &str,
) -> Vec<ClientNavigatorRow> {
    let federated = endpoints.len() > 1;
    let mut scored: Vec<(bool, i32, ClientNavigatorRow)> = Vec::new();
    for endpoint in endpoints {
        let Some(snapshot) = endpoint.snapshot.as_deref() else {
            continue;
        };
        let stale = endpoint.status != ClientEndpointStatus::Online;
        let agents = snapshot
            .agents
            .iter()
            .map(|agent| (agent.pane_id.as_str(), agent))
            .collect::<HashMap<_, _>>();
        let mut tabs_by_workspace = HashMap::new();
        for tab in &snapshot.tabs {
            tabs_by_workspace
                .entry(tab.workspace_id.as_str())
                .or_insert_with(Vec::new)
                .push(tab);
        }
        let mut panes_by_tab = HashMap::new();
        for pane in &snapshot.panes {
            panes_by_tab
                .entry(pane.tab_id.as_str())
                .or_insert_with(Vec::new)
                .push(pane);
        }
        for workspace in &snapshot.workspaces {
            let workspace_tabs = tabs_by_workspace
                .get(workspace.workspace_id.as_str())
                .map(Vec::as_slice)
                .unwrap_or_default();
            for tab in workspace_tabs {
                let tab_panes = panes_by_tab
                    .get(tab.tab_id.as_str())
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let pane_status = |pane: &crate::protocol::ClientShellPane| {
                    let agent = agents.get(pane.pane_id.as_str()).copied();
                    visible_pane_status(agent, navigator.agents_only, navigator.filter)
                        .map(|status| (agent, status))
                };
                let visible_panes = tab_panes
                    .iter()
                    .filter(|pane| pane_status(pane).is_some())
                    .count();
                for (index, pane) in tab_panes.iter().enumerate() {
                    let Some((agent, status)) = pane_status(pane) else {
                        continue;
                    };
                    let pane_name = pane_label(pane, agent, index);
                    let label = if visible_panes > 1 {
                        format!("{} › {pane_name}", tab.label)
                    } else {
                        tab.label.clone()
                    };
                    let cwd = pane
                        .foreground_cwd
                        .clone()
                        .or_else(|| pane.cwd.clone())
                        .unwrap_or_default();
                    let meta = if federated {
                        format!("{} › {}", endpoint.label, workspace.label)
                    } else {
                        workspace.label.clone()
                    };
                    let agent_fields = match agent {
                        Some(agent) => [
                            agent.title.as_deref(),
                            agent.terminal_title_stripped.as_deref(),
                            agent.agent.as_deref(),
                            agent.display_agent.as_deref(),
                        ],
                        None => [None; 4],
                    };
                    let fields = [
                        (Some(tab.label.as_str()), true),
                        (Some(pane_name.as_str()), true),
                        (agent_fields[0], true),
                        (agent_fields[1], true),
                        (agent_fields[2], true),
                        (agent_fields[3], true),
                        (Some(workspace.label.as_str()), false),
                        (workspace.branch.as_deref(), false),
                        (federated.then_some(endpoint.label.as_str()), false),
                        (Some(cwd.as_str()), false),
                        (Some(pane.pane_id.as_str()), false),
                    ];
                    let label_is_tab = visible_panes <= 1;
                    let meta_is_workspace = !federated;
                    let mut score = 0;
                    let mut primary = false;
                    let mut label_hits = Vec::new();
                    let mut meta_hits = Vec::new();
                    let mut matched_all = true;
                    for term in query.split_whitespace() {
                        let mut best: Option<(i32, bool)> = None;
                        let mut tab_matched = false;
                        let mut workspace_matched = false;
                        for (index, (field, is_primary)) in fields.iter().enumerate() {
                            let Some(found) =
                                field.and_then(|field| crate::fuzzy::fuzzy_term_score(term, field))
                            else {
                                continue;
                            };
                            if best.is_none_or(|(top, _)| found > top) {
                                best = Some((found, *is_primary));
                            }
                            match index {
                                0 => tab_matched = true,
                                6 => workspace_matched = true,
                                _ => {}
                            }
                        }
                        let Some((term_score, term_primary)) = best else {
                            matched_all = false;
                            break;
                        };
                        score += term_score;
                        primary |= term_primary;
                        let label_match = if !label_is_tab || tab_matched {
                            crate::fuzzy::fuzzy_term_match(term, &label)
                        } else {
                            None
                        };
                        if let Some(found) = label_match {
                            label_hits.extend(found.positions);
                            continue;
                        }
                        let meta_match = if !meta_is_workspace || workspace_matched {
                            crate::fuzzy::fuzzy_term_match(term, &meta)
                        } else {
                            None
                        };
                        if let Some(found) = meta_match {
                            meta_hits.extend(found.positions);
                        }
                    }
                    if !matched_all {
                        continue;
                    }
                    label_hits.sort_unstable();
                    label_hits.dedup();
                    meta_hits.sort_unstable();
                    meta_hits.dedup();
                    scored.push((
                        primary,
                        score,
                        ClientNavigatorRow {
                            depth: 0,
                            title: label.clone(),
                            label,
                            meta,
                            cwd,
                            label_hits,
                            meta_hits,
                            status: Some(status),
                            stale,
                            current: endpoint.endpoint_id == *active_endpoint_id
                                && snapshot.focused_pane_id.as_deref() == Some(&pane.pane_id),
                            target: ClientNavigatorTarget::Pane {
                                endpoint_id: endpoint.endpoint_id.clone(),
                                pane_id: pane.pane_id.clone(),
                            },
                        },
                    ));
                }
            }
        }
    }
    scored.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then(right.1.cmp(&left.1))
            .then(left.2.stale.cmp(&right.2.stale))
    });
    scored.into_iter().map(|(_, _, row)| row).collect()
}

/// Panes passing the agents-only and status filters on every endpoint,
/// regardless of query or which workspaces are expanded.
pub(super) fn navigator_pane_total(
    endpoints: &[ClientShellEndpoint],
    navigator: &ClientNavigatorOverlay,
) -> usize {
    endpoints
        .iter()
        .filter_map(|endpoint| endpoint.snapshot.as_deref())
        .map(|snapshot| {
            let agents = snapshot
                .agents
                .iter()
                .map(|agent| (agent.pane_id.as_str(), agent))
                .collect::<HashMap<_, _>>();
            snapshot
                .panes
                .iter()
                .filter(|pane| {
                    let agent = agents.get(pane.pane_id.as_str()).copied();
                    visible_pane_status(agent, navigator.agents_only, navigator.filter).is_some()
                })
                .count()
        })
        .sum()
}

/// Display lines for the tree: `Some(row)` for a row, `None` for the blank gap
/// before a workspace or machine, except directly under a machine row.
pub(super) fn navigator_lines(rows: &[ClientNavigatorRow]) -> Vec<Option<usize>> {
    let mut lines = Vec::with_capacity(rows.len() + rows.len() / 2);
    for (index, row) in rows.iter().enumerate() {
        let starts_section = matches!(
            row.target,
            ClientNavigatorTarget::Workspace { .. } | ClientNavigatorTarget::Machine { .. }
        );
        let workspace_under_machine = index > 0
            && matches!(row.target, ClientNavigatorTarget::Workspace { .. })
            && matches!(
                rows[index - 1].target,
                ClientNavigatorTarget::Machine { .. }
            );
        if index > 0 && starts_section && !workspace_under_machine {
            lines.push(None);
        }
        lines.push(Some(index));
    }
    lines
}

pub(super) fn navigator_selected_index(
    rows: &[ClientNavigatorRow],
    navigator: &ClientNavigatorOverlay,
) -> Option<usize> {
    match navigator.selected.as_ref() {
        Some(target) => rows.iter().position(|row| row.target == *target),
        None => (!rows.is_empty()).then_some(0),
    }
}

pub(super) fn selected_navigator_target(
    rows: &[ClientNavigatorRow],
    navigator: &ClientNavigatorOverlay,
) -> Option<ClientNavigatorTarget> {
    navigator_selected_index(rows, navigator).map(|index| rows[index].target.clone())
}
