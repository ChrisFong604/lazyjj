# lazyjj

`lazyjj` is a terminal UI for [Jujutsu](https://jj-vcs.github.io/jj/latest/) with the interaction model of `lazygit`: persistent panels, fast keyboard navigation, one-key common actions, and a diff viewer that stays front and center.

This repository now contains a working first slice, not full parity with `lazygit` yet.

## Current features

- Multi-panel TUI built with `ratatui`
- Repository discovery from the current working directory
- Live views for:
  - changed files
  - revision stack
  - bookmarks
  - operation log
  - command output
- Styled diff viewer with:
  - file headers
  - commit metadata
  - hunk highlighting
  - added/removed/context line coloring
  - scroll support
  - live addition/removal counters
- Interactive actions:
  - inspect selected file, revision, bookmark, or operation
  - describe a revision
  - create a new change
  - create a bookmark
  - move a bookmark
  - squash a revision into another revision
  - abandon a revision with confirmation
  - undo the last operation
  - restore an operation from the op log
  - run arbitrary `jj` subcommands

## Keyboard shortcuts

- `Tab` / `Shift+Tab`: cycle focus
- `1`-`6`: jump to a panel
- `j` / `k` or arrows: move selection
- `Enter`: inspect selected item
- `J` / `K`: scroll diff
- `g` / `G`: jump to top/bottom
- `r`: refresh
- `d`: describe selected revision
- `n`: create new change
- `b`: create bookmark
- `m`: move selected bookmark
- `s`: squash selected revision
- `a`: abandon selected revision
- `u`: undo last operation
- `o`: restore selected operation
- `p`: run arbitrary `jj ...`
- `?`: toggle help
- `q`: quit

## Running

```bash
cargo run
```

Run it from inside any `jj` repository.

## Install Globally

Use the install script from the project root:

```bash
./install.sh
```

It installs `lazyjj` as a global Cargo binary. Equivalent manual command:

```bash
cargo install --path /Users/chris/Code/lazyjj --locked
```

## JJ mappings

`lazygit` concepts do not map 1:1 onto Jujutsu. `lazyjj` currently uses these JJ-native equivalents:

- branches -> bookmarks
- commit amend / reword -> `jj describe`
- stash-like undo safety -> `jj undo` and `jj op restore`
- interactive history edits -> `jj squash`, `jj abandon`, `jj new`
- reflog-style recovery -> operation log

## Parity roadmap

The architecture is meant to grow into a real `lazygit`-class client. Major pieces still missing:

- configurable keybindings and theme files
- side-by-side diff mode and syntax-aware intra-line diffing
- interactive split/diffedit workflows
- revset search/filter UI
- remote operations and fetch/push/pull flows
- merge/conflict resolution helpers
- workspaces/worktrees view
- commit graph rendering
- custom command presets and context menus
- richer modal workflows instead of single-line prompts
- tests around command parsing and UI state transitions

## Design intent

The diff panel is the core of the app. The current version emphasizes readability and density without copying `less` or `git diff` verbatim. The next iteration should push that further with:

- side gutters
- code-aware word diffing
- inline statistics per file
- better file-to-file transitions
- theme presets inspired by modern editor palettes
