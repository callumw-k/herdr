//! BSP tree layout for tiling panes within a workspace.

use std::cmp::Reverse;

use ratatui::{
    layout::{Direction, Rect},
    widgets::Borders,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct PaneId(u32);

/// Global atomic counter for unique PaneId generation across all workspaces.
static NEXT_PANE_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

impl PaneId {
    /// Allocate a globally unique PaneId.
    pub fn alloc() -> Self {
        Self(NEXT_PANE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }

    pub fn raw(self) -> u32 {
        self.0
    }

    /// Reconstruct from a saved u32 (persistence only).
    pub fn from_raw(id: u32) -> Self {
        Self(id)
    }
}

/// Snapshot of a pane's position and focus state after layout.
#[derive(Clone)]
pub struct PaneInfo {
    pub id: PaneId,
    /// Outer rect (including borders if present).
    pub rect: Rect,
    /// Inner rect (content area, excluding borders). Used for selection.
    pub inner_rect: Rect,
    /// Visible scrollbar lane, when scrollback is present. `inner_rect` may still
    /// exclude a stable hidden gutter when this is `None`.
    pub scrollbar_rect: Option<Rect>,
    /// Borders drawn around this pane after UI chrome is applied.
    pub borders: Borders,
    pub is_focused: bool,
}

/// Info about a split boundary, used for mouse drag resize.
#[derive(Clone)]
pub struct SplitBorder {
    /// Position of the divider line (x for horizontal split, y for vertical).
    pub pos: u16,
    /// Direction of the split that created this border.
    pub direction: Direction,
    /// Ratio assigned to the first child of this split.
    pub ratio: f32,
    /// Total area of the split node.
    pub area: Rect,
    /// Path from root to this split node (false=first, true=second).
    pub path: Vec<bool>,
}

/// Cardinal direction for pane navigation.
#[derive(Debug, Clone, Copy)]
pub enum NavDirection {
    Left,
    Right,
    Up,
    Down,
}

/// A named pane arrangement. Cycling regenerates the tree from the tab's
/// ordered pane list rather than mutating the existing shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Arrangement {
    /// Even columns.
    Vertical,
    /// Even rows.
    Horizontal,
    /// Balanced rectangular grid.
    Grid,
    /// One stack holding every pane.
    #[default]
    Stacked,
}

impl Arrangement {
    #[cfg(test)]
    pub const ALL: [Arrangement; 4] = [
        Arrangement::Vertical,
        Arrangement::Horizontal,
        Arrangement::Grid,
        Arrangement::Stacked,
    ];

    pub fn next(self) -> Self {
        match self {
            Arrangement::Vertical => Arrangement::Horizontal,
            Arrangement::Horizontal => Arrangement::Grid,
            Arrangement::Grid => Arrangement::Stacked,
            Arrangement::Stacked => Arrangement::Vertical,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Arrangement::Vertical => Arrangement::Stacked,
            Arrangement::Horizontal => Arrangement::Vertical,
            Arrangement::Grid => Arrangement::Horizontal,
            Arrangement::Stacked => Arrangement::Grid,
        }
    }
}

/// Build a layout tree for `panes` under `kind`. Pure: no runtime, no PTY.
/// `area` is used only by `Grid` to pick a column count. Returns `None` for an
/// empty pane list, which callers treat as "keep the existing tree".
pub fn arrange(kind: Arrangement, panes: &[PaneId], focus: PaneId, area: Rect) -> Option<Node> {
    if panes.is_empty() {
        return None;
    }
    match kind {
        Arrangement::Vertical => even_chain(panes, Direction::Horizontal),
        Arrangement::Horizontal => even_chain(panes, Direction::Vertical),
        Arrangement::Grid => grid_node(panes, area),
        Arrangement::Stacked => Some(Node::Stack {
            panes: panes.to_vec(),
            active: panes.iter().position(|id| *id == focus).unwrap_or(0),
        }),
    }
}

/// A right-leaning chain of splits with ratios 1/n, 1/(n-1), ... so every
/// child ends up the same size.
fn even_chain(panes: &[PaneId], direction: Direction) -> Option<Node> {
    match panes {
        [] => None,
        [only] => Some(Node::Pane(*only)),
        [head, tail @ ..] => Some(Node::Split {
            direction,
            ratio: 1.0 / panes.len() as f32,
            first: Box::new(Node::Pane(*head)),
            second: Box::new(even_chain(tail, direction)?),
        }),
    }
}

/// Terminal cells are roughly twice as tall as they are wide, so a visually
/// square cell wants twice as many columns as rows.
fn grid_columns(count: usize, area: Rect) -> usize {
    let count = count.max(1);
    (1..=count)
        .min_by_key(|cols| {
            let rows = count.div_ceil(*cols);
            let cell_width = (area.width as f32 / *cols as f32).max(1.0);
            let cell_height = (area.height as f32 / rows as f32).max(1.0);
            // Score how far the cell is from square as a ratio, not as a
            // difference in cells: an absolute error grows with the terminal, so
            // on a wide screen it drowns the empty-cell penalty below and a
            // ragged grid wins.
            let aspect_error = (cell_width / (cell_height * 2.0)).ln().abs();
            // Squareness alone picks 3 columns for 4 panes, which leaves a
            // ragged 2/1/1 grid. Penalise the empty cells so a balanced grid
            // wins unless a ragged one is much squarer.
            let empty_cells = (rows * cols - count) as f32;
            // A wide enough area makes one row of full-height columns the
            // squarest option, which reads as a row of strips rather than a
            // grid. Pull the shape back toward equal rows and columns, weakly
            // enough that a narrow area still stacks.
            let imbalance = (*cols as f32 / rows as f32).ln().abs();
            ((aspect_error + empty_cells * 0.5 + imbalance * 0.5) * 1000.0) as i32
        })
        .unwrap_or(1)
}

fn grid_node(panes: &[PaneId], area: Rect) -> Option<Node> {
    if panes.is_empty() {
        return None;
    }
    let cols = grid_columns(panes.len(), area);
    let base = panes.len() / cols;
    let extra = panes.len() % cols;

    let mut columns: Vec<&[PaneId]> = Vec::with_capacity(cols);
    let mut rest = panes;
    for index in 0..cols {
        let take = base + usize::from(index < extra);
        let (head, tail) = rest.split_at(take);
        columns.push(head);
        rest = tail;
    }
    column_chain(&columns)
}

fn column_chain(columns: &[&[PaneId]]) -> Option<Node> {
    match columns {
        [] => None,
        [only] => even_chain(only, Direction::Vertical),
        [head, tail @ ..] => Some(Node::Split {
            direction: Direction::Horizontal,
            ratio: 1.0 / columns.len() as f32,
            first: Box::new(even_chain(head, Direction::Vertical)?),
            second: Box::new(column_chain(tail)?),
        }),
    }
}

/// The active pane in a stack keeps at least this many rows when the stack is
/// tall enough to afford it.
const MIN_ACTIVE_STACK_HEIGHT: u16 = 3;

/// One rect per stack member, index-aligned with the stack's pane list. Every
/// inactive pane gets a single collapsed bar and the active pane takes the
/// remainder. When the area is too short for that, later panes collapse to
/// zero height and the renderer folds them into a summary row.
fn stack_rects(area: Rect, count: usize, active: usize) -> Vec<Rect> {
    let mut rects = vec![Rect::new(area.x, area.y, area.width, 0); count];
    if count == 0 || area.height == 0 {
        return rects;
    }
    let active = active.min(count - 1);
    let bars = (count - 1) as u16;
    let active_height = area
        .height
        .saturating_sub(bars)
        .max(MIN_ACTIVE_STACK_HEIGHT.min(area.height));

    // Reserve the active pane's rows before handing out bars. Allocating
    // purely left to right let enough preceding bars eat the whole area and
    // starve the active pane to zero height.
    let mut budget = area.height;
    let mut heights = vec![0u16; count];
    heights[active] = active_height.min(budget);
    budget -= heights[active];
    for (index, height) in heights.iter_mut().enumerate() {
        if index != active {
            *height = 1.min(budget);
            budget -= *height;
        }
    }

    let mut y = area.y;
    for (rect, height) in rects.iter_mut().zip(heights) {
        *rect = Rect::new(area.x, y, area.width, height);
        y = y.saturating_add(height);
    }
    rects
}

/// A node in the BSP tree. Public for serialisation.
#[derive(Debug)]
pub enum Node {
    Pane(PaneId),
    Split {
        direction: Direction,
        ratio: f32,
        first: Box<Node>,
        second: Box<Node>,
    },
    /// A flat stack of panes sharing one rect. Stacks do not nest, and a split
    /// cannot live inside one, which keeps the tree recursion to a single shape.
    Stack {
        panes: Vec<PaneId>,
        active: usize,
    },
}

/// Keep every stack's active index pointing at the focused pane when it holds
/// it. Rendering and navigation both rely on the focused pane being the one
/// with the expanded rect.
fn sync_stack_active(node: &mut Node, focus: PaneId) {
    match node {
        Node::Pane(_) => {}
        Node::Split { first, second, .. } => {
            sync_stack_active(first, focus);
            sync_stack_active(second, focus);
        }
        Node::Stack { panes, active } => {
            if let Some(index) = panes.iter().position(|id| *id == focus) {
                *active = index;
            }
        }
    }
}

/// BSP tiling layout. Tracks a tree of splits and a focused pane.
pub struct TileLayout {
    root: Node,
    focus: PaneId,
    /// Pane focused before `focus`, used by `close_focused`. Only a real focus
    /// move writes it; tree edits go through the target-taking primitives
    /// (`split_pane`, `close_pane`, unfocused `insert_pane_near`) so internal
    /// focus excursions never corrupt it.
    prev_focus: Option<PaneId>,
}

impl TileLayout {
    /// Create a new layout with a single pane (globally unique ID).
    /// Returns (layout, root_pane_id) so the caller can create the pane.
    pub fn new() -> (Self, PaneId) {
        let root_id = PaneId::alloc();
        (
            Self {
                root: Node::Pane(root_id),
                focus: root_id,
                prev_focus: None,
            },
            root_id,
        )
    }

    /// Move focus, recording the pane being left. No-op when focus is unchanged.
    fn set_focus(&mut self, id: PaneId) {
        if id != self.focus {
            self.prev_focus = Some(self.focus);
            self.focus = id;
        }
        // Runs unconditionally: a tree edit can leave a stale `active` index
        // behind even when the focused id itself did not change.
        sync_stack_active(&mut self.root, id);
    }

    pub fn focused(&self) -> PaneId {
        self.focus
    }

    pub fn pane_count(&self) -> usize {
        count_panes(&self.root)
    }

    /// Compute rects for all panes given the available area.
    pub fn panes(&self, area: Rect) -> Vec<PaneInfo> {
        let mut result = Vec::new();
        collect_panes(&self.root, area, self.focus, &mut result);
        result
    }

    /// Collect all split boundaries for mouse drag resize.
    pub fn splits(&self, area: Rect) -> Vec<SplitBorder> {
        let mut result = Vec::new();
        collect_splits(&self.root, area, vec![], &mut result);
        result
    }

    /// Split the focused pane. Returns the new pane's id. Production splits
    /// flow through `Tab` so a failed runtime spawn can roll back; this remains
    /// as the user-split shape for tests.
    #[cfg(test)]
    pub fn split_focused(&mut self, direction: Direction) -> PaneId {
        self.split_focused_with_ratio(direction, 0.5)
    }

    /// Split the focused pane with a custom first-child ratio.
    #[cfg(test)]
    pub fn split_focused_with_ratio(&mut self, direction: Direction, ratio: f32) -> PaneId {
        let new_id = self
            .split_pane(self.focus, direction, ratio)
            .expect("focused pane is in the layout");
        self.set_focus(new_id);
        new_id
    }

    /// Split `target` without moving focus. Returns the new pane's id, or None
    /// when `target` is not in the layout.
    pub fn split_pane(
        &mut self,
        target: PaneId,
        direction: Direction,
        ratio: f32,
    ) -> Option<PaneId> {
        if !self.pane_ids().contains(&target) {
            return None;
        }
        let new_id = PaneId::alloc();
        let placeholder = PaneId::from_raw(0);
        let old = std::mem::replace(&mut self.root, Node::Pane(placeholder));
        self.root = split_at(old, target, direction, new_id, valid_split_ratio(ratio));
        Some(new_id)
    }

    /// Insert an existing pane id next to a target pane without allocating a new
    /// pane or spawning a terminal runtime. When `focus` is false, focus and its
    /// history are left untouched.
    pub fn insert_pane_near(
        &mut self,
        target: PaneId,
        moved: PaneId,
        direction: Direction,
        ratio: f32,
        focus: bool,
    ) -> bool {
        if target == moved {
            return false;
        }
        let ids = self.pane_ids();
        if !ids.contains(&target) || ids.contains(&moved) {
            return false;
        }

        let placeholder = PaneId::from_raw(0);
        let old = std::mem::replace(&mut self.root, Node::Pane(placeholder));
        self.root = split_at(old, target, direction, moved, valid_split_ratio(ratio));
        // `split_at` returns the tree untouched when it cannot place the pane,
        // so confirm the insertion rather than reporting success blindly.
        if !self.pane_ids().contains(&moved) {
            return false;
        }
        if focus {
            self.set_focus(moved);
        }
        true
    }

    /// Close the focused pane, returning focus to the pane it came from when
    /// that pane is still open. Returns false if it's the last pane.
    pub fn close_focused(&mut self) -> bool {
        if self.pane_count() <= 1 {
            return false;
        }
        let target = self.focus;
        let ids = self.pane_ids();
        let Some(pos) = ids.iter().position(|id| *id == target) else {
            // Nothing to close, but a focus outside the tree would keep routing
            // input at a pane nobody can see, so re-anchor it on a real one.
            if let Some(first) = ids.first().copied() {
                self.set_focus(first);
            }
            return false;
        };
        let ordered = if pos + 1 < ids.len() {
            ids[pos + 1]
        } else {
            ids[pos - 1]
        };
        let new_focus = match self.prev_focus {
            Some(prev) if prev != target && ids.contains(&prev) => prev,
            _ => ordered,
        };
        let placeholder = PaneId::from_raw(0);
        let old = std::mem::replace(&mut self.root, Node::Pane(placeholder));
        if let Some(new_root) = remove_pane(old, target) {
            self.root = new_root;
            self.set_focus(new_focus);
            // The history entry was just consumed to pick this focus.
            self.prev_focus = None;
            true
        } else {
            false
        }
    }

    /// Close any pane. Focus and its history are left alone unless the closed
    /// pane is the focused one.
    pub fn close_pane(&mut self, id: PaneId) -> bool {
        if self.focus == id {
            return self.close_focused();
        }
        if self.pane_count() <= 1 || !self.pane_ids().contains(&id) {
            return false;
        }
        let placeholder = PaneId::from_raw(0);
        let old = std::mem::replace(&mut self.root, Node::Pane(placeholder));
        let Some(new_root) = remove_pane(old, id) else {
            return false;
        };
        self.root = new_root;
        if self.prev_focus == Some(id) {
            self.prev_focus = None;
        }
        true
    }

    pub fn focus_pane(&mut self, id: PaneId) {
        if self.pane_ids().contains(&id) {
            self.set_focus(id);
        }
    }

    /// Swap two pane ids in the layout tree while preserving split shape and
    /// ratios. Returns true only when both panes exist and are different.
    pub fn swap_panes(&mut self, first: PaneId, second: PaneId) -> bool {
        if first == second {
            return false;
        }
        let ids = self.pane_ids();
        if !ids.contains(&first) || !ids.contains(&second) {
            return false;
        }
        swap_pane_ids(&mut self.root, first, second);
        sync_stack_active(&mut self.root, self.focus);
        true
    }

    /// Set the ratio of a split node at the given path.
    pub fn set_ratio_at(&mut self, path: &[bool], ratio: f32) -> bool {
        set_ratio_at(&mut self.root, path, ratio.clamp(0.1, 0.9))
    }

    /// Adjust the nearest split in the given direction for the focused pane.
    /// `delta` is positive to grow, negative to shrink.
    pub fn resize_focused(&mut self, nav: NavDirection, delta: f32, area: Rect) {
        let panes = self.panes(area);
        let Some(focused) = panes.iter().find(|p| p.is_focused) else {
            return;
        };
        let focused_rect = focused.rect;
        let splits = self.splits(area);

        let target_dir = match nav {
            NavDirection::Left | NavDirection::Right => Direction::Horizontal,
            NavDirection::Up | NavDirection::Down => Direction::Vertical,
        };
        let grows = matches!(nav, NavDirection::Right | NavDirection::Down);

        let best = nearest_resize_split(&splits, target_dir, focused_rect, nav).or_else(|| {
            nearest_resize_split(&splits, target_dir, focused_rect, opposite_direction(nav))
        });

        if let Some(split) = best {
            let path = split.path.clone();
            let current_ratio = get_ratio_at(&self.root, &path).unwrap_or(0.5);
            let adj = if grows { delta } else { -delta };
            self.set_ratio_at(&path, current_ratio + adj);
        }
    }

    pub fn resize_pane(
        &mut self,
        pane_id: PaneId,
        nav: NavDirection,
        delta: f32,
        area: Rect,
    ) -> bool {
        if !self.pane_ids().contains(&pane_id) {
            return false;
        }
        let before = split_ratios(&self.root);
        let previous_focus = self.focus;
        self.focus = pane_id;
        self.resize_focused(nav, delta, area);
        self.focus = previous_focus;
        split_ratios(&self.root) != before
    }

    pub fn pane_ids(&self) -> Vec<PaneId> {
        let mut ids = Vec::new();
        collect_ids(&self.root, &mut ids);
        ids
    }

    /// Access the tree root for serialization.
    pub fn root(&self) -> &Node {
        &self.root
    }

    /// Reconstruct a layout from a saved tree.
    pub fn from_saved(root: Node, focus: PaneId) -> Self {
        let mut layout = Self {
            root,
            focus,
            prev_focus: None,
        };
        sync_stack_active(&mut layout.root, focus);
        layout
    }
}

// --- Directional pane navigation ---

/// Find the nearest pane in the given direction from `focused`.
pub fn find_in_direction(
    focused: &PaneInfo,
    direction: NavDirection,
    panes: &[PaneInfo],
) -> Option<PaneId> {
    let fr = focused.rect;

    panes
        .iter()
        .enumerate()
        .filter(|(_, p)| p.id != focused.id)
        .filter(|(_, p)| {
            let r = p.rect;
            match direction {
                NavDirection::Left => {
                    r.x + r.width <= fr.x && ranges_overlap(r.y, r.height, fr.y, fr.height)
                }
                NavDirection::Right => {
                    r.x >= fr.x + fr.width && ranges_overlap(r.y, r.height, fr.y, fr.height)
                }
                NavDirection::Up => {
                    r.y + r.height <= fr.y && ranges_overlap(r.x, r.width, fr.x, fr.width)
                }
                NavDirection::Down => {
                    r.y >= fr.y + fr.height && ranges_overlap(r.x, r.width, fr.x, fr.width)
                }
            }
        })
        .min_by_key(|(index, p)| {
            let r = p.rect;
            let edge_distance = match direction {
                NavDirection::Left => fr.x.saturating_sub(r.x + r.width),
                NavDirection::Right => r.x.saturating_sub(fr.x + fr.width),
                NavDirection::Up => fr.y.saturating_sub(r.y + r.height),
                NavDirection::Down => r.y.saturating_sub(fr.y + fr.height),
            };
            let overlap = match direction {
                NavDirection::Left | NavDirection::Right => {
                    range_overlap_amount(r.y, r.height, fr.y, fr.height)
                }
                NavDirection::Up | NavDirection::Down => {
                    range_overlap_amount(r.x, r.width, fr.x, fr.width)
                }
            };
            let center_distance = match direction {
                NavDirection::Left | NavDirection::Right => {
                    range_center_distance(r.y, r.height, fr.y, fr.height)
                }
                NavDirection::Up | NavDirection::Down => {
                    range_center_distance(r.x, r.width, fr.x, fr.width)
                }
            };
            (edge_distance, Reverse(overlap), center_distance, *index)
        })
        .map(|(_, p)| p.id)
}

fn ranges_overlap(a_start: u16, a_len: u16, b_start: u16, b_len: u16) -> bool {
    a_start < b_start + b_len && a_start + a_len > b_start
}

fn split_on_requested_edge(split: &SplitBorder, focused: Rect, nav: NavDirection) -> bool {
    split_edge_distance(split, focused, nav) <= 1
}

fn split_area_overlaps_focused_pane(split: &SplitBorder, focused: Rect, nav: NavDirection) -> bool {
    match nav {
        NavDirection::Left | NavDirection::Right => {
            ranges_overlap(split.area.y, split.area.height, focused.y, focused.height)
        }
        NavDirection::Up | NavDirection::Down => {
            ranges_overlap(split.area.x, split.area.width, focused.x, focused.width)
        }
    }
}

fn nearest_resize_split(
    splits: &[SplitBorder],
    target_dir: Direction,
    focused: Rect,
    nav: NavDirection,
) -> Option<&SplitBorder> {
    splits
        .iter()
        .filter(|s| s.direction == target_dir)
        .filter(|s| split_area_overlaps_focused_pane(s, focused, nav))
        .filter(|s| split_on_requested_edge(s, focused, nav))
        .min_by_key(|s| split_edge_distance(s, focused, nav))
}

fn opposite_direction(nav: NavDirection) -> NavDirection {
    match nav {
        NavDirection::Left => NavDirection::Right,
        NavDirection::Right => NavDirection::Left,
        NavDirection::Up => NavDirection::Down,
        NavDirection::Down => NavDirection::Up,
    }
}

fn split_edge_distance(split: &SplitBorder, focused: Rect, nav: NavDirection) -> u32 {
    match nav {
        NavDirection::Left => (split.pos as i32 - focused.x as i32).unsigned_abs(),
        NavDirection::Right => {
            (split.pos as i32 - (focused.x + focused.width) as i32).unsigned_abs()
        }
        NavDirection::Up => (split.pos as i32 - focused.y as i32).unsigned_abs(),
        NavDirection::Down => {
            (split.pos as i32 - (focused.y + focused.height) as i32).unsigned_abs()
        }
    }
}

fn range_overlap_amount(a_start: u16, a_len: u16, b_start: u16, b_len: u16) -> u16 {
    let a_end = a_start.saturating_add(a_len);
    let b_end = b_start.saturating_add(b_len);
    a_end.min(b_end).saturating_sub(a_start.max(b_start))
}

fn range_center_distance(a_start: u16, a_len: u16, b_start: u16, b_len: u16) -> u16 {
    let a_center = a_start.saturating_mul(2).saturating_add(a_len);
    let b_center = b_start.saturating_mul(2).saturating_add(b_len);
    a_center.abs_diff(b_center)
}

// --- Tree operations ---

fn count_panes(node: &Node) -> usize {
    match node {
        Node::Pane(_) => 1,
        Node::Split { first, second, .. } => count_panes(first) + count_panes(second),
        Node::Stack { panes, .. } => panes.len(),
    }
}

fn collect_panes(node: &Node, area: Rect, focus: PaneId, result: &mut Vec<PaneInfo>) {
    match node {
        Node::Pane(id) => {
            result.push(PaneInfo {
                id: *id,
                rect: area,
                // inner_rect is set during render when we know if borders are shown
                inner_rect: area,
                scrollbar_rect: None,
                borders: Borders::NONE,
                is_focused: *id == focus,
            });
        }
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let (a, b) = split_rect(area, *direction, *ratio);
            collect_panes(first, a, focus, result);
            collect_panes(second, b, focus, result);
        }
        Node::Stack { panes, active } => {
            for (rect, id) in stack_rects(area, panes.len(), *active)
                .into_iter()
                .zip(panes.iter())
            {
                result.push(PaneInfo {
                    id: *id,
                    rect,
                    inner_rect: rect,
                    scrollbar_rect: None,
                    borders: Borders::NONE,
                    is_focused: *id == focus,
                });
            }
        }
    }
}

