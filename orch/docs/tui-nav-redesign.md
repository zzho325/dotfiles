# TUI Navigation Redesign

Supersedes the keymap section of `redesign.md` §3 and the focus model in
`redesign.md` §5. Layout (three panes, four detail tabs) is unchanged.

## 1. Diagnosis

The current `tui3.rs` has three nested focus levels and overloads keys
across them:

```
Level 1: Pane focus       List ──Tab──▶ Details ──Tab──▶ Log ──Tab──▶ List
Level 2: Detail tab       Overview ──l──▶ PRs ──l──▶ Linear ──l──▶ Panes   (only when Details focused)
Level 3: tmux pane row    [ ──prev── ]  ── \ jump-active                   (only when Panes tab + Details focused)
```

Same keys mean different things at different levels:

| Key | List focus | Details focus | Log focus |
|---|---|---|---|
| `j`/`k` | move task selection | (no-op) | scroll log |
| `Enter` | attach session | attach pane (Panes tab only) | (no-op) |
| `g`/`G` | top/bot of list | (no-op) | top/bot of log |

The user has to remember "which level am I on" to predict what `j` does.
Focus is signalled only by a header text-color shift (MUTED → LOVE), no
border/separator/edge-ruler. From the screenshot the user couldn't tell.

Plus: `s p R x M n W o` are advertised in `?` but not wired — discoverable
keys that silently no-op are worse than absent ones.

## 2. New focus model

Two zones, not three. The Log is a viewer, not a focusable peer.

```
┌──────────────┬─────────────────────────────────────────┐
│              │                                         │
│   LIST       ▎  RIGHT  (Overview·PRs·Linear·Panes)     │
│   zone A     ▎  zone B  ── content of the active tab   │
│              ▎                                         │
│              ├─────────────────────────────────────────┤
│              │  log: passive ─ scroll-only via PgUp/Dn │
│              │                                         │
└──────────────┴─────────────────────────────────────────┘
```

- **Tab** toggles between zone A (list) and zone B (active detail tab).
  Two-state cycle, not three. No `Shift-Tab` needed; reuse `Tab`.
- **Number keys 1·2·3·4** jump straight to a detail tab from anywhere
  *and* move focus to zone B. No need to first Tab to Details, then `l l l`.
- **Log is not in the focus cycle.** It scrolls via `PgUp`/`PgDn` from
  any focus. Auto-follows tail unless the user scrolled.
- **`Esc`** always returns focus to zone A (list) and resets to Overview.
  One stable home base.

Visual indication is unambiguous:

- The focused zone gets a 1-col `▎` rule on its left edge in `LOVE`.
- The unfocused zone's text drops one shade (TEXT → SUBTLE) so brightness
  itself signals focus.
- A solid `│` divider between list and right column (currently absent).
- Selected row keeps `HL_LOW` background regardless of focus, so the user
  can navigate back to it.

## 3. Complete keymap

`Δ` flags changes vs. tui3.rs ~L1010-1140.

### Global (work from any focus)

| Key | Action | Δ |
|---|---|---|
| `q` / `Esc`-from-list | quit | same |
| `?` | toggle help overlay | same |
| `Tab` | toggle focus: list ↔ right | **was 3-cycle list→details→log** |
| `1` `2` `3` `4` | jump to Overview·PRs·Linear·Panes (and focus right) | **new** |
| `r` | refresh integrations for selected task | same |
| `m` | message orchestrator about selected task | same |
| `Esc` | from right zone → focus list; from list → quit | **was: only quit** |
| `PgUp` / `PgDn` | scroll log (always — no need to focus log) | **was: only when log focused** |
| `<` / `>` | log: scroll-to-top / tail-follow | **replaces log-focus `g/G`** |

### List zone (focus = list)

