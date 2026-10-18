use std::collections::HashMap;

use ratatui::layout::{Direction, Rect};

use crate::layout::{arrange, Arrangement, Node, PaneId, TileLayout};
use crate::pane::PaneState;
use crate::terminal::TerminalId;
#[cfg(test)]
use crate::terminal::TerminalRuntime;

use super::tab::DetachedPane;

pub struct FloatLayer {
    /// The floating layer's own layout, or None when no floats exist.
    /// `TileLayout::new()` allocates a pane, so an empty layer cannot hold one.
    pub layout: Option<TileLayout>,
    /// Defaults to Stacked so a second float looks as it did before this layer
    /// gained arrangements: one expanded, the rest as collapsed bars.
    pub arrangement: Arrangement,
    /// Hide the whole floating layer without closing anything.
    pub hidden: bool,
    /// When true, keyboard focus is the focused float.
    pub focused: bool,
    /// Separate from the tiled layer's flag because a re-flow rebuilds a layer
    /// under its arrangement and so discards manual sizing in it; a float
    /// mutation must not cost the tiled layer its dragged borders, nor the
    /// other way round.
    pub needs_reflow: bool,
    pub panes: HashMap<PaneId, PaneState>,
    #[cfg(test)]
    pub runtimes: HashMap<PaneId, TerminalRuntime>,
}

impl Default for FloatLayer {
    fn default() -> Self {
        Self {
            layout: None,
            arrangement: Arrangement::Stacked,
            hidden: false,
            focused: false,
            needs_reflow: false,
            panes: HashMap::new(),
            #[cfg(test)]
            runtimes: HashMap::new(),
        }
    }
}

impl FloatLayer {
    pub fn contains(&self, pane_id: PaneId) -> bool {
        self.layout
            .as_ref()
            .is_some_and(|layout| layout.pane_ids().contains(&pane_id))
    }

    /// The floating layer's panes, in tree order.
    pub fn pane_ids(&self) -> Vec<PaneId> {
        self.layout
            .as_ref()
            .map(TileLayout::pane_ids)
            .unwrap_or_default()
    }

    /// The float that holds focus within the layer, or None when the layer is
    /// hidden. Replaces the old `top_float`: without a z-order there is no top.
    pub fn focused_float(&self) -> Option<PaneId> {
        if self.hidden {
            return None;
        }
        self.layout.as_ref().map(TileLayout::focused)
    }

    pub fn has_focus(&self) -> bool {
        self.focused && !self.hidden
    }

    pub fn visible_pane_ids(&self) -> Vec<PaneId> {
        if self.hidden {
            Vec::new()
        } else {
            self.pane_ids()
        }
    }

    pub fn terminal_id(&self, pane_id: PaneId) -> Option<&TerminalId> {
        self.panes
            .get(&pane_id)
            .map(|pane| &pane.attached_terminal_id)
    }

    pub fn push(&mut self, pane_id: PaneId, pane_state: PaneState) {
        match self.layout.as_mut() {
            Some(layout) => {
                let target = layout.focused();
                // Placement here is provisional: the re-flow below rebuilds the
                // layer under the arrangement. insert_pane_near declines when
                // `pane_id` is already in the layer (a re-push, matching the old
                // dedup-on-push behaviour) or when `target` is missing, which
                // can't happen since `target` is the layer's own focus. Only the
                // second case would orphan a live PTY in `self.panes` with no
                // slot in the tree, so fall back to a stack entry rather than
                // trust that invariant blindly.
                if !layout.insert_pane_near(target, pane_id, Direction::Vertical, 0.5, true)
                    && !layout.pane_ids().contains(&pane_id)
                {
                    let mut ids = layout.pane_ids();
                    ids.push(pane_id);
                    let active = ids.len() - 1;
                    *layout = TileLayout::from_saved(Node::Stack { panes: ids, active }, pane_id);
                }
            }
            None => {
                self.layout = Some(TileLayout::from_saved(Node::Pane(pane_id), pane_id));
            }
        }
        self.panes.insert(pane_id, pane_state);
        self.hidden = false;
        self.focused = true;
        self.needs_reflow = true;
    }

