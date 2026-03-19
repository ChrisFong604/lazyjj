# AGENTS.md

## Project Brief

`lazyjj` is a Rust terminal UI for Jujutsu (`jj`) with `lazygit` as the product reference, but not as a Git-centric implementation constraint.

Current code layout:

- `src/main.rs`: thin entrypoint
- `src/app.rs`: app state, event loop, action dispatch, terminal lifecycle
- `src/jj.rs`: all `jj` command execution and parsing
- `src/model.rs`: shared domain types
- `src/ui.rs`: rendering only

The product goal is a fast, keyboard-first JJ client with a strong diff experience, clear panel layout, and recoverable workflows via JJ’s operation log.

## Core Rules

- Keep the architecture close to model/update/view. State lives in app/model types, mutations happen in update/action code, and rendering stays in `ui.rs`.
- Do not put repository I/O, subprocess execution, parsing, or mutation logic inside rendering code.
- Do not parse colored terminal output from `jj`. Use machine-friendly templates where possible, and keep `--color=never --no-pager` on internal command execution.
- Prefer JJ-native concepts over forcing Git metaphors. Use bookmarks, revsets, op log, and restore/undo workflows directly.
- The diff viewer is a first-class feature. Changes that reduce diff readability, navigation, or performance need strong justification.

## Rust Best Practices

- Prefer `Result`-based error propagation with context over panics in normal control flow. Use `anyhow::Context` on fallible boundaries so failures are actionable.
- Use `expect` only for invariants that are truly impossible in production and where the message explains the invariant.
- Keep data structures explicit and boring. Favor small typed structs/enums over ad hoc tuples and stringly typed state.
- Derive standard traits when they add value (`Debug`, `Clone`, `PartialEq`, `Eq`, `Default`) and keep names aligned with Rust conventions.
- Keep functions focused:
  - parsing functions parse
  - command functions execute commands
  - UI functions render
  - action functions coordinate state changes
- Avoid hidden global state. Thread state through `App`, explicit parameters, or dedicated structs.
- Minimize allocation and cloning in hot paths, especially per-frame rendering and event handling.
- When introducing concurrency or async work later, isolate it behind explicit message passing. Do not smear synchronization primitives across the UI code.
- New public or cross-module APIs should follow Rust API Guidelines naming and trait conventions.

## TUI Best Practices

- Always restore terminal state correctly. Alternate screen and raw mode must be exited on normal shutdown and on panic paths.
- Treat rendering as a pure projection of current state. No side effects in draw code.
- Avoid blocking the event loop with long-running subprocesses if the feature is expected to be interactive. If a command may take noticeable time, move toward background execution and render progress explicitly.
- Keep selection state and scroll state persistent and explicit. Prefer proper stateful widgets or dedicated state structs over recomputing viewport state ad hoc.
- Prefer composable widgets and panel helpers over one giant render function.
- Make focus and selection obvious. Keyboard-first navigation must remain consistent across panels.
- Support narrow terminals gracefully. Avoid layouts that collapse into unreadable noise.
- Avoid visual churn:
  - stable panel ordering
  - stable keybindings
  - consistent colors for additions, removals, metadata, and focus
- If adding mouse support, it must remain additive. Keyboard interaction stays primary.
- If spawning external tools or editors, always restore the terminal first and reinitialize cleanly after the child process exits.

## Testing And Verification

Before finishing substantive changes, run as many of these as the change warrants:

```bash
cargo fmt
cargo check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Additional expectations:

- Add unit tests for parsing logic in `src/jj.rs` when templates or output parsing changes.
- Add state-transition tests when key handling or action dispatch gets more complex.
- Prefer Ratatui `TestBackend` and snapshot-style rendering tests for non-trivial UI regressions.
- For terminal lifecycle changes, manually verify startup, quit, and panic/early-error cleanup behavior.

## Project-Specific Guidance

- Keep all JJ subprocess behavior centralized in `src/jj.rs`. Do not scatter `Command::new("jj")` calls around the codebase.
- If a `jj` command or template is brittle, document the assumption near the parser.
- Do not silently discard stderr from JJ commands. Surface enough detail in the command log or status area to debug failures.
- Preserve a clean separation between:
  - repository snapshot loading
  - command execution
  - UI state mutation
  - rendering
- Prefer adding typed actions/messages over wiring new key handlers directly to ad hoc logic.
- When introducing richer widgets, graduate repeated manual list/scroll logic to proper Ratatui stateful widget usage.
- Treat panic handling and terminal cleanup as product requirements, not polish.

## UX Bar

- `lazyjj` should feel fast, legible, and intentional.
- Diff presentation should remain stylish but practical:
  - strong contrast
  - clear file/hunk hierarchy
  - stable scrolling
  - no decorative noise that competes with code
- Every shortcut added should either save real time or unlock a meaningful JJ workflow.
- Avoid copying `lazygit` blindly where JJ offers a better mental model.

## References

These sources informed the rules above and should be preferred when extending the project:

- Rust error handling: <https://doc.rust-lang.org/book/ch09-02-recoverable-errors-with-result.html>
- Rust API Guidelines: <https://rust-lang.github.io/api-guidelines/checklist.html>
- Clippy documentation: <https://doc.rust-lang.org/clippy/>
- Cargo test documentation: <https://doc.rust-lang.org/cargo/commands/cargo-test.html>
- Ratatui application structure (TEA): <https://ratatui.rs/concepts/application-patterns/the-elm-architecture/>
- Ratatui widgets and stateful widgets: <https://docs.rs/ratatui/latest/ratatui/widgets/>
- Crossterm terminal/raw mode guidance: <https://docs.rs/crossterm/latest/crossterm/terminal/index.html>
- Ratatui panic hook guidance: <https://ratatui.rs/recipes/apps/panic-hooks/>
- Ratatui terminal/event handler guidance: <https://ratatui.rs/recipes/apps/terminal-and-event-handler/>
- Ratatui testing guidance: <https://ratatui.rs/recipes/testing/>
- Ratatui snapshot testing guidance: <https://ratatui.rs/recipes/testing/snapshots/>