| Key | Action | Δ |
|---|---|---|
| `j` / `k` / `↓` / `↑` | move selection | same |
| `g` / `G` | top / bottom of list | same |
| `J` / `K` | reorder open tasks | same (Phase 1F) |
| `Enter` | attach to selected task's **active pane** | clarified |
| `n` | new task | (Phase 1F) |
| `s` | start (New → Active) | (Phase 1F) |
| `p` | pause (Active → Paused) | (Phase 1F) |
| `R` | resume (Paused → Active) | (Phase 4a) |
| `x` | close | (Phase 1F) |
| `M` | modify slug | (Phase 4a) |
| `o` | open external link | (Phase 4a) |
| `W` | stranded-worktree overlay | (Phase 1F) |

### Right zone (focus = right). Active tab determines what `j/k/Enter` do.

| Key | Overview | PRs | Linear | Panes |
|---|---|---|---|---|
| `j`/`k` | (no-op) | move PR cursor | move issue cursor | move pane cursor |
| `Enter` | (no-op) | open PR in browser | open issue in browser | **attach to that pane** |
| `o` | (no-op) | open PR | open issue | (no-op) |
| `r` | refresh task | refresh `gh` | refresh Linear | refresh tmux panes |

Notes on right-zone:

- `h`/`l` are **removed** from tab switching. Use `1·2·3·4`. h/l were the
  worst overload — they only worked when right was focused, and the user
  had no on-screen cue that told them to press Tab first.
- `[ ] \` are **removed** from the global Panes flow. Pane navigation is
  just `j/k` inside the Panes tab now, which matches the rest of the right
  zone.

### Message-input modal

| Key | Action |
|---|---|
| any printable | append to buffer |
| `Backspace` | delete |
| `Enter` | send |
| `Esc` | cancel, drop buffer |

Unchanged from current tui3.rs.

## 4. Attach semantics (the central simplification)

Today there are two attach paths and the user has to remember which one
needs which focus level. The new model collapses them:

```
flow                                 keystrokes
──────────────────────────────────── ─────────────────────────────────
attach to task (active pane)         Enter                         (1)
attach to a specific pane            4 → j/k to row → Enter        (3-4)
```

`Enter` from the list **always** means "drop me into the live work" —
i.e., attach to the active pane in the task's tmux session. If the user
wants a specific non-active pane, they explicitly go to the Panes tab
first, which makes the second `Enter` unambiguous (it's the row cursor,
not the task cursor).

Pane navigation inside the Panes tab is just `j/k` — same key, same
direction, same target type as the list. No `[ ] \`, no extra mental
model. The "last pane" affordance moves to Phase 5 (persisted
`last_known_pane_id`).

## 5. Visual mockups

### A. Default — list focused

```
┌─────────────────┐│  Overview · PRs · Linear · Panes
▎ tasks           ││  ───────────────────────────────────────────
▎─────────────────││
▎ ▸ #1 infra-tri… ││   title:    infra-triage
▎   #2 ach-saniti…││   status:   working
▎   #3 fresh-task ││   session:  task-infra-triage
                  ││   worktree: ~/column/task-infra-triage
                  ││   prs:      #25163
                  │├─────────────────────────────────────────────
                  ││ log: 2026-04-30T14:22…  ·done
                  ││ ─────────────────────────────────────────────
                  ││  [scan] starting
                  ││  checking 3 tasks
                  ││  infra-triage: working
└─────────────────┘└─────────────────────────────────────────────
```

- Left edge `▎` ruler in LOVE on the list zone.
- Right zone text in SUBTLE (one shade darker than focused). Tab-bar tabs
  all in SUBTLE except the active one (TEXT, not LOVE — focus is
  elsewhere).
- Solid `│` divider between zones.

### B. Right focused, Panes tab active (after pressing `4`)

```
┌─────────────────┐│  Overview · PRs · Linear · ▎Panes
  tasks           ││  ───────────────────────────────────────────
  ─────────────── ││
    #1 infra-tri… ││   ▸ ● %1   claude
    #2 ach-saniti…││     · %2   jj
    #3 fresh-task ││
                  ││   Enter attach    j/k navigate
                  │├─────────────────────────────────────────────
                  ││ log: …
                  ││ ─────────────────────────────────────────────
                  ││  …