    /// Unlike `Tab::detach_pane`, there is no last-pane guard: closing the final float
    /// is valid because the tiled layer still exists.
    pub fn close(&mut self, pane_id: PaneId) -> Option<DetachedPane> {
        let layout = self.layout.as_mut()?;
        if !layout.pane_ids().contains(&pane_id) {
            return None;
        }
        let previous = layout.focused();
        layout.focus_pane(pane_id);
        if layout.close_focused() {
            if previous != pane_id {
                layout.focus_pane(previous);
            }
        } else {
            // close_focused refuses the last pane, so the layer is now empty.
            self.layout = None;
            self.focused = false;
        }
        let pane = self.panes.remove(&pane_id)?;
        self.needs_reflow = true;
        Some((pane_id, pane.attached_terminal_id))
    }

    pub fn focus(&mut self) -> bool {
        if self.layout.is_none() || (self.focused && !self.hidden) {
            return false;
        }
        self.hidden = false;
        self.focused = true;
        true
    }

    pub fn set_hidden(&mut self, hidden: bool) -> bool {
        if self.hidden == hidden {
            return false;
        }
        self.hidden = hidden;
        if hidden {
            self.focused = false;
        } else if self.layout.is_some() {
            self.focused = true;
        }
        true
    }

    /// `region` is None when it cannot be resolved, which leaves the layer
    /// dirty so it re-flows once the terminal is big enough to hold one.
    pub fn reflow(&mut self, region: Option<Rect>) {
        if !self.needs_reflow {
            return;
        }
        let Some((panes, focus)) = self
            .layout
            .as_ref()
            .map(|layout| (layout.pane_ids(), layout.focused()))
        else {
            self.needs_reflow = false;
            return;
        };
        let Some(region) = region else {
            return;
        };
        self.needs_reflow = false;
        if let Some(root) = arrange(self.arrangement, &panes, focus, region) {
            self.layout = Some(TileLayout::from_saved(root, focus));
        }
    }

    pub fn focus_pane(&mut self, pane_id: PaneId) -> bool {
        let Some(layout) = self.layout.as_mut() else {
            return false;
        };
        if !layout.pane_ids().contains(&pane_id) {
            return false;
        }
        layout.focus_pane(pane_id);
        self.hidden = false;
        self.focused = true;
        true
    }

    pub fn absorb(&mut self, other: FloatLayer) {
        let Some(other_layout) = other.layout.as_ref() else {
            return;
        };
        let other_ids = other_layout.pane_ids();
        match self.layout.as_ref() {
            None => {
                self.layout = other.layout;
                self.arrangement = other.arrangement;
                self.hidden = other.hidden;
                self.needs_reflow = other.needs_reflow;
            }
            Some(layout) => {
                let focus = layout.focused();
                let mut ids = layout.pane_ids();
                let active = ids.iter().position(|id| *id == focus).unwrap_or(0);
                ids.extend(other_ids);
                self.layout = Some(TileLayout::from_saved(
                    Node::Stack { panes: ids, active },
                    focus,
                ));
                self.needs_reflow = true;
            }
        }
        self.panes.extend(other.panes);
        #[cfg(test)]
        self.runtimes.extend(other.runtimes);
    }

