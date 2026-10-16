use std::collections::HashSet;

use super::{Tab, Workspace};

impl Workspace {
    pub(super) fn remove_tab_at(&mut self, idx: usize) -> Tab {
        let tab = self.tabs.remove(idx);
        for child in &mut self.tabs {
            if child.parent == Some(tab.number) {
                child.parent = tab.parent;
            }
        }
        tab
    }

    /// For a freshly created tab only: it moves this tab and none of its descendants.
    pub(crate) fn attach_tab_to_parent(&mut self, tab_idx: usize, parent_number: usize) -> usize {
        let Some(number) = self.tabs.get(tab_idx).map(|tab| tab.number) else {
            return tab_idx;
        };
        let Some(parent_idx) = self.tabs.iter().position(|tab| tab.number == parent_number) else {
            return tab_idx;
        };
        if parent_number >= number {
            return tab_idx;
        }
        self.tabs[tab_idx].parent = Some(parent_number);
        let insert_idx = self.subtree_end(parent_idx);
        self.move_tab(tab_idx, insert_idx);
        self.tabs
            .iter()
            .position(|tab| tab.number == number)
            .unwrap_or(tab_idx)
    }

    fn subtree_end(&self, root_idx: usize) -> usize {
        let mut members = HashSet::from([self.tabs[root_idx].number]);
        let mut end = root_idx + 1;
        while let Some(tab) = self.tabs.get(end) {
            match tab.parent {
                Some(parent) if members.contains(&parent) => {
                    members.insert(tab.number);
                    end += 1;
                }
                _ => break,
            }
        }
        end
    }

    pub fn tab_display_name_with_parent(&self, tab_idx: usize) -> Option<String> {
        let name = self.tab_display_name(tab_idx)?;
        let parent_name = self.tabs[tab_idx]
            .parent
            .and_then(|parent| self.tabs.iter().position(|tab| tab.number == parent))
            .and_then(|parent_idx| self.tab_display_name(parent_idx));
        Some(match parent_name {
            Some(parent) => format!("{parent} › {name}"),
            None => name,
        })
    }
}