└─────────────────┘└─────────────────────────────────────────────
```

- List ruler gone, list text in SUBTLE.
- Right edge ruler `▎` in LOVE on the right zone (drawn just left of the
  tab bar's first letter, and continued down the tab body).
- Active tab gets a small `▎` underline-equivalent next to the label
  (`▎Panes`), making it obvious which tab is live without color alone.
- Selected pane row uses cursor `▸` + `HL_LOW` background.

### C. Help overlay (compact, only shows wired keys)

```
┌─ key bindings ─────────────────────────────────────┐
│                                                    │
│  Global                                            │
│    q          quit                                 │
│    Tab        toggle list ↔ right                  │
│    1 2 3 4    Overview · PRs · Linear · Panes      │
│    Esc        focus list                           │
│    PgUp/PgDn  scroll log     < / >  top / tail     │
│    ?          this overlay                         │
│    r          refresh                              │
│    m          message                              │
│                                                    │
│  List                                              │
│    j k g G    move · top / bottom                  │
│    Enter      attach to active pane                │
│                                                    │
│  Right zone                                        │
│    j k        move cursor in active tab            │
│    Enter      open/attach in active tab            │
│                                                    │
│  Phase 1F+:  n s p R x M J K o W                   │
│  (greyed — see redesign-notes Phase 1F/4a)         │
└────────────────────────────────────────────────────┘
```

Unimplemented keys appear once at the bottom in `MUTED`, listed but not
described. Pressing one shows a single-line toast in the log header:

```
log: not yet wired — Phase 4a (M = modify slug)
```

This keeps muscle memory honest without lying about what works.

## 6. Worked flows

### Flow 1 — "I just opened orch and want to check the PRs on task #2"

```
state                             keys     result
────────────────────────────────  ───────  ──────────────────────────
launch, list focused, #1 selected (init)
move to #2                        j        list cursor on #2
jump to PRs tab                   2        right zone focus, PRs tab
                                           shows linked PRs for #2
move to a specific PR             j        PR cursor on row 2
open in browser                   Enter    `gh pr view --web` fires
back to list to pick another      Esc      focus returns to list
```

4 keys to glance, 5 to open. Old flow needed Tab → Tab → l → j → Enter
(also 5, but with two implicit focus levels to track).

### Flow 2 — "Switch to the second tmux pane in the active task"

```
state                             keys     result
────────────────────────────────  ───────  ──────────────────────────
list focused, target task is #1   (start)
jump to Panes tab                 4        right focus, Panes tab
move to second pane row           j        row cursor on %2
attach                            Enter    tmux switch-client + select-pane
```

3 keys. Old flow was Tab → l → l → l → ] → Enter (6 keys, with the
mid-flow risk of being on the wrong focus level).

### Flow 3 — "Send a message to the orchestrator about task #3"

```
state                             keys     result
────────────────────────────────  ───────  ──────────────────────────
list focused                      (start)
move to #3                        j j      cursor on #3
open message input                m        modal opens
type message                      …text…
send                              Enter    written to .orch/inbox
```

`m` is global, works from any zone. Same as before — kept because the
user already has muscle memory for it.

### Flow 4 — "Quickly skim the log of the running task without losing my place"

```
state                             keys     result
────────────────────────────────  ───────  ──────────────────────────
list focused, on #2               (start)
scroll log up                     PgUp     log scrolls, focus stays
                                           on list, list still navigable
keep paging                       PgUp     …
return to tail                    >        log re-pins to bottom
move to #1                        k        list cursor moves —
                                           never had to switch focus
```

This is the big ergonomic win. In the current tool, scrolling log
required Tab→Tab to log focus, then `j`/`k`/PgUp, then Tab→Tab back.
Now log is always-available without claiming the focus.

### Flow 5 — "Resume a paused task and watch it come up"

```
state                             keys     result
────────────────────────────────  ───────  ──────────────────────────
list focused, #2 is paused        (start)
move to #2                        j        cursor on #2
resume                            R        FSM transition (Phase 4a)
                                           — until then: toast
                                           "not yet wired — Phase 4a"