    pub fn take_all(&mut self) -> Vec<DetachedPane> {
        let ids = self.pane_ids();
        let mut taken: Vec<DetachedPane> = ids
            .into_iter()
            .filter_map(|id| {
                self.panes
                    .remove(&id)
                    .map(|pane| (id, pane.attached_terminal_id))
            })
            .collect();
        taken.extend(
            self.panes
                .drain()
                .map(|(id, pane)| (id, pane.attached_terminal_id)),
        );
        #[cfg(test)]
        self.runtimes.clear();
        self.layout = None;
        self.focused = false;
        taken
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pushed(count: usize) -> (FloatLayer, Vec<PaneId>) {
        let mut layer = FloatLayer::default();
        let ids: Vec<_> = (0..count).map(|_| PaneId::alloc()).collect();
        for id in &ids {
            layer.push(*id, PaneState::new(TerminalId::alloc()));
        }
        (layer, ids)
    }

    #[test]
    fn a_layer_starts_empty_and_stacked() {
        let layer = FloatLayer::default();
        assert!(layer.layout.is_none());
        assert_eq!(layer.arrangement, Arrangement::Stacked);
        assert!(layer.pane_ids().is_empty());
    }

    #[test]
    fn pushing_the_first_float_creates_the_layout() {
        let (layer, ids) = pushed(1);
        assert_eq!(layer.pane_ids(), ids);
        assert_eq!(layer.focused_float(), Some(ids[0]));
        assert!(layer.focused);
        assert!(layer.contains(ids[0]));
    }

    #[test]
    fn pushing_more_floats_keeps_them_all_in_order() {
        let (layer, ids) = pushed(3);
        assert_eq!(layer.pane_ids(), ids);
        assert_eq!(layer.focused_float(), Some(ids[2]));
    }

    #[test]
    fn focused_float_is_none_while_the_layer_is_hidden() {
        let (mut layer, _) = pushed(1);
        layer.set_hidden(true);
        assert_eq!(layer.focused_float(), None);
        assert!(layer.visible_pane_ids().is_empty());
    }

    #[test]
    fn closing_a_float_removes_it_and_keeps_the_others() {
        let (mut layer, ids) = pushed(3);
        assert!(layer.close(ids[1]).is_some());
        assert_eq!(layer.pane_ids(), vec![ids[0], ids[2]]);
        assert!(layer.layout.is_some());
    }

    #[test]
    fn closing_the_last_float_clears_the_layout_and_focus() {
        let (mut layer, ids) = pushed(1);
        assert!(layer.close(ids[0]).is_some());
        assert!(layer.layout.is_none());
        assert!(!layer.focused);
        assert!(layer.pane_ids().is_empty());
        assert!(!layer.panes.contains_key(&ids[0]));
    }

    #[test]
    fn closing_a_pane_that_is_not_a_float_returns_none() {
        let (mut layer, _) = pushed(1);
        assert!(layer.close(PaneId::alloc()).is_none());
    }

    #[test]
    fn push_with_duplicate_id_does_not_duplicate_stack_entry() {
        let (mut layer, ids) = pushed(1);
        let updated_terminal = TerminalId::alloc();
        layer.push(ids[0], PaneState::new(updated_terminal.clone()));

        assert_eq!(layer.pane_ids(), ids);
        assert_eq!(layer.terminal_id(ids[0]), Some(&updated_terminal));
    }

    #[test]
    fn focus_does_nothing_without_a_float() {
        let mut layer = FloatLayer::default();

        assert!(!layer.focus());
        assert!(!layer.focused);
    }

    #[test]
    fn focus_unhides_and_focuses_an_existing_float() {
        let (mut layer, _) = pushed(1);
        layer.set_hidden(true);

        assert!(layer.focus());

        assert!(!layer.hidden);
        assert!(layer.focused);
        assert!(layer.has_focus());
    }

    #[test]
    fn focus_does_nothing_when_already_focused() {
        let (mut layer, _) = pushed(1);

        assert!(!layer.focus(), "already focused, nothing to do");
    }

    #[test]
    fn hiding_the_layer_clears_float_focus() {
        let (mut layer, _) = pushed(1);
        assert!(layer.focused);

        layer.set_hidden(true);

        assert!(!layer.focused);
    }

    #[test]
    fn showing_the_layer_restores_float_focus() {
        let (mut layer, ids) = pushed(1);
        layer.set_hidden(true);

        layer.set_hidden(false);

        assert!(layer.focused);
        assert_eq!(layer.focused_float(), Some(ids[0]));
    }

    #[test]
    fn reflow_rebuilds_the_layer_under_its_arrangement() {
        let region = Rect::new(10, 5, 40, 10);
        let (mut layer, ids) = pushed(3);
        layer.arrangement = Arrangement::Stacked;
        layer.needs_reflow = true;

        layer.reflow(Some(region));

        assert!(!layer.needs_reflow);
        assert_eq!(layer.pane_ids(), ids, "float order survives a re-flow");
        let root = layer.layout.as_ref().expect("a float layout").root();
        assert!(matches!(root, Node::Stack { .. }));
    }

    #[test]
    fn reflow_keeps_focus_on_the_same_pane() {
        let region = Rect::new(10, 5, 40, 10);
        let (mut layer, ids) = pushed(3);
        let focused = ids[1];
        layer
            .layout
            .as_mut()
            .expect("a float layout")
            .focus_pane(focused);
        layer.arrangement = Arrangement::Grid;
        layer.needs_reflow = true;

        layer.reflow(Some(region));

        assert_eq!(layer.focused_float(), Some(focused));
    }

    #[test]
    fn reflow_without_a_region_leaves_the_layer_alone() {
        let (mut layer, ids) = pushed(2);
        layer.needs_reflow = true;

        layer.reflow(None);

        assert_eq!(
            layer.pane_ids(),
            ids,
            "no region means no float geometry to apply"
        );
        assert!(layer.needs_reflow);
    }

    #[test]
    fn an_unresolvable_region_keeps_the_layer_dirty() {
        let region = Rect::new(10, 5, 40, 10);
        let (mut layer, _) = pushed(2);
        layer.arrangement = Arrangement::Vertical;

        layer.reflow(None);
        assert!(layer.needs_reflow);

        layer.reflow(Some(region));
        assert!(!layer.needs_reflow);
        let root = layer.layout.as_ref().expect("a float layout").root();
        assert!(matches!(root, Node::Split { .. }));
    }

    #[test]
    fn absorbing_into_an_empty_layer_takes_the_other_layers_shape() {
        let mut layer = FloatLayer::default();
        let mut other = FloatLayer::default();
        let a = PaneId::alloc();
        other.push(a, PaneState::new(TerminalId::alloc()));
        other.arrangement = Arrangement::Grid;
        other.hidden = true;
        layer.absorb(other);
        assert_eq!(layer.pane_ids(), vec![a]);
        assert_eq!(layer.arrangement, Arrangement::Grid);
        assert!(layer.hidden);
        assert!(layer.panes.contains_key(&a));
    }

    #[test]
    fn absorbing_into_a_populated_layer_appends_and_keeps_its_focus_and_arrangement() {
        let mut layer = FloatLayer::default();
        let (a, b) = (PaneId::alloc(), PaneId::alloc());
        layer.push(a, PaneState::new(TerminalId::alloc()));
        layer.needs_reflow = false;
        let mut other = FloatLayer::default();
        other.push(b, PaneState::new(TerminalId::alloc()));
        other.arrangement = Arrangement::Grid;
        layer.absorb(other);
        assert_eq!(layer.pane_ids(), vec![a, b]);
        assert_eq!(layer.layout.as_ref().map(TileLayout::focused), Some(a));
        assert_eq!(layer.arrangement, Arrangement::Stacked);
        assert!(layer.needs_reflow);
    }

    #[test]
    fn absorbing_an_empty_layer_changes_nothing() {
        let (mut layer, ids) = pushed(2);
        layer.needs_reflow = false;
        layer.absorb(FloatLayer::default());
        assert_eq!(layer.pane_ids(), ids);
        assert!(!layer.needs_reflow);
    }

    #[test]
    fn take_all_empties_the_layer_and_clears_focus() {
        let mut layer = FloatLayer::default();
        let (a, b) = (PaneId::alloc(), PaneId::alloc());
        layer.push(a, PaneState::new(TerminalId::alloc()));
        layer.push(b, PaneState::new(TerminalId::alloc()));
        let taken = layer.take_all();
        assert_eq!(taken.iter().map(|(id, _)| *id).collect::<Vec<_>>().len(), 2);
        assert!(layer.layout.is_none() && layer.panes.is_empty() && !layer.focused);
    }

    #[test]
    fn focus_pane_unhides_and_focuses_a_member_only() {
        let mut layer = FloatLayer::default();
        let (a, b) = (PaneId::alloc(), PaneId::alloc());
        layer.push(a, PaneState::new(TerminalId::alloc()));
        layer.push(b, PaneState::new(TerminalId::alloc()));
        layer.set_hidden(true);
        assert!(layer.focus_pane(a));
        assert_eq!(layer.focused_float(), Some(a));
        assert!(layer.has_focus());
        assert!(!layer.focus_pane(PaneId::alloc()));
    }
}
