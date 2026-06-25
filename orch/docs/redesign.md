# Orch Redesign

## Summary

Rebuild `orch` around one idea: the persisted task record owns intent, and
tmux / worktrees / agents / GitHub / Linear are integrations that reconcile to
that intent. Today the tool infers too much from filenames, tmux session names,
and background caches. The redesign should make lifecycle explicit, keep the
good parts of the current tmux/worktree flow, and make the TUI a real control
surface instead of a thin status list.

Three design rules:

- Separate `desired lifecycle state` from `observed runtime state`.
- Use a durable task identity that is not derived from filename or row order.
- Persist links first, then enrich them in the background. The TUI should never
  hide a PR or Linear ticket just because a cache refresh has not happened yet.

## 1. Data Model

### Core decision

Task identity becomes a durable numeric `task_id`, allocated once and never
reused. Filename, slug, tmux session name, and worktree path all become
attributes of that task rather than its identity.

One note on numbering: "durable ID" and "permanently gap-free numeric ID" are
in tension once closed tasks disappear. This proposal chooses durable task IDs
for identity and fixed bindings, and uses dense row position or hotkeys for
temporary navigation in the TUI. That removes renumber churn without coupling
tmux names to list order.

### Persistence layout

Keep the human-written task markdown files in `~/tasks/`. Replace the current
`.state/*.json` plus `order.json` split with a small registry plus per-task
records:

```text
~/tasks/
├── foo.md
├── bar.md
├── done/
│   └── 12-foo.md
└── .orch/
    ├── registry.json
    ├── tasks/
    │   ├── 12.json
    │   └── 13.json
    ├── cache/
    │   ├── runtime.json
    │   ├── github.json
    │   ├── linear.json
    │   └── lease.json
    ├── inbox/
    └── runs/
```

### `registry.json`

Small, stable, and rarely edited:

- `version`
- `next_task_id`
- `open_order: Vec<TaskId>`
- `closed_order: Vec<TaskId>` for history views only

`open_order` replaces `order.json`. Tmux session names are no longer renamed to
mirror that order.

### Per-task record

Each `tasks/<id>.json` is the source of truth for one task. Suggested fields:

- `id`
- `slug`
- `title`
- `task_file`
- `archived_task_file`
- `created_at`, `started_at`, `paused_at`, `closed_at`, `updated_at`
- `desired_state`
  - `New`
  - `Active`
  - `Paused`
  - `Closed`
- `attention`
  - `needs_input: bool`
  - `last_prompt_from_worker: Option<String>`
- `worktree`
  - `path`
  - `base_ref`
  - `cleanup_on_close: bool`
- `tmux`
  - `session_name`
  - `last_known_pane_id`
  - `pane_titles`
  - `attached: bool` is observed, not authoritative
- `agent`
  - `mode: DirectWorker | Orchestrated`
  - `worker_kind: ClaudeCode`
  - `orchestrator_enabled: bool`
- `links`
  - `prs: Vec<PrLink>`
  - `linear_issues: Vec<LinearLink>`
  - `notes_urls: Vec<String>`
- `drift`
  - `session_missing`
  - `worktree_missing`
  - `cleanup_failed`
  - `last_error`

Important distinction:

- `desired_state` is persisted and user-controlled.
- `runtime_state` is derived from tmux + hook markers + caches.
- `links` are persisted even if enrichment data is stale or missing.

### Link records

PRs and Linear tickets should carry both identity and provenance:

- `id` / `number`
- `repo` or workspace
- `source: Manual | BranchDiscovery | MarkdownScan | Migration`
- `last_verified_at`

This matters because the discovery path will never be perfect. Manual links
must coexist with auto-discovered ones, and migration should preserve existing
associations without pretending they were re-derived.

### Runtime caches

Keep ephemeral data in cache files, not task records:

- `runtime.json`: current tmux/session/pane/busy observations
- `github.json`: PR title, CI, reviews, merge state, updated time
- `linear.json`: issue title, state, assignee, priority, updated time
- `lease.json`: daemon heartbeat

That keeps task records stable and diff-friendly while still letting the TUI
refresh quickly.

## 2. State Machine

### Lifecycle FSM

Persist exactly four lifecycle states:

- `New`: task exists but has not been started
- `Active`: task should have a live worker/session
- `Paused`: task is intentionally parked; no worker should be running
- `Closed`: task is complete and archived