fn collect_splits(node: &Node, area: Rect, path: Vec<bool>, result: &mut Vec<SplitBorder>) {
    if let Node::Split {
        direction,
        ratio,
        first,
        second,
    } = node
    {
        let (a, b) = split_rect(area, *direction, *ratio);
        let pos = match direction {
            Direction::Horizontal => a.x + a.width,
            Direction::Vertical => a.y + a.height,
        };
        result.push(SplitBorder {
            pos,
            direction: *direction,
            ratio: *ratio,
            area,
            path: path.clone(),
        });
        let mut lp = path.clone();
        lp.push(false);
        collect_splits(first, a, lp, result);
        let mut rp = path;
        rp.push(true);
        collect_splits(second, b, rp, result);
    }
}

fn collect_ids(node: &Node, ids: &mut Vec<PaneId>) {
    match node {
        Node::Pane(id) => ids.push(*id),
        Node::Split { first, second, .. } => {
            collect_ids(first, ids);
            collect_ids(second, ids);
        }
        Node::Stack { panes, .. } => ids.extend(panes.iter().copied()),
    }
}

fn split_ratios(node: &Node) -> Vec<(Vec<bool>, f32)> {
    fn collect(node: &Node, path: &mut Vec<bool>, out: &mut Vec<(Vec<bool>, f32)>) {
        match node {
            Node::Pane(_) => {}
            Node::Split {
                ratio,
                first,
                second,
                ..
            } => {
                out.push((path.clone(), *ratio));
                path.push(false);
                collect(first, path, out);
                path.pop();
                path.push(true);
                collect(second, path, out);
                path.pop();
            }
            Node::Stack { .. } => {}
        }
    }

    let mut out = Vec::new();
    collect(node, &mut Vec::new(), &mut out);
    out
}