pub(crate) fn retain_valid_tab_parents(tabs: &mut [Tab]) {
    let numbers: HashSet<usize> = tabs.iter().map(|tab| tab.number).collect();
    for tab in tabs.iter_mut() {
        if tab
            .parent
            .is_some_and(|parent| parent >= tab.number || !numbers.contains(&parent))
        {
            tab.parent = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Workspace;
    use super::*;

    type Removal = (&'static str, fn(&mut Workspace, usize));

    fn parents(ws: &Workspace) -> Vec<Option<usize>> {
        ws.tabs.iter().map(|tab| tab.parent).collect()
    }

    fn numbers(ws: &Workspace) -> Vec<usize> {
        ws.tabs.iter().map(|tab| tab.number).collect()
    }

    fn index_of(ws: &Workspace, number: usize) -> usize {
        ws.tabs
            .iter()
            .position(|tab| tab.number == number)
            .expect("tab should exist")
    }

    fn add_child(ws: &mut Workspace, parent_number: usize) -> usize {
        let idx = ws.test_add_tab(None);
        let number = ws.tabs[idx].number;
        ws.attach_tab_to_parent(idx, parent_number);
        number
    }

    fn chain() -> (Workspace, usize) {
        let mut ws = Workspace::test_new("family");
        let middle = add_child(&mut ws, 1);
        add_child(&mut ws, middle);
        (ws, middle)
    }

    #[test]
    fn second_child_lands_after_the_first_childs_descendants() {
        let mut ws = Workspace::test_new("family");
        ws.test_add_tab(None);
        let first = add_child(&mut ws, 1);
        let grandchild = add_child(&mut ws, first);
        let second = add_child(&mut ws, 1);

        assert_eq!(numbers(&ws), vec![1, first, grandchild, second, 2]);
        ws.assert_invariants_for_test();
    }

    #[test]
    fn placing_a_child_keeps_the_active_tab() {
        let mut ws = Workspace::test_new("family");
        ws.test_add_tab(None);
        ws.switch_tab(1);

        add_child(&mut ws, 1);

        assert_eq!(ws.tabs[ws.active_tab].number, 2);
    }

    #[test]
    fn every_removal_path_reparents_children_to_the_grandparent() {
        let removals: [Removal; 4] = [
            ("close_tab", |ws, idx| assert!(ws.close_tab(idx))),
            ("remove_pane", |ws, idx| {
                let pane = ws.tabs[idx].root_pane;
                assert!(!ws.remove_pane(pane));
            }),
            ("close_pane", |ws, idx| {
                let pane = ws.tabs[idx].root_pane;
                assert!(!ws.close_pane(pane));
            }),
            ("take_pane_for_move", |ws, idx| {
                let pane = ws.tabs[idx].root_pane;
                assert!(ws.take_pane_for_move(pane).is_some());
            }),
        ];
        for (name, remove) in removals {
            let (mut ws, middle) = chain();
            let idx = index_of(&ws, middle);

            remove(&mut ws, idx);

            assert_eq!(parents(&ws), vec![None, Some(1)], "{name}");
        }
    }

    #[test]
    fn closing_a_root_promotes_its_children() {
        let (mut ws, middle) = chain();

        assert!(ws.close_tab(0));

        assert_eq!(parents(&ws), vec![None, Some(middle)]);
        ws.assert_invariants_for_test();
    }

    #[test]
    fn invalid_parents_are_dropped() {
        let mut ws = Workspace::test_new("family");
        ws.test_add_tab(None);
        ws.test_add_tab(None);
        ws.tabs[0].parent = Some(2);
        ws.tabs[1].parent = Some(1);
        ws.tabs[2].parent = Some(9);

        retain_valid_tab_parents(&mut ws.tabs);

        assert_eq!(parents(&ws), vec![None, Some(1), None]);
    }

    #[test]
    #[should_panic(expected = "invalid parent")]
    fn invariants_reject_a_parent_that_is_not_older() {
        let mut ws = Workspace::test_new("family");
        ws.test_add_tab(None);
        ws.tabs[0].parent = Some(2);

        ws.assert_invariants_for_test();
    }

    #[test]
    #[should_panic(expected = "invalid parent")]
    fn invariants_reject_a_parent_that_no_longer_exists() {
        let mut ws = Workspace::test_new("family");
        ws.test_add_tab(None);
        ws.test_add_tab(None);
        let removed = ws.tabs.remove(1);
        for pane in removed.layout.pane_ids() {
            ws.public_pane_numbers.remove(&pane);
        }
        ws.tabs[1].parent = Some(2);

        ws.assert_invariants_for_test();
    }

    #[test]
    fn adversarial_identity_state_keeps_parent_invariants_through_closes() {
        let mut ws = Workspace::test_adversarial_identity_state();
        let root = ws.tabs[0].number;
        let child = add_child(&mut ws, root);
        add_child(&mut ws, child);
        let idx = index_of(&ws, child);

        assert!(ws.close_tab(idx));

        ws.assert_invariants_for_test();
    }

    #[test]
    fn display_name_with_parent_follows_the_parents_current_name() {
        let mut ws = Workspace::test_new("family");
        ws.tabs[0].set_custom_name("web".into());
        let child = add_child(&mut ws, 1);
        let idx = index_of(&ws, child);
        ws.tabs[idx].set_custom_name("api".into());

        assert_eq!(
            ws.tab_display_name_with_parent(idx).as_deref(),
            Some("web › api")
        );
        ws.tabs[0].set_custom_name("site".into());
        assert_eq!(
            ws.tab_display_name_with_parent(idx).as_deref(),
            Some("site › api")
        );
        assert_eq!(ws.tab_display_name_with_parent(0).as_deref(), Some("site"));
    }
}