Allowed transitions:

- `start`: `New -> Active`
- `pause`: `Active -> Paused`
- `resume`: `Paused -> Active`
- `close`: `New | Active | Paused -> Closed`

No other transition is implicit. Tmux presence does not change lifecycle on its
own.

### Runtime state

For display, derive a separate runtime badge:

- `Detached`: desired active, but no tmux session seen yet
- `Ready`: session exists, worker process alive, not busy
- `Working`: fresh busy marker says Claude is in a turn
- `Input`: `needs_input` is set
- `Attached`: user is attached to that tmux session
- `Error`: desired state and observed state disagree in a meaningful way

This cleanly absorbs the busy-marker plan from
`docs/busy-detection-plan.md`: busy detection belongs in runtime derivation, not
in lifecycle persistence.

### Working / Ready stability

Today's `tmux capture-pane` + cursor-position detection is unreliable under
Claude Code's reduced-motion mode — the visible frame is stable while Claude
is mid-turn, so orch reports false `Ready`. The redesign adopts the
hook-marker scheme detailed in `docs/busy-detection-plan.md`:

- Claude Code hooks (`UserPromptSubmit`, `Stop`, `SessionEnd`) write/remove a
  per-session marker file under `$XDG_RUNTIME_DIR/orch/busy/<sid>`
- orch reads marker presence + freshness instead of capturing pane content
- Per-session, not per-cwd — multiple Claude sessions in one worktree stay
  independent
- Stale guard: markers older than 30 min are auto-swept on poll + startup

All `tmux capture-pane` calls disappear from orch. `Working` is reported for
the full turn lifetime including API-wait windows; `Ready` flips within one
poll tick after `Stop`. Full design and migration steps in the linked plan.

### Transition side effects

`start`

- Persist `desired_state=Active` first.
- Allocate worktree if missing.
- Create tmux session if missing using `<slug>` as the name (e.g. `infra-triage`).
  The durable `task_id` lives only in orch's record, not in the tmux name —
  this avoids the visual collision between tmux's choose-tree selection
  index `(N)` and an orch-encoded number prefix.
- Spawn worker directly or enqueue orchestrator reconciliation, depending on
  task mode.
- Clear stale drift flags.

`pause`

- Persist `desired_state=Paused` first.
- Kill the worker tmux session, matching today's good pause behavior.
- Keep the worktree on disk.
- Preserve links, notes, and run history.

`resume`

- Persist `desired_state=Active` first.
- Recreate the tmux session from persisted worktree/session bindings.
- Restore the last selected pane if it still exists; otherwise default to pane
  0.
- Re-spawn the worker or notify the orchestrator.

`close`

- Persist `desired_state=Closed` first.
- Kill the tmux session.
- Archive the task markdown into `~/tasks/done/`.
- Remove the worktree if `cleanup_on_close` is true.
- Freeze link history and keep the task record for later inspection.

### Failure handling

External operations can fail after state has been persisted. Do not roll back
the FSM. Instead set drift flags and show them in the TUI:

- `Paused + session still alive` means cleanup is pending
- `Active + no session` means restart is needed
- `Closed + worktree still present` means archive/cleanup drift

For a personal tool this is simpler and safer than trying to build a full job
queue.

### Badge derivation matrix

The runtime badge is derived from `desired_state` plus six observation
inputs. Drift flags are *separate* indicators on the row, not badges.
The matrix is total — every input combination has exactly one row.

Inputs:

- `desired_state`: `New | Active | Paused | Closed`
- `session`: `present | absent` (tmux session for this task exists)
- `worker_alive`: `yes | no | n/a` (`pane_current_command` says claude is
  the foreground process; n/a when session is absent)
- `attached`: `yes | no | n/a` (user is attached to the tmux session;
  n/a when session is absent)