fn swap_pane_ids(node: &mut Node, first: PaneId, second: PaneId) {
    match node {
        Node::Pane(id) if *id == first => *id = second,
        Node::Pane(id) if *id == second => *id = first,
        Node::Pane(_) => {}
        Node::Split {
            first: first_child,
            second: second_child,
            ..
        } => {
            swap_pane_ids(first_child, first, second);
            swap_pane_ids(second_child, first, second);
        }
        Node::Stack { panes, .. } => {
            for id in panes.iter_mut() {
                if *id == first {
                    *id = second;
                } else if *id == second {
                    *id = first;
                }
            }
        }
    }
}

fn split_at(
    node: Node,
    target: PaneId,
    direction: Direction,
    new_id: PaneId,
    split_ratio: f32,
) -> Node {
    match node {
        Node::Pane(id) if id == target => Node::Split {
            direction,
            ratio: split_ratio,
            first: Box::new(Node::Pane(id)),
            second: Box::new(Node::Pane(new_id)),
        },
        Node::Pane(_) => node,
        Node::Split {
            direction: d,
            ratio,
            first,
            second,
        } => Node::Split {
            direction: d,
            ratio,
            first: Box::new(split_at(*first, target, direction, new_id, split_ratio)),
            second: Box::new(split_at(*second, target, direction, new_id, split_ratio)),
        },
        // A stack has no split direction, so the new pane joins the stack
        // directly after its target. Leaving it out would orphan a pane the
        // caller has already allocated and focused: re-flow rebuilds from
        // `pane_ids()`, so a pane missing from the tree can never come back.
        Node::Stack { mut panes, active } => match panes.iter().position(|id| *id == target) {
            Some(index) => {
                panes.insert(index + 1, new_id);
                Node::Stack {
                    panes,
                    active: index + 1,
                }
            }
            None => Node::Stack { panes, active },
        },
    }
}

