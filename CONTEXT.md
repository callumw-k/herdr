# Glossary

## Tab

A `Tab` is the unit a user spawns per task or working context (`prefix+c`,
`new_tab`). It holds a tiled layer of `Pane`s and an optional floating layer.
This is Herdr's equivalent of what Zellij calls a "pane" at the
spawn-a-new-context level. Do not conflate it with Herdr's own `Pane`, which
means something narrower.

## Pane

A `Pane` is a single terminal surface: a leaf in the tab's tiled layout
(`Tab::layout`) or a leaf in its floating layout (`Tab::float_layout`). Tiled
versus floating is membership, not a property of the pane: `Tab::is_float`
answers it by looking at the float layout.

## Layer and arrangement

Each layer is a `TileLayout` plus an `Arrangement` (vertical, horizontal,
grid, stacked). A layer is rebuilt under its arrangement whenever a pane is
created, closed or moved (`Tab::reflow`). The float layer defaults to
stacked, so the focused float fills the float region and the others collapse
to single-row bars. Arrangement keys act on whichever layer holds focus
(`AppState::float_layer_has_focus`).

## Floating pane

A pane in `Tab::float_layout`. Floats share the float region
(`ui.floating_pane_width` / `height`) and are laid out inside it under the
float layer's arrangement, so several can be visible at once. The layer can
be hidden without closing anything (`Tab::floats_hidden`); `toggle_floats`
flips it and focusing a tiled pane hides it. `Tab::float_focused` says
whether keyboard focus is in the layer; `Tab::focused_pane` resolves through
it. `Tab::visible_pane_ids` is the one list of panes a client can currently
see and is what input routing and graphics visibility consult.

## Pinned path

`Workspace::pinned_path` is a directory a workspace claims. A new pane whose
cwd falls under a pinned path is routed into that workspace
(`App::auto_move_pane_to_pinned_workspace`), and an idle shell that `cd`s
into one is moved too (`App::reclaim_pane_after_cwd_change`). Declared repos
(`[[repos]]` in `config.toml`) create pinned workspaces at startup.
