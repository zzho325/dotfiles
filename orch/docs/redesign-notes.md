# Orch Redesign — Implementation Notes

## Phased plan

Big bang rewrite is risky for a daily-use tool. Slice into phases that each
ship independently and keep orch usable. Every phase that touches the TUI
ships its own `insta` snapshot tests so a review agent can inspect behavior
from snapshots without running the binary.

Snapshot fixtures use a shared base set of three tasks for the focus / tab /
log variants so snapshots only differ on the dimension under test:

```
fixture::base_three_tasks
  #1 "tm-cb-rules"        Active + Ready    no drift
  #2 "infra-triage"       Active + Working  no drift  (selected)
  #3 "agentserver"        Paused            cleanup_pending drift
fixture::base_overview     Overview tab populated for #2
fixture::base_log_lines    5 long lines that must wrap
```

### Phase 0 — Stability quick wins

Pre-redesign fixes on the current codebase. **No data model change,
no session-name change** (the rename is in Phase 4a once the new store
can record naming atomically).

Worktree GC:

- [ ] `orch gc` subcommand: scans `~/column/worktrees/`, lists worktrees
      not bound to any task in `.state/*.json`, prompts to remove
- [ ] Auto-run reconcile on TUI startup; orphans appear as warnings
- [ ] Make `close` retry `git worktree remove` and warn (not silently swallow)
      on failure
- [ ] Workers no longer attempt their own worktree cleanup

Working/Ready stability (busy-detection-plan.md):

- [ ] Claude Code hooks for `UserPromptSubmit`/`Stop`/`SessionEnd` that
      write/remove `$XDG_RUNTIME_DIR/orch/busy/<sid>` markers
- [ ] orch reads marker freshness; rip out `tmux capture-pane` + pane-hash
- [ ] 30-min stale sweep on poll + startup
- [ ] Validate against `prefersReducedMotion` mid-turn (the failure case)

Snapshot deliverables (Phase 0 — current-code TUI, pre-redesign):

- [ ] `snapshot_p0_status_ready` — current TUI row, fixture: active task,
      worker alive, no marker
- [ ] `snapshot_p0_status_working` — current TUI row, fixture: active task,
      fresh marker
- [ ] `snapshot_p0_orphan_warning` — current TUI startup warning for
      unbound clean worktrees (P0 surface only; Phase 1 replaces this with
      auto-removal + stranded overlay for dirty cases)

**Lands on:** existing code, ~300-500 LOC total.

### Phase 1 — Persistence + FSM foundation

Goal: replace `.state/` with new store so everything else can build on it.

Data store:

- [ ] `.orch/store.v2/registry.json` + `.orch/store.v2/tasks/<id>.json`
- [ ] **Idempotent cutover** per `redesign.md` §6: stage under
      `.orch/store.v2.tmp/`, fsync files + dir, atomic rename to
      `.orch/store.v2/`, fsync `.orch/`, then write `.orch/store.version=v2`
      and fsync. New reader becomes authoritative only when `store.version=v2`
      exists. Partial migration on crash leaves remnants that are discarded
      on next launch — no double ID allocation.
- [ ] Migration reader: `.state/*.json` + `order.json` + live tmux + existing
      `.orch/runs/`, IDs assigned in current visible order

Task record fields:

- [ ] Persisted `desired_state` (`New / Active / Paused / Closed`)
- [ ] Reserve pane fields (P5 prep): `tmux.last_known_pane_id`,
      `tmux.pane_titles`, `tmux.active_pane_id`. Phase 1 writes them
      opportunistically; restore-on-resume waits until Phase 5
- [ ] Drift flags: `session_missing`, `worker_dead`, `cleanup_pending`,
      `cleanup_failed`, `worktree_missing`, `rename_failed`

Derivation:

- [ ] `runtime_state` derived per `redesign.md` §2 *Badge derivation
      matrix*. Includes new `worker_alive` input from
      `pane_current_command` check
- [ ] Stranded-worktree reconciler implementing the partition table:
      auto-remove clean unbound worktrees silently; populate the
      stranded overlay with dirty unbound worktrees