fn valid_split_ratio(ratio: f32) -> f32 {
    if ratio.is_finite() {
        ratio.clamp(0.1, 0.9)
    } else {
        0.5
    }
}

fn remove_pane(node: Node, target: PaneId) -> Option<Node> {
    match node {
        Node::Pane(id) if id == target => None,
        Node::Pane(_) => Some(node),
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => match (remove_pane(*first, target), remove_pane(*second, target)) {
            (None, Some(s)) => Some(s),
            (Some(f), None) => Some(f),
            (Some(f), Some(s)) => Some(Node::Split {
                direction,
                ratio,
                first: Box::new(f),
                second: Box::new(s),
            }),
            (None, None) => None,
        },
        Node::Stack {
            mut panes,
            mut active,
        } => {
            panes.retain(|id| *id != target);
            match panes.len() {
                0 => None,
                1 => Some(Node::Pane(panes[0])),
                len => {
                    active = active.min(len - 1);
                    Some(Node::Stack { panes, active })
                }
            }
        }
    }
}

fn set_ratio_at(node: &mut Node, path: &[bool], new_ratio: f32) -> bool {
    if let Node::Split {
        ratio,
        first,
        second,
        ..
    } = node
    {
        if path.is_empty() {
            *ratio = new_ratio;
            true
        } else if path[0] {
            set_ratio_at(second, &path[1..], new_ratio)
        } else {
            set_ratio_at(first, &path[1..], new_ratio)
        }
    } else {
        false
    }
}

fn get_ratio_at(node: &Node, path: &[bool]) -> Option<f32> {
    if let Node::Split {
        ratio,
        first,
        second,
        ..
    } = node
    {
        if path.is_empty() {
            Some(*ratio)
        } else if path[0] {
            get_ratio_at(second, &path[1..])
        } else {
            get_ratio_at(first, &path[1..])
        }
    } else {
        None
    }
}