attach to active pane             Enter    drops into tmux
```

Pre-Phase 4a, the user gets honest feedback that `R` isn't wired yet,
not silent emptiness.

## 7. What to delete from tui3.rs

The redesign removes more than it adds. Concrete code paths that should
be deleted/simplified once this lands:

```
fn handle_log_key             ── obsolete: log is no longer a focus zone.
                                 PgUp/PgDn/`<`/`>` handled in handle_key
                                 directly. Saves ~30 LOC.

KeyCode::BackTab arm           ── obsolete: Tab is a 2-state toggle, no
                                 reverse direction needed. -8 LOC.

Pane::Log variant              ── delete from `enum Pane`. Simplifies all
                                 `match app.focus` arms. -1 variant ×
                                 ~5 sites.

Three-arm dispatch in
handle_key (List/Details/Log)  ── collapses to two: list vs right.
                                 The right arm dispatches by detail_tab,
                                 not by a separate Pane variant.

KeyCode::Char('h')|Left,
KeyCode::Char('l')|Right
in handle_details_key          ── tab cycling moves to global `1`/`2`/
                                 `3`/`4`. -10 LOC and the user no longer
                                 needs to remember h/l semantics.

KeyCode::Char('['), `]`, '\\'
in handle_details_key          ── pane cursor uses j/k like everything
                                 else. -25 LOC.

Help overlay listing for
unimplemented keys             ── moved to a single-line "Phase 1F+"
                                 footer. Removes the lying section that
                                 listed s/p/R/x/M/n/W/o as if they
                                 worked.

Pane focus signalled only by
header text color (LOVE/MUTED) ── replaced by left-edge `▎` ruler +
                                 SUBTLE/TEXT brightness shift +
                                 explicit `│` divider. Same render
                                 surface, fewer pixels of confusion.
```

Net diff is roughly **-80 LOC, +30 LOC** in `tui3.rs` plus an updated
help overlay.

## 8. Migration of existing tests

Snapshot tests that need updating (named per `redesign-notes.md` Phase 3):

- `snapshot_three_pane_log_focus` — **delete**. There's no log focus
  state to snapshot.
- `snapshot_three_pane_list_focus` / `_details_focus` — keep both;
  rename `details_focus` → `right_focus`.
- `snapshot_panes_tab_focus_indicator` — keep; should now show the
  right-edge ruler + active-tab `▎` marker.
- `snapshot_key_help_overlay` — rewrite to match §5C above.
- `tab_cycling_next_prev` (h/l test) — **delete**. Replace with
  `tab_jump_by_number` covering `1·2·3·4`.
- `pane_focus_cycles` (3-state Tab cycle) — rewrite as 2-state toggle.
- `pane_switching_brackets` — **delete**. Replace with
  `panes_tab_jk_navigation` using `j/k`.

## 9. Why this and not alternatives

- **Why not a command palette (`:` / `Ctrl-P`)?** Personal tool, single
  user, single screen. The keymap is small enough to memorise. A palette
  adds a discovery surface the user doesn't need.
- **Why not modal vim-style (`i`/`v`/normal)?** Same — overkill for the
  task surface. Two zones don't need modes.
- **Why number keys 1-4 instead of `H/L` for tabs?** `H/L` shadow
  shift-h/shift-l (which mean "top/bottom of viewport" in vim) and the
  user already uses capital `J/K` for reorder. Numbers are unambiguous,
  jump directly, and visually align with the four-tab strip.
- **Why is log not in the focus cycle?** The user's primary log
  interaction is "skim while keeping my place". Forcing focus
  acquisition turns a glance into a 4-keystroke side-trip. Making
  PgUp/PgDn always-available costs nothing — they don't conflict with
  any other binding.
- **Why keep `Tab` instead of dropping it for just numbers?** Tab is the
  one binding everyone already has muscle memory for. Keeping it as a
  pure two-state toggle preserves that without the three-cycle
  confusion.
