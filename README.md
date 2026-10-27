# Cerdr

<p align="center">
  <img src="assets/logo.png" alt="cerdr" width="100" />
</p>

Cerdr is my fork of [herdr](https://github.com/herdrdev/herdr), the terminal runtime for coding agents. It tracks upstream `master` and adds floating panes, tab arrangements, workspaces that follow your directories, nested tabs, a reworked agent overview and a quieter look.

The binary, CLI, config file and socket API are all still called `herdr`, so [herdr's docs](https://herdr.dev/docs/) cover everything this page doesn't.

## install

Cerdr isn't packaged, so build it from source:

```bash
git clone https://github.com/callumw-k/herdr cerdr
cd cerdr
scripts/build_release.sh
```

> [!WARNING]
> `scripts/build_release.sh` fully stops every running herdr session before it installs the new binary. Every pane in those sessions exits, agents and shells included. herdr restores the saved layouts and resumes supported agent sessions on the next start, but the original processes don't survive. Finish or park running work first.

The script runs in this order:

1. `just build` compiles a release binary into `target/release/herdr`. If the build fails, the script exits here and nothing is stopped.
2. `herdr session list --json` finds every running session, and `herdr session stop` stops each one. This needs `jq` on your `PATH`.
3. The old binary is removed and `target/release/herdr` is copied to `~/.local/bin/herdr`. Pass a path to install somewhere else, for example `scripts/build_release.sh ~/bin/herdr`.

Run `herdr` afterwards to start the new build.

## what's different

### arrangements

Every tab lays its panes out under one arrangement: `vertical`, `horizontal`, `grid` or `stacked`.

- `prefix+]` and `prefix+[` cycle a tab through them. The tab is rebuilt from its panes in order, so a full lap brings back the layout you started with. Copy mode moves from `prefix+[` to `prefix+u` to make room.
- `prefix+v` and `prefix+minus` switch the tab to even columns or even rows and add a pane. Pressing `prefix+v` twice gives three even columns, not a split nested inside a split.
- `prefix+enter` adds a pane in the arrangement the tab already has.
- New tabs start in `grid`. Set `ui.default_arrangement` to change it.
- A stacked tab shows the focused pane at full size and collapses the rest to titled bars you can click. Panes that don't fit fold into a `+N more` bar.
- A border you drag holds until the next pane is created or closed, then the arrangement's even sizing takes over. `layout.apply` in the socket API still builds an exact tree.

Every pane has a border title. A manual name wins, then the agent's terminal title, then the foreground process, the working directory and finally the pane number. A lone pane keeps a one-row title strip.

### floating panes

Each workspace has one floating layer that sits above its tabs and stays put when you switch between them.

| Action | Key |
| --- | --- |
| New floating pane | `prefix+f` |
| Open, focus or hide the floating layer | `prefix+ctrl+f` |
| Show or hide the floating layer | `prefix+shift+f` |

`prefix+ctrl+f` opens a float when the workspace has none, focuses the layer when it isn't focused, and hides it when it is. None of these keys close a float.

The layer has its own arrangement, stacked by default. The arrangement, split and `prefix+enter` keys act on whichever layer holds focus, and `prefix+h/j/k/l` and `prefix+tab` move between floats once the layer is focused.

Floats draw a thick frame in the theme's orange and take keyboard focus, clicks, scrolling and selection. They survive a server restart with their scrollback and get agent detection like any other pane. Closing a tab never closes a float, and closing the workspace does.

Set the size with `ui.floating_pane_width` and `ui.floating_pane_height`, in cells or as a percentage like `"60%"`. From the CLI, `herdr pane float` opens one and `herdr tab floats-toggle` shows or hides the layer.

### workspaces that follow your directories

A workspace can be pinned to a directory with `prefix+.` or `herdr workspace set-path <workspace_id> <path>`. When a pane sitting at a shell prompt changes directory into a pinned path, it moves into that workspace. A pane that's busy moves once it goes idle.

Repos declared in config each own a workspace, created the first time a pane enters one:

```toml
[[repos]]
path = "~/code/herdr"

[[repos]]
path = "~/code/projects"
children = true
```

With `children = true` the entry is a projects folder, and each immediate child gets its own workspace without being listed. Deeper declarations win, so an explicit entry for a worktree beats its parent folder. `prefix+>` declares or undeclares the active workspace's directory.

Shells that never emit OSC 7, bash included, still report their directory, so this works in those panes too.

### nested tabs

`herdr tab create` run from inside a pane makes the new tab a child of that pane's tab. It opens next to its siblings, reads `parent › child` in the tab bar, sidebar and window title, and its agents nest under the parent's in the agents sidebar. `--parent <tab_id>` picks the parent and `--no-parent` creates a top-level tab. Closing a parent hands its children to the grandparent.

### agent overview

`prefix+g` opens a centred agent overview with search focused, in place of herdr's goto list. On wide terminals a live preview of the selected pane sits on the right, in the pane's own colours.

- It lists only panes with a detected agent by default. `alt+a` in search, or `a` in tree mode, shows everything, and the choice is remembered per machine.
- Results lead with the tab name. Matches on tab, pane and agent names, agent kinds and titles rank above matches on workspace, branch, path or pane id.
- `ctrl+j`/`ctrl+k` and `ctrl+n`/`ctrl+p` move the selection, `enter` opens the pane, and `ctrl+x` closes it while keeping the overview open.
- `tab` switches between search and tree mode. In tree mode `p` edits the selected workspace's pinned path, `ctrl+o` creates a workspace with a name and path, and `h`/`l` jump between workspaces.
- `esc` clears the search, then closes. `prefix+g` again closes it too, and `F1` lists every key for the current mode.

`ui.navigator_on_start = true` opens it when the client attaches, and `ui.navigator_agents_only` sets the starting filter.

### navigation

- `ui.agent_panel_follows_workspace = true` limits the agent panel, and the next, previous and numbered agent jumps, to the focused workspace. The overview still lists every agent.
- `keys.next_tab_global` and `keys.previous_tab_global` step through tabs across every workspace. They're unset by default.

### sidebar and tab bar

- Agent state shows as a coloured ribbon down the left of each space and agent entry, so the default rows drop `state_icon`. The blocked dot pulses while an agent is waiting on you. `ui.status_pulse = false` stops it.
- The `activity` token shows the pane's terminal title, name or agent, whichever is set first, and is now the second row of the default agent entry. The `index` token shows an entry's number for the numbered jumps, and `{ text = "..." }` places literal text in a row.
- `ui.sidebar.spaces.divider = true` draws a rule between top-level spaces.
- The collapsed sidebar sizes its sections to their contents and ends an overflowing section with a `+N` count.
- `ui.tab_bar_status = true` shows each tab's agent status before its label. `ui.tab_bar_workspace = true` shows the focused workspace's name at the left of the tab row.
- `ui.pane_borders = "quiet"` draws borders only between split panes, with a gap between them, and names panes only inside stacks. `ui.tab_bar_style = "quiet"` pads and fills every tab and shows its pane count.

## docs

The fork's changes are written up in `docs/next/website/src/content/docs/`, mainly `keyboard.mdx`, `concepts.mdx`, `configuration.mdx` and `cli-reference.mdx`. For everything else, see [herdr.dev/docs](https://herdr.dev/docs/).

## agent instructions

If you're an AI agent working in this repository, read [`AGENTS.md`](./AGENTS.md) before making changes.

## development

```bash
git clone https://github.com/callumw-k/herdr cerdr
cd cerdr
cargo build --release

just test        # unit tests
just check       # formatting, tests, and maintenance checks
```

## license

Cerdr, like herdr, is licensed under the [Apache License 2.0](LICENSE).
