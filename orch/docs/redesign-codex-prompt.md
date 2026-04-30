# Orch Redesign — Codex Brief

Read the current implementation:
- `~/dotfiles/orch/src/main.rs`
- `~/dotfiles/orch/src/tui.rs`
- `~/dotfiles/orch/src/state.rs`
- `~/dotfiles/orch/src/runs.rs`
- `~/dotfiles/orch/src/gh.rs`
- `~/dotfiles/orch/src/cache.rs`
- `~/dotfiles/orch/docs/busy-detection-plan.md`
- `~/dotfiles/orch/agents/` and `~/dotfiles/orch/skills/` (if any)

## Goal

Propose a redesign of orch — a personal Rust ratatui task orchestrator that
manages worktrees, tmux panes, and Claude Code worker agents. The current
implementation has evolved organically; redesign from first principles while
keeping what works.

## Keep (today's behavior is good)

- **Worktree lifecycle:** create per task, clean up on close
- **Tmux pane lifecycle:** today's start / attach pattern
- **Optional orchestrator agent:** can spawn workers autonomously when desired

## Codify (currently ad-hoc, make first-class)

- **Task state machine:** explicit `start → pause → resume → close` transitions
  with a single source of truth for state. Today start/close happens frequently
  and pause/resume isn't really a thing — the redesign should make this a
  proper FSM with persisted state.

## New requirements

- **Linear integration:** show Linear tickets related to a task in the TUI.
  User hands off some tasks to remote agents via Linear; orch should surface
  the linked tickets per task.
- **PR detection that surfaces:** current PR-show logic is broken — PRs don't
  appear in the TUI. Redesign the detection + display path.
- **Stable task numbering:** no gaps, removed tasks should not cause renumbering;
  IDs are durable.
- **Log line wrapping:** don't truncate, wrap.
- **In-app pane switching:** keybind to switch between task panes inside the
  TUI itself — no need to drop to `tmux prefix+o` to navigate.

## Output

Write a `DESIGN.md`-style document covering:

1. **Data model** — task struct, state, persistence format
2. **State machine** — transitions, side effects (worktree/tmux/agent ops)
3. **Key bindings** — full keymap including in-app pane switching
4. **Integration points** — Linear (auth, query, refresh cadence), gh (PR
   detection that works), tmux (pane lifecycle), Claude Code (worker spawn)
5. **TUI layout** — panes, focus model, what changes vs. today
6. **Migration path** — how to move from today's state file to the new model
   without losing in-flight tasks

**Do not write code.** This is a proposal only — the user will review and
decide what to implement.

## Style notes

- Match existing orch conventions: ratatui + crossterm, Rust 2024, Rosé Pine
  Dawn palette, `prelude::*` import, snapshot tests via `insta`.
- Prefer simplicity — this is a personal tool for one user, not a product.

## Output file

Save the redesign proposal to `~/dotfiles/orch/docs/redesign.md`.