- `needs_input`: `true | false` (orch's `attention.needs_input`)
- `busy_marker`: `fresh | stale_or_absent | n/a`

| desired_state | session | worker_alive | attached | needs_input | busy_marker | -> badge | drift marker |
|---|---|---|---|---|---|---|---|
| `New`    | —       | —   | —   | —     | —              | `New`      | — |
| `Active` | absent  | n/a | n/a | —     | n/a            | `Detached` | `session_missing` |
| `Active` | present | yes | yes | —     | —              | `Attached` | — |
| `Active` | present | yes | no  | true  | —              | `Input`    | — |
| `Active` | present | yes | no  | false | fresh          | `Working`  | — |
| `Active` | present | yes | no  | false | stale/absent   | `Ready`    | — |
| `Active` | present | no  | —   | —     | —              | `Error`    | `worker_dead` |
| `Paused` | absent  | n/a | n/a | —     | n/a            | `Paused`   | — |
| `Paused` | present | —   | —   | —     | —              | `Paused`   | `cleanup_pending` |
| `Closed` | absent  | n/a | n/a | —     | n/a            | (hidden)   | — |
| `Closed` | present | —   | —   | —     | —              | (hidden)   | `cleanup_failed` |

Rules:

- Within `Active` the precedence is `Attached > Input > Working > Ready > Detached`,
  applied automatically by the matrix's row order
- `Error` is the badge whenever orch holds a session whose worker process
  is dead. `R` (resume) re-spawns. The `worker_dead` drift renders alongside
  the badge.
- `Error` is *also* used for explicit operation failures (e.g., spawn
  rejected by the OS); those carry an `error_message` field on the task
  record for the row tooltip
- Drift markers render as a separate glyph next to the badge (e.g., `⚠`),
  never replacing it

### Drift surface partition

Every drift flag has exactly one rendering channel. The one deliberate
exception (`cleanup_failed`) is documented as dual-render.

| Drift flag | Channel | Trigger | Resolution |
|---|---|---|---|
| `session_missing` | per-task row | `Active` task has no tmux session | `R` resume re-creates session |
| `worker_dead` | per-task row | session present, claude process gone | `R` resume re-spawns worker |
| `cleanup_pending` | per-task row | `Paused` task has live session | reconciler kills on next tick |
| `worktree_missing` | per-task row | task record points at a worktree path that doesn't exist on disk | `R` resume re-creates worktree at recorded path |
| `rename_failed` | per-task row + details | atomic rename op partially failed | TUI shows `old_name` / `new_name`; user retries `M` |
| `cleanup_failed` | per-task row **and** stranded-overlay (dual) | `Closed` task's `git worktree remove` failed | per-task: visible in closed-history view; overlay: actionable cleanup row |
| (unbound, clean) | reconciler silent removal | worktree on disk, no live task record, clean working tree | auto-removed on next reconcile (no UI) |
| (unbound, dirty) | stranded-overlay only | worktree on disk, no live task record, dirty/unmerged | overlay row with `inspect / attach / force-remove` keys |

Stranded-overlay channel (key: `W`) is global and only shows directory-level
findings the reconciler discovered that don't have a per-task row. The
dual-rendering of `cleanup_failed` is intentional: the per-task row keeps
state visibility in the task history view, and the overlay surfaces the
actionable directory-level cleanup. No other drift flag dual-renders.

## 3. Key Bindings

### Global

- `q`: quit the TUI; if log/detail overlay is focused, close that first
- `Tab` / `Shift-Tab`: cycle focus between task list, details, and log panes
- `?`: show a compact key help overlay
- `r`: refresh selected task integrations immediately
- `m`: open the message input for the orchestrator or selected worker

### Task list

- `j` / `k`: move selection
- `g` / `G`: jump to top / bottom
- `J` / `K`: reorder open tasks in `open_order`
- `Enter`: attach/switch to the selected task session
- `n`: create a new task (also the prompt on the zero-task screen)
- `s`: start selected `New` task
- `p`: pause selected `Active` task
- `R`: **R**esume selected `Paused` task
- `M`: **M**odify slug — rename the selected task (3-step staged op,
  see §4 tmux)
- `x`: close selected task
- `o`: open the selected external link in the browser
- `W`: open the stranded-worktree overlay (global; reconciler findings
  not bound to any task)

### Details pane

- `h` / `l`: switch detail tabs
  - `Overview`
  - `PRs`
  - `Linear`
  - `Panes`
- `o`: open the highlighted PR or Linear ticket

### In-app tmux pane switching

- `[` / `]`: select previous / next pane in the selected task's tmux session
- `\\`: jump back to the last pane used in that session
- `Enter` from the `Panes` tab: attach to the session with that pane selected

This is the new behavior the current tool is missing. Pane rotation happens
from the TUI by calling tmux directly; the user no longer needs `prefix + o`.

### Log pane

- `j` / `k`: scroll
- `PgUp` / `PgDn`: page scroll
- `g` / `G`: top / bottom
- `Esc`: return focus to the task list

Log lines always wrap. No horizontal scrolling mode.

## 4. Integration Points

### Linear

Single workspace. One linked issue per task is the common case;
the panel mirrors that and stays small. Linear's deeper hierarchy
(initiatives, projects, cycles, parent chains) is intentionally cut: it
doesn't pull weight in the daily orch flow, and the browser is one keystroke
away when richer context is needed.

Auth:

- `LINEAR_API_KEY` env var; absent -> non-fatal "Linear disconnected" badge,
  rest of orch keeps working

Discovery:

- Auto-scan task markdown + slug for issue keys (`[A-Z]+-\d+`)
- Manual `linear add ENG-123` / `linear rm`
- Keys are persisted in `links.linear_issues[]` on the task record; the
  panel always renders from this list, enrichment is best-effort

Display (the whole panel):

- Anchor issue (`*`) at top: key, title, state glyph, assignee
- Direct sub-issues indented one level, same row format
- Multi-link case: stack each anchor + sub-tree top-to-bottom (rare)
- Header shows last-synced age; tints `LOVE` when > 5 min stale
- Empty state: "no Linear issue linked" + how to link
- Disconnected state: render last-known cache as a flat list of linked
  keys + titles

Operations:

- `o` open highlighted in browser
- `r` refresh now
- `j` / `k` walk

That's it. No scope cycling, no project/cycle/chain views, no transition
modal, no comment composer, no multi-link numbered strip. If you need to
write, open in browser.

Mockup:

```
┌─ Linear · ENG-29151 ────────────────────┐
│ * ENG-29151  Fix bene-matching boundary │
│    [progress] In Progress · @ashley     │
│    ├ ENG-30210  Tighten name normalize  │
│    │    [open] Backlog · unassigned     │
│    ├ ENG-30444  Investigate ALERT       │
│    │    [done] Done · @ashley           │
│    └ ENG-30445  Add unit tests          │
│         [progress] In Progress · @agent │
│                                         │
└── synced 47s · `o` open · `r` refresh ──┘
```

Cache:

- `.orch/cache/linear.json`, keyed by issue key
- Per issue: title, state, assignee, updated_at, child_keys
- One batched query per refresh covering linked keys + their immediate
  children
- Refresh: every 2 min in daemon; immediate on task select if > 30s stale

### GitHub / `gh`

The current PR path is too indirect: it derives task PRs from `jj op log`, then
tries to enrich them later. Redesign it as two separate steps.

Discovery:

- Keep manual `pr add` / `pr rm`
- Auto-detect from the task worktree's current branch or tracked bookmark
- Also scan task markdown for pasted PR URLs
- Persist discovered PR numbers in `links.prs` immediately

Enrichment:

- Background loop uses `gh` to fetch title, CI rollup, review state, draft
  state, mergeability, and updated time for every linked PR
- Cache those results in `github.json`

The critical fix is display ownership:

- Task records own the list of linked PRs
- GitHub cache only enriches those PRs
- The TUI renders linked PR rows even if enrichment is stale or absent

That removes the "PR exists but does not appear" failure mode.

Refresh cadence:

- Every 60 seconds in the daemon
- Immediate refresh after `start`, `resume`, `git push`, or manual `r`

### Worktree garbage collection

Today's failure mode: workers (agents) sometimes don't clean up their
worktrees when they exit, so the configured worktree root accumulates orphans.

The redesign makes orch — not the worker — own worktree lifecycle:

- `start` / `resume` create the worktree if missing, bound to the task record
  via `worktree.path`
- `close` runs `git worktree remove`. If it fails (dirty tree, locked
  worktree), set `drift.cleanup_failed=true` and surface in the TUI; never
  silently leave an orphan
- A reconciler runs every N minutes and on TUI startup. It scans the worktree
  parent dir and flags any path not referenced by an open task record
- Orphan policy:
  - clean working tree -> auto-remove on next reconcile
  - dirty / unmerged -> show in a "stranded worktrees" overlay with
    keys to inspect, attach, or force-remove
- Workers must never `git worktree remove` themselves. Their contract ends at
  process exit; orch reconciles from there

This combines with the `Closed + worktree still present` drift flag to
guarantee orphans always surface and can be cleaned from the TUI.

### tmux

Keep today's good lifecycle:

- One tmux session per task
- `Enter` still does attach/switch-client
- Session is created on start/resume and killed on pause/close

What changes:

- Session name is `<slug>` only — no numeric prefix, no embedded id. The
  durable `task_id` lives in orch's task record; tmux's choose-tree
  selection index `(N)` is the only number visible in tmux UI, eliminating
  the today's collision between the two counters.
- Slug collisions: if a new task's slug already exists as a tmux session or
  worktree path, append `-2`, `-3`, … until unique.
- Rename op (`M` in the TUI — *modify* slug; distinct from `R` resume) is
  a three-step staged sequence:

  1. Persist `tmux.rename_in_flight = { old_name, new_name }` to the task record
  2. `tmux rename-session old_name new_name`; on success, set
     `tmux.session_name = new_name`
  3. `git worktree move <old_path> <new_path>`; on success, set
     `worktree.path = new_path` and clear `tmux.rename_in_flight`

  If any step fails, the `rename_in_flight` field stays populated and a
  `rename_failed` drift is shown. The TUI surfaces both `old_name` and
  `new_name` so the user can either retry `M` (re-run from the failed step
  forward, idempotent) or roll back manually. The record always reflects
  reality — partial state is visible, never hidden.
- The TUI records pane IDs/titles and can select panes directly
- Pane switching becomes first-class TUI behavior

Observed tmux state should include:

- session exists?
- attached?
- pane ids and titles
- active pane id
- worker process present?
- `CLAUDE_SESSION_ID` if available, for busy marker correlation

### Claude Code workers and orchestrator

Keep the existing direct worker model and optional orchestrator mode.

Per task:

- `agent.mode=DirectWorker` means `start` and `resume` spawn the worker session
  immediately
- `agent.mode=Orchestrated` means `start` and `resume` only establish the task
  runtime; the orchestrator decides when to fan out workers

Busy detection:

- Adopt the hook-marker design from `busy-detection-plan.md`
- Busy markers feed runtime `Working`, not lifecycle

Messaging:

- Keep the file-backed inbox/runs model because it is simple and inspectable
- Move it under `~/tasks/.orch/inbox` for consistency

## 5. TUI Layout

### Layout

Replace the current expandable flat list with a stable three-pane layout:

- Left workspace: task list
- Right top: selected task details
- Right bottom: activity/log pane

### Left workspace: task list

Each row shows:

- durable `#id`
- task title or slug
- lifecycle/runtime badge
- PR count badge
- Linear count badge
- small drift marker if cleanup or restart is needed

No nested PR rows in the main list. The main list is for task navigation only.
This keeps it dense and stops PR visibility from depending on expand/fold state.

### Right top: details

Tabbed details for the selected task:

- `Overview`: summary, task file, worktree, lifecycle, last activity
- `PRs`: linked PRs with CI/review/codex status
- `Linear`: linked tickets and remote handoff status
- `Panes`: tmux panes, active pane highlight, last selected pane

This is where PRs and Linear tickets become first-class instead of hidden child
rows.

### Right bottom: activity/log pane

Show one stream at a time:

- latest orchestrator run output
- selected worker/session output preview
- system drift/errors

Behavior:

- wrap long lines
- preserve blank lines
- keep scroll position unless the user is already at the bottom

### Focus model

Use explicit focus, visible in the Rosé Pine Dawn palette:

- focused pane gets the stronger highlight
- selected row/item gets `HL_LOW`
- inactive panes stay readable but subdued

This is a better fit for ratatui than modal overlays for everyday use.

### What changes vs. today

- PRs are no longer hidden under expandable task rows
- pane switching becomes in-app
- numbering is decoupled from tmux renaming
- logs are a permanent pane, not a special temporary subview
- the TUI reflects desired lifecycle state and runtime state separately

### Zero-task / first-launch state

The three-pane layout always renders, even with zero tasks:

- left list: `no tasks · n to create`
- right details: `select a task`
- right log: `no activity`

This avoids a separate "empty app" mode and keeps the layout snapshot-stable
across first-run and steady-state.

## 6. Migration Path

### First-run import

On first launch of the redesigned binary:

- read `~/tasks/*.md`
- read legacy `.state/*.json`
- read legacy `order.json`
- inspect live tmux sessions
- inspect existing `.orch/runs`

Then create:

- `registry.json`
- one per-task record for every legacy task

### Idempotent cutover

The data store gets its own versioned subdirectory inside `.orch/`. This
keeps `.orch/runs/` (run inbox/outbox files) untouched by migration, and
makes the cutover a single rename of a fresh subpath.

Layout during and after migration:

```
~/tasks/.orch/
├── runs/                    # untouched by migration; pre-existed
├── store.v2.tmp/            # staging area, deleted on crash recovery
│   ├── registry.json
│   └── tasks/<id>.json
├── store.v2/                # post-cutover authoritative store
│   ├── registry.json
│   └── tasks/<id>.json
└── store.version            # one-line pointer: "v2"; absence = legacy mode
```

Steps and fsync points:

1. Read legacy `.state/*.json` + `order.json` + live tmux + `.orch/runs/`
2. Build the new store under `.orch/store.v2.tmp/`:
   - write each `tasks/<id>.json` and `registry.json`, fsync each file
   - fsync `.orch/store.v2.tmp/` itself (directory entry durability)
3. `rename(.orch/store.v2.tmp, .orch/store.v2)` — atomic on a single
   filesystem; the rename is the cutover point for the data
4. fsync `.orch/` (so the rename is durable)
5. Write `.orch/store.version` containing `v2`, fsync the file, fsync `.orch/`
6. The new reader becomes authoritative *only* when `store.version=v2` is
   readable. Without that marker the reader falls back to legacy `.state/`.

Crash recovery on next launch:

- `store.version` absent or != `v2`: discard any `.orch/store.v2.tmp/`,
  discard any `.orch/store.v2/` (treat as partially-migrated), re-run
  migration from legacy
- `store.version=v2` and `.orch/store.v2/` present: cutover succeeded;
  trust the new store. Legacy `.state/` is read-only fallback for one
  release, then deleted by a follow-up.

Legacy `.state/` files are never modified during migration — they remain
readable across the cutover as the last-resort fallback.

### ID assignment

Allocate task IDs in current visible order:

- first tasks listed in legacy `order.json`
- then any remaining markdown tasks in alphabetical order

This preserves the current mental model as much as possible on day one.

### Legacy field mapping

- `file stem` -> `slug`
- `.state/<name>.json.session` -> `tmux.session_name`
- `.state/<name>.json.worktree` -> `worktree.path`
- `.state/<name>.json.prs` -> `links.prs` with `source=Migration`
- `.state/<name>.json.needs_input` -> `attention.needs_input`
- `.state/<name>.json.paused=true` -> `desired_state=Paused`
- otherwise:
  - if the task has ever been started or has a live session/worktree,
    `desired_state=Active`
  - else `desired_state=New`

### Conflict handling

Migration must not kill in-flight work. If persisted state and live tmux state
disagree:

- prefer preserving the live session
- import the task as `Active`
- set a drift note explaining the mismatch

That keeps the worker reachable and lets the user clean it up intentionally
after the upgrade.

### Compatibility window

For one migration release:

- continue reading legacy `.state/*.json` only if a new task record is missing
- do not write back to legacy files after import
- keep old run directories in place

### Close/archive behavior after migration

Once on the new model:

- closed tasks move their markdown file to `~/tasks/done/<id>-<slug>.md`
  — the `<id>` prefix here is *only* the archive filename (so done/ stays
  unique even with reused slugs); live tmux/worktree names never include id
- their JSON task record remains for history
- new tasks use `<slug>` as the tmux session name and worktree path leaf
  immediately

Existing migrated worktrees can keep their old path until they are closed.
The durable `task_id` is the identity-of-record in orch's task store; tmux
session names and worktree paths are display-friendly slugs that can be
renamed without changing
identity.

## Implementation Notes

- Keep `ratatui` + `crossterm`
- Keep Rosé Pine Dawn colors
- Keep `prelude::*` imports
- Replace the current render tests with `insta` snapshots for the three-pane
  layout and migration fixtures
- Prefer simple JSON files and atomic writes over adding a database

This remains a personal tool. The redesign should optimize for clarity and
recoverability, not for generic multi-user architecture.