- [ ] Stranded-overlay UI bound to `W` (rendered in current TUI shell;
      Phase 3 redoes the shell)

Snapshot deliverables (Phase 1 — task list / status / drift surfaces):

- [ ] Status badges (each fixture pins all six matrix inputs:
      `desired_state`, `session`, `worker_alive`, `attached`, `needs_input`,
      `busy_marker`):
  - [ ] `snapshot_status_detached` —
        `Active / absent / n/a / n/a / false / n/a`
  - [ ] `snapshot_status_attached` —
        `Active / present / yes / yes / false / stale_or_absent`
  - [ ] `snapshot_status_input` —
        `Active / present / yes / no / true / stale_or_absent`
  - [ ] `snapshot_status_working` —
        `Active / present / yes / no / false / fresh`
  - [ ] `snapshot_status_ready` —
        `Active / present / yes / no / false / stale_or_absent`
  - [ ] `snapshot_status_error_worker_dead` —
        `Active / present / no / n/a / false / n/a`
  - [ ] `snapshot_status_paused` —
        `Paused / absent / n/a / n/a / false / n/a`
- [ ] Per-task drift markers (each fixture is one drift flag in isolation,
      everything else clean):
  - [ ] `snapshot_drift_session_missing` — `Active` task, `session=absent`
        (this is also the canonical "task list row with drift glyph"
        rendering; no separate snapshot needed)
  - [ ] `snapshot_drift_cleanup_pending` — `Paused` task, `session=present`
  - [ ] `snapshot_drift_cleanup_failed_per_task` — `Closed` task whose
        worktree remove failed; visible in closed-history view (per-task
        channel of the dual-render case)
  - [ ] `snapshot_drift_worktree_missing` — `Active` task, recorded
        worktree path absent on disk
- [ ] Stranded overlay (key: `W`):
  - [ ] `snapshot_stranded_overlay_dirty` — fixture: 2 unbound dirty
        worktrees with `inspect / attach / force-remove` affordances
  - [ ] `snapshot_stranded_overlay_cleanup_failed` — fixture: 1 entry
        from a `Closed` task whose `git worktree remove` failed (the
        overlay channel of the dual-render case; per-task channel
        covered above)
  - (clean unbound case is silent — no snapshot needed)
- [ ] Task list row variants (use `fixture::base_three_tasks` with one row
      altered per snapshot): `snapshot_task_row_new`, `_active_ready`,
      `_active_working`, `_active_input`, `_paused`, `_attached`

**Lands on:** new branch. Migration tested against a fixture legacy store
before any cutover code merges.

### Phase 2 — PR display layer

- [ ] `links.prs[]` persisted on task record, sourced from branch + manual
- [ ] `gh` cache enriches (title, CI, review state) but never gates display
- [ ] PR list always renders linked PRs even when cache is empty/stale
- [ ] PR-discovery scan on branch change

(No new TUI surface in Phase 2 — the PRs tab itself is part of Phase 3's
shell; Phase 2 just makes sure the data layer is ready when Phase 3 wires
it up. The `snapshot_detail_tab_prs` test lands in Phase 3 against this
data layer.)

### Phase 3 — TUI rewrite + minimal Linear render

Phase 3 owns the new TUI shell. Linear *rendering* is here too, fed by a
fixture-driven cache stub so snapshots can land without any Linear API code.
Real Linear data layer is Phase 4b.

