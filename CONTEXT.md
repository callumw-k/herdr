# Glossary

## Tab

A `Tab` is the unit a user spawns per task or working context (`prefix+c`,
`new_tab`). It contains a tiled layout of `Pane`s (via splits) plus an
optional floating layer. This is herdr's equivalent of what tools like
Zellij call a "pane" at the spawn-a-new-context level — do not conflate it
with herdr's own `Pane` (see below), which means something narrower.

## Pane

A `Pane` is a single terminal surface: either a cell in a `Tab`'s tiled
split layout, or a member of that `Tab`'s floating layer (`Tab::floats`).
A pane's tiled-vs-floating status is not a property of the pane itself —
it is purely membership in `Tab::layout` vs `Tab::floats`.

## Floating pane

A `Pane` whose id is in `Tab::floats` rather than part of the tiled
`Tab::layout`. Floating panes stack (`Vec<PaneId>`, back to front); the
topmost is the only one rendered/focusable at a time. The floating layer
as a whole can be hidden (`Tab::floats_hidden`) without closing any panes
in it.

"A floating pane is open" means visible: `Tab::floats` is non-empty *and*
`Tab::floats_hidden` is false (equivalently, `Tab::top_float()` is
`Some`). A tab with floats that are all hidden does not count as having
an open floating pane.