fn split_rect(area: Rect, direction: Direction, ratio: f32) -> (Rect, Rect) {
    match direction {
        Direction::Horizontal => {
            let first_w = ((area.width as f32) * ratio).round() as u16;
            let second_w = area.width.saturating_sub(first_w);
            (
                Rect::new(area.x, area.y, first_w, area.height),
                Rect::new(area.x + first_w, area.y, second_w, area.height),
            )
        }
        Direction::Vertical => {
            let first_h = ((area.height as f32) * ratio).round() as u16;
            let second_h = area.height.saturating_sub(first_h);
            (
                Rect::new(area.x, area.y, area.width, first_h),
                Rect::new(area.x, area.y + first_h, area.width, second_h),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: u32) -> PaneId {
        PaneId::from_raw(id)
    }

    fn sample_layout() -> TileLayout {
        TileLayout::from_saved(
            Node::Split {
                direction: Direction::Horizontal,
                ratio: 0.3,
                first: Box::new(Node::Pane(pane(1))),
                second: Box::new(Node::Split {
                    direction: Direction::Vertical,
                    ratio: 0.6,
                    first: Box::new(Node::Pane(pane(2))),
                    second: Box::new(Node::Split {
                        direction: Direction::Horizontal,
                        ratio: 0.4,
                        first: Box::new(Node::Pane(pane(3))),
                        second: Box::new(Node::Pane(pane(4))),
                    }),
                }),
            },
            pane(2),
        )
    }

    fn pane_rects(layout: &TileLayout) -> Vec<(PaneId, Rect)> {
        layout
            .panes(Rect::new(0, 0, 100, 40))
            .into_iter()
            .map(|info| (info.id, info.rect))
            .collect()
    }

    fn pane_rect(layout: &TileLayout, pane_id: PaneId) -> Rect {
        pane_rects(layout)
            .into_iter()
            .find_map(|(id, rect)| (id == pane_id).then_some(rect))
            .expect("pane should exist")
    }

    fn split_snapshot(layout: &TileLayout) -> Vec<(Direction, f32)> {
        fn collect(node: &Node, out: &mut Vec<(Direction, f32)>) {
            match node {
                Node::Pane(_) => {}
                Node::Split {
                    direction,
                    ratio,
                    first,
                    second,
                } => {
                    out.push((*direction, *ratio));
                    collect(first, out);
                    collect(second, out);
                }
                Node::Stack { .. } => {}
            }
        }

        let mut out = Vec::new();
        collect(layout.root(), &mut out);
        out
    }

    #[test]
    fn swap_panes_exchanges_leaf_ids_without_changing_cells() {
        let mut layout = sample_layout();
        let before_rects = pane_rects(&layout);
        let before_splits = split_snapshot(&layout);

        assert!(layout.swap_panes(pane(2), pane(4)));

        assert_eq!(layout.pane_count(), 4);
        assert_eq!(split_snapshot(&layout), before_splits);
        assert_eq!(layout.focused(), pane(2));

        let after_rects = pane_rects(&layout);
        assert_eq!(after_rects[0], before_rects[0]);
        assert_eq!(after_rects[1], (pane(4), before_rects[1].1));
        assert_eq!(after_rects[2], before_rects[2]);
        assert_eq!(after_rects[3], (pane(2), before_rects[3].1));
    }

    #[test]
    fn swap_panes_is_noop_for_same_or_missing_pane() {
        let mut layout = sample_layout();
        let before_rects = pane_rects(&layout);
        let before_splits = split_snapshot(&layout);
        let before_focus = layout.focused();

        assert!(!layout.swap_panes(pane(2), pane(2)));
        assert!(!layout.swap_panes(pane(2), pane(99)));
        assert!(!layout.swap_panes(pane(99), pane(2)));

        assert_eq!(pane_rects(&layout), before_rects);
        assert_eq!(split_snapshot(&layout), before_splits);
        assert_eq!(layout.focused(), before_focus);
    }

    #[test]
    fn insert_existing_pane_near_target_preserves_existing_ids_and_focuses_moved_pane() {
        let (mut layout, root) = TileLayout::new();
        let moved = pane(99);

        assert!(layout.insert_pane_near(root, moved, Direction::Horizontal, 0.25, true));

        assert_eq!(layout.pane_count(), 2);
        assert_eq!(layout.pane_ids(), vec![root, moved]);
        assert_eq!(layout.focused(), moved);
        let splits = split_snapshot(&layout);
        assert_eq!(splits, vec![(Direction::Horizontal, 0.25)]);
        assert_eq!(pane_rect(&layout, root), Rect::new(0, 0, 25, 40));
        assert_eq!(pane_rect(&layout, moved), Rect::new(25, 0, 75, 40));
    }

    #[test]
    fn split_focused_with_ratio_sets_new_split_ratio() {
        let (mut layout, root) = TileLayout::new();
        layout.focus_pane(root);

        layout.split_focused_with_ratio(Direction::Horizontal, 0.333);

        let splits = split_snapshot(&layout);
        assert_eq!(splits.len(), 1);
        assert_eq!(splits[0].0, Direction::Horizontal);
        assert!((splits[0].1 - 0.333).abs() < f32::EPSILON);
    }

    #[test]
    fn resize_pane_preserves_focus_and_reports_change() {
        let mut layout = sample_layout();
        let original_focus = layout.focused();

        assert!(layout.resize_pane(pane(1), NavDirection::Right, 0.05, Rect::new(0, 0, 100, 40),));

        assert_eq!(layout.focused(), original_focus);
        let split = split_snapshot(&layout)[0];
        assert_eq!(split.0, Direction::Horizontal);
        assert!((split.1 - 0.35).abs() < f32::EPSILON);
    }

    #[test]
    fn resize_second_child_toward_split_decreases_ratio() {
        let (mut layout, root) = TileLayout::new();
        let right = layout.split_focused(Direction::Horizontal);
        layout.focus_pane(root);

        assert!(layout.resize_pane(right, NavDirection::Left, 0.05, Rect::new(0, 0, 100, 40),));

        let split = split_snapshot(&layout)[0];
        assert_eq!(split.0, Direction::Horizontal);
        assert!((split.1 - 0.45).abs() < f32::EPSILON);
        assert_eq!(layout.focused(), root);
    }

    #[test]
    fn resize_outer_edges_shrink_focused_pane() {
        let (mut horizontal, left) = TileLayout::new();
        horizontal.split_focused(Direction::Horizontal);

        assert!(horizontal.resize_pane(left, NavDirection::Left, 0.05, Rect::new(0, 0, 100, 40),));
        let split = split_snapshot(&horizontal)[0];
        assert_eq!(split.0, Direction::Horizontal);
        assert!((split.1 - 0.45).abs() < f32::EPSILON);

        let (mut horizontal, _left) = TileLayout::new();
        let right = horizontal.split_focused(Direction::Horizontal);

        assert!(horizontal.resize_pane(right, NavDirection::Right, 0.05, Rect::new(0, 0, 100, 40),));
        let split = split_snapshot(&horizontal)[0];
        assert_eq!(split.0, Direction::Horizontal);
        assert!((split.1 - 0.55).abs() < f32::EPSILON);

        let (mut vertical, top) = TileLayout::new();
        vertical.split_focused(Direction::Vertical);

        assert!(vertical.resize_pane(top, NavDirection::Up, 0.05, Rect::new(0, 0, 100, 40),));
        let split = split_snapshot(&vertical)[0];
        assert_eq!(split.0, Direction::Vertical);
        assert!((split.1 - 0.45).abs() < f32::EPSILON);

        let (mut vertical, _top) = TileLayout::new();
        let bottom = vertical.split_focused(Direction::Vertical);

        assert!(vertical.resize_pane(bottom, NavDirection::Down, 0.05, Rect::new(0, 0, 100, 40),));
        let split = split_snapshot(&vertical)[0];
        assert_eq!(split.0, Direction::Vertical);
        assert!((split.1 - 0.55).abs() < f32::EPSILON);
    }

    #[test]
    fn resize_outer_edge_falls_back_to_horizontal_ancestor_split() {
        let mut layout = TileLayout::from_saved(
            Node::Split {
                direction: Direction::Horizontal,
                ratio: 0.6,
                first: Box::new(Node::Split {
                    direction: Direction::Vertical,
                    ratio: 0.5,
                    first: Box::new(Node::Pane(pane(1))),
                    second: Box::new(Node::Pane(pane(2))),
                }),
                second: Box::new(Node::Pane(pane(3))),
            },
            pane(1),
        );
        let before = pane_rect(&layout, pane(1));

        assert!(layout.resize_pane(pane(1), NavDirection::Left, 0.05, Rect::new(0, 0, 100, 40),));

        let after = pane_rect(&layout, pane(1));
        assert_eq!(after.height, before.height);
        assert!(after.width < before.width);
        let splits = split_snapshot(&layout);
        assert_eq!(splits[0].0, Direction::Horizontal);
        assert!((splits[0].1 - 0.55).abs() < f32::EPSILON);
        assert_eq!(splits[1], (Direction::Vertical, 0.5));
    }

    #[test]
    fn resize_outer_edge_falls_back_to_vertical_ancestor_split() {
        let mut layout = TileLayout::from_saved(
            Node::Split {
                direction: Direction::Vertical,
                ratio: 0.6,
                first: Box::new(Node::Split {
                    direction: Direction::Horizontal,
                    ratio: 0.5,
                    first: Box::new(Node::Pane(pane(1))),
                    second: Box::new(Node::Pane(pane(2))),
                }),
                second: Box::new(Node::Pane(pane(3))),
            },
            pane(1),
        );
        let before = pane_rect(&layout, pane(1));

        assert!(layout.resize_pane(pane(1), NavDirection::Up, 0.05, Rect::new(0, 0, 100, 40),));

        let after = pane_rect(&layout, pane(1));
        assert_eq!(after.width, before.width);
        assert!(after.height < before.height);
        let splits = split_snapshot(&layout);
        assert_eq!(splits[0].0, Direction::Vertical);
        assert!((splits[0].1 - 0.55).abs() < f32::EPSILON);
        assert_eq!(splits[1], (Direction::Horizontal, 0.5));
    }

    #[test]
    fn resize_uses_split_in_same_branch_when_borders_share_coordinate() {
        let mut layout = TileLayout::from_saved(
            Node::Split {
                direction: Direction::Vertical,
                ratio: 0.5,
                first: Box::new(Node::Split {
                    direction: Direction::Horizontal,
                    ratio: 0.5,
                    first: Box::new(Node::Pane(pane(1))),
                    second: Box::new(Node::Pane(pane(2))),
                }),
                second: Box::new(Node::Split {
                    direction: Direction::Horizontal,
                    ratio: 0.5,
                    first: Box::new(Node::Pane(pane(3))),
                    second: Box::new(Node::Pane(pane(4))),
                }),
            },
            pane(3),
        );

        assert!(layout.resize_pane(pane(3), NavDirection::Right, 0.05, Rect::new(0, 0, 100, 40),));

        let splits = split_snapshot(&layout);
        assert_eq!(splits[0], (Direction::Vertical, 0.5));
        assert_eq!(splits[1], (Direction::Horizontal, 0.5));
        assert_eq!(splits[2].0, Direction::Horizontal);
        assert!((splits[2].1 - 0.55).abs() < f32::EPSILON);
    }

    #[test]
    fn find_in_direction_tiebreaks_by_larger_overlap_before_layout_order() {
        let focused = PaneInfo {
            id: pane(1),
            rect: Rect::new(10, 10, 10, 10),
            inner_rect: Rect::new(10, 10, 10, 10),
            scrollbar_rect: None,
            borders: Borders::NONE,
            is_focused: true,
        };
        let small_overlap_first = PaneInfo {
            id: pane(2),
            rect: Rect::new(0, 10, 10, 2),
            inner_rect: Rect::new(0, 10, 10, 2),
            scrollbar_rect: None,
            borders: Borders::NONE,
            is_focused: false,
        };
        let larger_overlap_second = PaneInfo {
            id: pane(3),
            rect: Rect::new(0, 10, 10, 8),
            inner_rect: Rect::new(0, 10, 10, 8),
            scrollbar_rect: None,
            borders: Borders::NONE,
            is_focused: false,
        };
        let panes = vec![focused.clone(), small_overlap_first, larger_overlap_second];

        assert_eq!(
            find_in_direction(&focused, NavDirection::Left, &panes),
            Some(pane(3))
        );
    }

    fn ids(count: usize) -> Vec<PaneId> {
        (1..=count as u32).map(PaneId::from_raw).collect()
    }

    #[test]
    fn arrange_preserves_pane_order_for_every_arrangement() {
        let area = Rect::new(0, 0, 120, 40);
        for kind in [
            Arrangement::Vertical,
            Arrangement::Horizontal,
            Arrangement::Grid,
        ] {
            for count in 1..=12 {
                let panes = ids(count);
                let root = arrange(kind, &panes, panes[0], area)
                    .expect("non-empty pane list always yields a tree");
                let mut walked = Vec::new();
                collect_ids(&root, &mut walked);
                assert_eq!(walked, panes, "{kind:?} reordered panes at count {count}");
            }
        }
    }

    #[test]
    fn arrange_returns_none_for_an_empty_pane_list() {
        let area = Rect::new(0, 0, 120, 40);
        assert!(arrange(Arrangement::Grid, &[], PaneId::from_raw(1), area).is_none());
    }

    #[test]
    fn vertical_arrangement_produces_even_columns() {
        let area = Rect::new(0, 0, 120, 40);
        let panes = ids(4);
        let root = arrange(Arrangement::Vertical, &panes, panes[0], area).expect("tree");
        let mut infos = Vec::new();
        collect_panes(&root, area, panes[0], &mut infos);
        assert_eq!(infos.len(), 4);
        for info in &infos {
            assert_eq!(info.rect.width, 30);
            assert_eq!(info.rect.height, 40);
        }
    }

    #[test]
    fn horizontal_arrangement_produces_even_rows() {
        let area = Rect::new(0, 0, 120, 40);
        let panes = ids(4);
        let root = arrange(Arrangement::Horizontal, &panes, panes[0], area).expect("tree");
        let mut infos = Vec::new();
        collect_panes(&root, area, panes[0], &mut infos);
        assert_eq!(infos.len(), 4);
        for info in &infos {
            assert_eq!(info.rect.width, 120);
            assert_eq!(info.rect.height, 10);
        }
    }

    #[test]
    fn grid_columns_favour_balanced_square_grids() {
        // Cells are about twice as tall as wide. Four panes on a wide area want
        // two columns of two: three columns would give squarer cells but leaves a
        // ragged 2/1/1 grid, which the empty-cell penalty rules out.
        let area = Rect::new(0, 0, 120, 40);
        assert_eq!(grid_columns(1, area), 1);
        assert_eq!(grid_columns(2, area), 2);
        assert_eq!(grid_columns(4, area), 2);
        assert_eq!(grid_columns(6, area), 3);
        assert_eq!(grid_columns(9, area), 3);
    }

    #[test]
    fn grid_columns_stay_balanced_on_a_wide_area() {
        // A wide terminal used to make the ragged 2/1/1 grid score better than
        // the balanced 2x2 one.
        let area = Rect::new(0, 0, 220, 50);
        assert_eq!(grid_columns(4, area), 2);
        assert_eq!(grid_columns(6, area), 3);
    }

    #[test]
    fn grid_gives_the_extra_pane_to_the_earlier_column() {
        let area = Rect::new(0, 0, 120, 40);
        let panes = ids(5);
        let root = arrange(Arrangement::Grid, &panes, panes[0], area).expect("tree");
        let mut walked = Vec::new();
        collect_ids(&root, &mut walked);
        assert_eq!(walked, panes);
        assert_eq!(count_panes(&root), 5);
        let Node::Split { first, .. } = &root else {
            panic!("expected a column split, got {root:?}");
        };
        assert_eq!(count_panes(first), 2);
    }

    #[test]
    fn arrangement_cycles_forward_and_wraps() {
        let mut kind = Arrangement::Vertical;
        let mut seen = vec![kind];
        for _ in 0..3 {
            kind = kind.next();
            seen.push(kind);
        }
        assert_eq!(
            seen,
            vec![
                Arrangement::Vertical,
                Arrangement::Horizontal,
                Arrangement::Grid,
                Arrangement::Stacked,
            ]
        );
        assert_eq!(kind.next(), Arrangement::Vertical);
    }

    #[test]
    fn arrangement_previous_is_the_inverse_of_next() {
        for kind in Arrangement::ALL {
            assert_eq!(kind.next().previous(), kind);
            assert_eq!(kind.previous().next(), kind);
        }
    }

    #[test]
    fn arrangement_defaults_to_stacked() {
        assert_eq!(Arrangement::default(), Arrangement::Stacked);
    }

    #[test]
    fn stacked_arrangement_produces_one_stack_holding_every_pane() {
        let area = Rect::new(0, 0, 120, 40);
        let panes = ids(4);
        let root = arrange(Arrangement::Stacked, &panes, panes[2], area).expect("tree");
        match &root {
            Node::Stack {
                panes: members,
                active,
            } => {
                assert_eq!(members, &panes);
                assert_eq!(*active, 2);
            }
            other => panic!("expected a stack, got {other:?}"),
        }
    }

    #[test]
    fn stacked_arrangement_falls_back_to_the_first_pane_when_focus_is_absent() {
        let area = Rect::new(0, 0, 120, 40);
        let panes = ids(3);
        let root = arrange(Arrangement::Stacked, &panes, PaneId::from_raw(99), area).expect("tree");
        match &root {
            Node::Stack { active, .. } => assert_eq!(*active, 0),
            other => panic!("expected a stack, got {other:?}"),
        }
    }

    #[test]
    fn stack_rects_give_bars_to_inactive_panes_and_the_rest_to_the_active_one() {
        let area = Rect::new(0, 0, 80, 10);
        let rects = stack_rects(area, 3, 1);
        assert_eq!(rects[0].height, 1);
        assert_eq!(rects[1].height, 8);
        assert_eq!(rects[2].height, 1);
        assert_eq!(rects[0].y, 0);
        assert_eq!(rects[1].y, 1);
        assert_eq!(rects[2].y, 9);
        let total: u16 = rects.iter().map(|r| r.height).sum();
        assert_eq!(total, area.height);
    }

    #[test]
    fn stack_rects_never_overflow_a_short_area() {
        let area = Rect::new(0, 0, 80, 4);
        let rects = stack_rects(area, 5, 2);
        let total: u16 = rects.iter().map(|r| r.height).sum();
        assert_eq!(total, area.height);
        // Panes with no room left collapse to zero height. The renderer folds
        // these into a summary row.
        assert!(rects.iter().any(|r| r.height == 0));
        assert!(rects[2].height >= 1, "the active pane always keeps a row");
    }

    #[test]
    fn stack_rects_keep_the_active_pane_visible_when_many_members_precede_it() {
        let area = Rect::new(0, 0, 80, 10);
        let rects = stack_rects(area, 20, 15);
        assert!(rects[15].height >= 1, "the active pane must stay visible");
        let total: u16 = rects.iter().map(|r| r.height).sum();
        assert_eq!(total, area.height);
    }

    #[test]
    fn stack_members_each_get_a_pane_info() {
        let area = Rect::new(0, 0, 80, 10);
        let panes = ids(3);
        let root = arrange(Arrangement::Stacked, &panes, panes[1], area).expect("tree");
        let mut infos = Vec::new();
        collect_panes(&root, area, panes[1], &mut infos);
        assert_eq!(infos.len(), 3);
        assert!(infos[1].is_focused);
        assert!(!infos[0].is_focused);
    }

    #[test]
    fn removing_a_stack_member_clamps_the_active_index() {
        let panes = ids(3);
        let node = Node::Stack {
            panes: panes.clone(),
            active: 2,
        };
        let pruned = remove_pane(node, panes[2]).expect("stack survives");
        match &pruned {
            Node::Stack {
                panes: members,
                active,
            } => {
                assert_eq!(members.len(), 2);
                assert_eq!(*active, 1);
            }
            other => panic!("expected a stack, got {other:?}"),
        }
    }

    #[test]
    fn a_stack_of_one_collapses_to_a_plain_pane() {
        let panes = ids(2);
        let node = Node::Stack {
            panes: panes.clone(),
            active: 0,
        };
        let pruned = remove_pane(node, panes[0]).expect("pane survives");
        assert!(matches!(pruned, Node::Pane(id) if id == panes[1]));
    }

    #[test]
    fn arrange_preserves_pane_order_for_stacks_too() {
        let area = Rect::new(0, 0, 120, 40);
        for count in 1..=12 {
            let panes = ids(count);
            let root = arrange(Arrangement::Stacked, &panes, panes[0], area).expect("tree");
            let mut walked = Vec::new();
            collect_ids(&root, &mut walked);
            assert_eq!(walked, panes, "stack reordered panes at count {count}");
        }
    }

    /// Allocated rather than `ids()`, because these tests also call
    /// `split_focused`, which allocates from the same global counter.
    fn allocated(count: usize) -> Vec<PaneId> {
        (0..count).map(|_| PaneId::alloc()).collect()
    }

    fn stack_of(members: &[PaneId], focus: PaneId) -> TileLayout {
        TileLayout::from_saved(
            Node::Stack {
                panes: members.to_vec(),
                active: 0,
            },
            focus,
        )
    }

    #[test]
    fn splitting_a_stack_member_inserts_the_new_pane_after_it() {
        let members = allocated(3);
        let mut layout = stack_of(&members, members[1]);

        let new_id = layout.split_focused(Direction::Horizontal);

        assert_eq!(
            layout.pane_ids(),
            vec![members[0], members[1], new_id, members[2]]
        );
        assert_eq!(layout.focused(), new_id);
        assert!(layout.pane_ids().contains(&layout.focused()));
        match layout.root() {
            Node::Stack { active, .. } => assert_eq!(*active, 2),
            other => panic!("expected a stack, got {other:?}"),
        }
    }

    #[test]
    fn closing_a_pane_split_into_a_stack_does_not_panic() {
        let members = allocated(2);
        let mut layout = stack_of(&members, members[0]);
        let new_id = layout.split_focused(Direction::Horizontal);

        assert!(layout.close_focused());

        assert_eq!(layout.pane_ids(), members);
        assert!(!layout.pane_ids().contains(&new_id));
        assert!(layout.pane_ids().contains(&layout.focused()));
    }

    #[test]
    fn insert_pane_near_a_stack_member_lands_in_the_stack() {
        let members = allocated(3);
        let moved = PaneId::alloc();
        let mut layout = stack_of(&members, members[0]);

        assert!(layout.insert_pane_near(members[0], moved, Direction::Vertical, 0.5, true));

        assert_eq!(
            layout.pane_ids(),
            vec![members[0], moved, members[1], members[2]]
        );
        assert_eq!(layout.focused(), moved);
        assert!(matches!(layout.root(), Node::Stack { active, .. } if *active == 1));
    }

    #[test]
    fn close_focused_survives_a_focus_that_left_the_tree() {
        let members = allocated(2);
        let mut layout = stack_of(&members, PaneId::from_raw(u32::MAX));

        assert!(!layout.close_focused());

        assert_eq!(layout.pane_ids(), members);
        assert!(layout.pane_ids().contains(&layout.focused()));
    }

    #[test]
    fn focusing_a_stack_member_makes_it_active() {
        let panes = ids(3);
        let mut layout = TileLayout::from_saved(
            Node::Stack {
                panes: panes.clone(),
                active: 0,
            },
            panes[0],
        );
        layout.focus_pane(panes[2]);
        assert_eq!(layout.focused(), panes[2]);
        match layout.root() {
            Node::Stack { active, .. } => assert_eq!(*active, 2),
            other => panic!("expected a stack, got {other:?}"),
        }
    }

    #[test]
    fn the_focused_stack_member_is_the_one_with_the_tall_rect() {
        let area = Rect::new(0, 0, 80, 10);
        let panes = ids(3);
        let mut layout = TileLayout::from_saved(
            Node::Stack {
                panes: panes.clone(),
                active: 0,
            },
            panes[0],
        );
        layout.focus_pane(panes[2]);
        let infos = layout.panes(area);
        let focused = infos.iter().find(|p| p.is_focused).expect("a focused pane");
        assert_eq!(focused.id, panes[2]);
        assert!(focused.rect.height > 1);
    }

    #[test]
    fn from_saved_repairs_a_stale_active_index() {
        let panes = ids(3);
        let layout = TileLayout::from_saved(
            Node::Stack {
                panes: panes.clone(),
                active: 0,
            },
            panes[1],
        );
        match layout.root() {
            Node::Stack { active, .. } => assert_eq!(*active, 1),
            other => panic!("expected a stack, got {other:?}"),
        }
    }

    #[test]
    fn swap_panes_maintains_stack_active_invariant_when_focused_pane_moves() {
        let area = Rect::new(0, 0, 80, 10);
        let panes = ids(3);
        let mut layout = TileLayout::from_saved(
            Node::Stack {
                panes: panes.clone(),
                active: 0,
            },
            panes[0],
        );

        // Swap the focused pane (at index 0) with pane at index 2
        assert!(layout.swap_panes(panes[0], panes[2]));

        // The focus should still be on panes[0]
        assert_eq!(layout.focused(), panes[0]);

        // But now panes[0] is at index 2 in the stack, so active should be 2
        match layout.root() {
            Node::Stack {
                active,
                panes: members,
            } => {
                assert_eq!(*active, 2);
                assert_eq!(members[2], panes[0]);
            }
            other => panic!("expected a stack, got {other:?}"),
        }

        // The focused pane should have the expanded rect, not a 1-row bar
        let infos = layout.panes(area);
        let focused = infos.iter().find(|p| p.is_focused).expect("a focused pane");
        assert_eq!(focused.id, panes[0]);
        assert!(
            focused.rect.height > 1,
            "focused pane should have expanded rect"
        );
    }

    #[test]
    fn close_focused_returns_to_the_pane_focus_came_from() {
        let mut layout = sample_layout();
        layout.focus_pane(pane(4));

        assert!(layout.close_focused());

        assert_eq!(layout.focused(), pane(2));
    }

    #[test]
    fn close_focused_returns_to_the_pane_that_opened_a_split() {
        // Allocated ids only: sample_layout() uses from_raw and shares the id
        // space with the allocator.
        let (mut layout, first) = TileLayout::new();
        let second = layout.split_focused(Direction::Horizontal);
        let third = layout.split_focused(Direction::Vertical);
        assert_eq!(layout.pane_ids().len(), 3);

        layout.focus_pane(first);
        let opened = layout.split_focused(Direction::Horizontal);
        assert_eq!(layout.focused(), opened);

        assert!(layout.close_focused());

        assert_eq!(layout.focused(), first);
        assert!(layout.pane_ids().contains(&second));
        assert!(layout.pane_ids().contains(&third));
    }

    #[test]
    fn closing_a_background_pane_keeps_the_focused_pane_history() {
        let mut layout = sample_layout();
        layout.focus_pane(pane(4));

        assert!(layout.close_pane(pane(1)));
        assert_eq!(layout.focused(), pane(4));

        assert!(layout.close_focused());
        assert_eq!(layout.focused(), pane(2));
    }

    #[test]
    fn closing_the_remembered_pane_drops_the_focus_history() {
        let mut layout = sample_layout();
        layout.focus_pane(pane(4));

        assert!(layout.close_pane(pane(2)));

        assert!(layout.close_focused());
        assert_eq!(layout.focused(), pane(3));
    }

    #[test]
    fn close_focused_uses_tree_order_without_focus_history() {
        let mut layout = sample_layout();

        assert!(layout.close_focused());

        assert_eq!(layout.focused(), pane(3));
    }

    #[test]
    fn close_focused_does_not_reuse_history_after_it_is_consumed() {
        let mut layout = sample_layout();
        layout.focus_pane(pane(4));

        assert!(layout.close_focused());
        assert_eq!(layout.focused(), pane(2));

        assert!(layout.close_focused());
        assert_eq!(layout.focused(), pane(3));
    }

    #[test]
    fn resize_does_not_disturb_the_close_focus_target() {
        let mut layout = sample_layout();
        layout.focus_pane(pane(4));
        layout.resize_pane(pane(1), NavDirection::Right, 0.05, Rect::new(0, 0, 100, 40));

        assert!(layout.close_focused());

        assert_eq!(layout.focused(), pane(2));
    }

    #[test]
    fn split_pane_leaves_focus_and_history_untouched() {
        let mut layout = sample_layout();
        layout.focus_pane(pane(4));

        let new_id = layout
            .split_pane(pane(1), Direction::Horizontal, 0.5)
            .expect("target exists");

        assert!(layout.pane_ids().contains(&new_id));
        assert_eq!(layout.focused(), pane(4));
        assert!(layout.close_focused());
        assert_eq!(layout.focused(), pane(2));
    }

    #[test]
    fn split_pane_missing_target_changes_nothing() {
        let mut layout = sample_layout();
        let ids = layout.pane_ids();

        assert_eq!(
            layout.split_pane(pane(99), Direction::Horizontal, 0.5),
            None
        );

        assert_eq!(layout.pane_ids(), ids);
    }

    #[test]
    fn insert_pane_near_unfocused_keeps_focus_and_history() {
        let mut layout = sample_layout();
        layout.focus_pane(pane(4));

        assert!(layout.insert_pane_near(pane(1), pane(9), Direction::Horizontal, 0.5, false));

        assert_eq!(layout.focused(), pane(4));
        assert!(layout.close_focused());
        assert_eq!(layout.focused(), pane(2));
    }

    #[test]
    fn failed_split_rollback_preserves_focus_history() {
        let mut layout = sample_layout();
        layout.focus_pane(pane(4));

        let new_id = layout
            .split_pane(layout.focused(), Direction::Horizontal, 0.5)
            .expect("target exists");
        assert!(layout.close_pane(new_id));

        assert_eq!(layout.focused(), pane(4));
        assert!(layout.close_focused());
        assert_eq!(layout.focused(), pane(2));
    }
}