- [ ] Three-pane layout: list / details / log
- [ ] Detail tabs: Overview · PRs · Linear · Panes
- [ ] Log lines wrap; scroll position preserved when not at bottom
- [ ] In-app tmux pane switching: `[`, `]`, `\`
- [ ] Linear tab renders from `links.linear_issues[]` + a fixture cache
      stub. Anchor (`*`) + indented sub-issues; `o` open, `r` refresh
      (no-op until Phase 4b)
- [ ] Zero-task screen (Q4): three-pane layout intact; list `no tasks · n
      to create`; details `select a task`; log `no activity`
- [ ] Key help overlay (`?`)
- [ ] Focus model: `LOVE` for selected, `HL_LOW` for focused pane,
      subdued elsewhere (Rosé Pine Dawn)

Snapshot deliverables (Phase 3 — TUI shell):

- [ ] Layout with focus variants (all reuse `fixture::base_three_tasks`):
  - [ ] `snapshot_three_pane_base` — no overlays, default focus on list
  - [ ] `snapshot_three_pane_list_focus`
  - [ ] `snapshot_three_pane_details_focus`
  - [ ] `snapshot_three_pane_log_focus`
- [ ] Detail tabs (selected task = #2 with full fixture content):
  - [ ] `snapshot_detail_tab_overview` — fixture (full Overview field set):
        - `title`: "Fix bene-matching boundary"
        - `slug`: "infra-triage"
        - `task_id`: 2
        - `desired_state`: `Active`
        - `runtime_badge`: `Working`
        - `worktree.path`: `~/column/worktrees/infra-triage`
        - `tmux.session_name`: `infra-triage`
        - `agent.mode`: `DirectWorker`
        - `links.prs`: 2
        - `links.linear_issues`: 1
        - `created_at`: 3 days ago
        - `started_at`: 2 days ago
        - `last_activity`: "2 min ago"
        - `task_file`: `~/tasks/infra-triage.md`
  - [ ] `snapshot_detail_tab_prs` — fixture: 2 PRs (one fully enriched
        with CI green + 1 review; one link-only/stale)
  - [ ] `snapshot_detail_tab_linear` — fixture: 1 anchor + 3 children
        (rendered from stub cache; data layer not yet wired)
  - [ ] `snapshot_detail_tab_panes` — fixture: 3 panes named `worker`,
        `jj-log`, `claude2`; second pane is active
- [ ] Log pane:
  - [ ] `snapshot_log_wrapped_lines` — fixture: 5 long lines mixed with
        2 blank lines, all preserved
  - [ ] `snapshot_log_scroll_preserved` — fixture: viewport at line 3
        of 20; new lines appended; viewport unchanged
- [ ] Pane switching:
  - [ ] `snapshot_panes_tab_focus_indicator` — fixture: same 3 panes,
        Panes tab selected and focused at tab bar level
  - [ ] `snapshot_panes_tab_selection` — fixture: same 3 panes, second
        pane active, prev/next affordances visible
- [ ] Misc:
  - [ ] `snapshot_empty_state_no_tasks` — zero-task layout
  - [ ] `snapshot_message_input` — fixture: `m` pressed on selected
        task #2 (`infra-triage`); input prompt rendered at bottom of the
        TUI as a single-line modal with cursor at column 0, addressed to
        `infra-triage`; rest of layout intact and subdued
  - [ ] `snapshot_task_list_pr_count_badge` — fixture (uses
        `fixture::base_three_tasks`): task #1 has 1 PR (badge `Ⓟ1`),
        task #2 has 3 PRs (badge `Ⓟ3`), task #3 has 0 PRs (no badge).
        Sourced from `links.prs[]` count, no enrichment required
  - [ ] `snapshot_key_help_overlay` — `?` overlay enumerating all bindings
        from `redesign.md` §3, grouped:
        - **Global:** `q` quit · `Tab`/`Shift-Tab` cycle pane focus ·
          `?` help · `r` refresh · `m` message
        - **Task list:** `j`/`k` move · `g`/`G` top/bottom · `J`/`K` reorder ·
          `Enter` attach · `n` new · `s` start · `p` pause · `R` resume ·
          `M` modify slug · `x` close · `o` open external · `W` stranded
          overlay
        - **Details:** `h`/`l` switch tab · `o` open highlighted
        - **Panes:** `[`/`]` prev/next · `\` last
        - **Log:** `j`/`k` scroll · `PgUp`/`PgDn` page · `g`/`G` top/bottom ·
          `Esc` back to list
- [ ] Linear stub-driven (Phase 4b ships the real data versions):
  - [ ] `snapshot_linear_anchor_subissues` — fixture: 1 anchor +
        3 children, all from stub cache
  - [ ] `snapshot_linear_empty` — fixture: `links.linear_issues=[]`

### Phase 4a — Session rename + slug rules

Independent of Linear. Lands on the new store.

- [ ] Tmux sessions and worktree paths use `<slug>` only (no numeric prefix,
      no embedded id)
- [ ] Slug-collision suffixes (`-2`, `-3`)
- [ ] `M` key (Modify slug — distinct from `R` resume): three-step staged
      rename per `redesign.md` §4 *tmux*. Persist
      `tmux.rename_in_flight = {old, new}`, rename tmux session,
      `git worktree move`, then clear `rename_in_flight`. Failures leave
      `rename_in_flight` populated and trigger `rename_failed` drift.
- [ ] One-shot migration step renames existing live sessions/worktrees;
      partial failures leave `rename_in_flight` for user retry
- [ ] `M` op is idempotent — re-running picks up from the failed step

Snapshot deliverables (Phase 4a):

- [ ] `snapshot_drift_rename_failed_row` — task row with `rename_failed`
      drift glyph
- [ ] `snapshot_drift_rename_failed_details` — details pane showing
      both `old_name` and `new_name` from `rename_in_flight`

### Phase 4b — Linear data layer

Independent of Phase 4a. Replaces the Phase 3 stub cache with the real one.

- [ ] `LINEAR_API_KEY` env auth, graceful when absent
- [ ] Scan task md/slug for `[A-Z]+-\d+` keys, persist in `links.linear_issues`
- [ ] `linear add ENG-123` / `linear rm` commands
- [ ] Cache: anchor + child_keys + per-issue title/state/assignee + updated_at
- [ ] Background refresh every 2 min, immediate on task select if > 30s stale
- [ ] Count badge in task list

Snapshot deliverables (Phase 4b):

- [ ] `snapshot_linear_multi_link` — fixture: 2 anchors stacked
      top-to-bottom, each with their own sub-issues
- [ ] `snapshot_linear_disconnected` — fixture: API down, render falls
      back to flat list of cached keys + titles
- [ ] `snapshot_linear_stale_warm` — fixture: cache age = 7 min;
      header tinted `LOVE`, content rendered from cache
- [ ] `snapshot_task_list_linear_count_badge` — fixture: 3 tasks,
      task #1 has 2 linked issues, task #3 has 5; badges visible

### Phase 5 — Pane state polish

- [ ] Persist `last_known_pane_id` updates on every pane switch
- [ ] Restore `last_known_pane_id` on `resume`
- [ ] Fall back to pane 0 if the persisted pane no longer exists

Snapshot deliverables (Phase 5):

- [ ] `snapshot_panes_resume_restored` — fixture: resumed task with
      previously-selected pane re-focused

## Decisions (formerly open)

- **Q1** Authoritative Linear doc: `redesign.md` §4. `linear-tui-design.md`
  marked exploratory/superseded at top.
- **Q2** Badge derivation: see matrix in `redesign.md` §2 with
  `worker_alive` as a first-class input. `Error` covers both `worker_dead`
  and explicit op failures.
- **Q3** Slug collisions append `-2`, `-3`. Renames are a 3-step staged op
  with `rename_in_flight` field for crash recovery and idempotent retry.
- **Q4** Zero-task screen: three-pane layout intact; list shows
  `no tasks · n to create`; details + log show empty placeholders.
- **Q5** Phase 3 owns Linear *rendering* via a fixture cache stub;
  Phase 4b ships the data layer.

## Open questions

- **Orphan worktree with uncommitted changes**: stranded-overlay shows it,
  but should `force-remove` first stash, or just `rm -rf`? Currently
  assumes `rm -rf` (user already saw the dirty state in the overlay).
- **Phase 1 migration trigger**: auto on first launch (current default)
  or explicit `orch migrate`? Current spec is auto with `store.version`
  marker as the safety; that seems fine.

## Status

- 2026-04-30: redesign.md generated by codex; iteration 1 folded F1-F5
  fixes; iteration 2 folded P1-P5 + Q1-Q5; iteration 3 added
  `worker_alive` to matrix, drift truth table, `.orch/store.v2/` staging
  with fsync points, split Phase 4a/4b, tightened all snapshot fixtures.
