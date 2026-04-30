# Redesign Review — Iteration 2

## Prior findings status

| Finding | Status | Notes |
|---|---|---|
| F1 | PARTIAL | `redesign.md` now has a badge derivation matrix and explicit precedence, so `Detached`, `Paused + live session`, and `Closed + leftover worktree` are no longer implicit. It still omits the `worker alive` dimension, so `Ready` vs `Error` is not fully deterministic. |
| F2 | PARTIAL | `busy-detection-plan.md` now scopes itself to busy-marker mechanics and points at redesign §2, but later sections still use old-model behavior (`Idle`, `session + !has_active_process -> Ready`). |
| F3 | RESOLVED | Naming is now coherent: Phase 0 explicitly avoids renaming, the target rule is `<slug>` only, collisions get `-2`/`-3`, and rename is an explicit later op. |
| F4 | RESOLVED | `linear-tui-design.md` is now clearly marked exploratory/superseded; `redesign.md` is the implementation reference. |
| F5 | PARTIAL | The docs now define the two drift channels and the orphan policy, but the supposed partition still overlaps (`cleanup_failed` can render in both) and does not account for `worktree_missing`. |
| P1 | RESOLVED | Phase 0 now explicitly says no session-name change; the rename is deferred until the new store exists. |
| P2 | PARTIAL | The temp-dir + marker cutover is now written down, but the atomic flip is not actually specified end-to-end because the target `.orch/` already exists in the migration story. |
| P3 | RESOLVED | Phase 3 now explicitly owns Linear rendering with fixture-fed data; Phase 4 owns auth/discovery/cache. |
| P4 | PARTIAL | Snapshot work is now distributed per phase with named tests, but some tests are scheduled in the wrong phase and several fixtures remain too generic for implementation without re-reading prose. |
| P5 | RESOLVED | Pane fields are now reserved in Phase 1 and written before Phase 5 resume-restore behavior lands. |
| Q1 | RESOLVED | The authoritative Linear doc is now explicit: `redesign.md`; the separate Linear design doc is non-normative. |
| Q2 | PARTIAL | There is now a precedence table, but it still does not fully settle `Ready`/`Error` around dead-worker sessions. |
| Q3 | RESOLVED | Slug collision policy and rename semantics are now explicit. |
| Q4 | RESOLVED | The zero-task screen is now specified. |
| Q5 | RESOLVED | Phase 3 now ships a real stub-driven Linear tab rather than punting the render surface to Phase 4. |

## Snapshot coverage verification

Coverage now exists for every surface that was previously `GAP`, but two surfaces are scheduled in the wrong phase and several of the container snapshots are still underspecified.

| Surface | Verdict | Notes |
|---|---|---|
| Three-pane layout (base render) | PARTIAL | Phase 3. `snapshot_three_pane_base` exists, but "populated lists, no overlays" is still too vague. |
| Three-pane layout (list focus) | PARTIAL | Phase 3. Scheduled, but it should explicitly say it reuses the base fixture and only flips focus. |
| Three-pane layout (details focus) | PARTIAL | Phase 3. Same issue as list focus. |
| Three-pane layout (log focus) | PARTIAL | Phase 3. Same issue as list focus. |
| Detail tab: Overview | PARTIAL | Phase 3. `snapshot_detail_tab_overview` exists, but the fixture contents are not enumerated. |
| Detail tab: PRs | PARTIAL | Named snapshot exists, but it is placed in Phase 2 even though the detail-tab shell appears in Phase 3. |
| Detail tab: Linear | PARTIAL | The Linear render surface is in Phase 3, but the explicit `snapshot_detail_tab_linear` is deferred to Phase 4. |
| Detail tab: Panes | PARTIAL | Phase 3. Scheduled, but the exact pane fixture still needs to be spelled out. |
| Linear panel: anchor + sub-issues (default) | OK | Phase 3. `snapshot_linear_anchor_subissues` is concrete enough: 1 anchor + 3 children from stub data. |
| Linear panel: multi-link (stacked anchors) | OK | Phase 4. `snapshot_linear_multi_link` is concrete enough. |
| Linear panel: empty (no linked issue) | OK | Phase 3. `snapshot_linear_empty` is concrete enough. |
| Linear panel: disconnected (flat cached list fallback) | OK | Phase 4. `snapshot_linear_disconnected` is concrete enough. |
| Linear panel: stale-warm header tint (>5 min stale) | OK | Phase 4. `snapshot_linear_stale_warm` is concrete enough. |
| Log pane: wrapped lines | OK | Phase 3. `snapshot_log_wrapped_lines` is concrete enough. |
| Log pane: scroll preserved when not at bottom | OK | Phase 3. `snapshot_log_scroll_preserved` is concrete enough. |
| Status badge: Detached | OK | Phase 1. `snapshot_status_detached` is concrete enough. |
| Status badge: Ready | OK | Phase 0. `snapshot_status_ready` is concrete enough. |
| Status badge: Working | OK | Phase 0. `snapshot_status_working` is concrete enough. |
| Status badge: Input | PARTIAL | Phase 1. `needs_input=true` is not quite enough; it should also pin the session/attach state so precedence is unambiguous. |
| Status badge: Attached | OK | Phase 1. `snapshot_status_attached` is concrete enough. |
| Status badge: Error | OK | Phase 1. `snapshot_status_error` is concrete enough. |
| Drift overlay: orphan worktree | PARTIAL | Phase 0. Scheduled, but this is current-code warning behavior; the final redesign later auto-removes clean orphans and only overlays dirty ones. |
| Drift overlay: cleanup_failed | OK | Phase 1. `snapshot_drift_cleanup_failed` is concrete enough for the per-task drift case. |
| Drift overlay: session_missing | OK | Phase 1. `snapshot_drift_session_missing` is concrete enough. |
| In-app pane switching: focus indicator on `Panes` tab | PARTIAL | Phase 3. Scheduled, but it still needs an exact pane/tab-bar fixture. |
| In-app pane switching: pane selection | PARTIAL | Phase 3. Scheduled, but it still needs an exact pane set and active pane. |
| Stranded-worktree overlay | OK | Phase 4. `snapshot_stranded_worktrees_overlay` is concrete enough. |
| Empty / initial state (no tasks) | OK | Phase 3. `snapshot_empty_state_no_tasks` is concrete enough. |
| Key help overlay (`?`) | OK | Phase 3. `snapshot_key_help_overlay` is concrete enough. |
| Task list row variant: New | OK | Phase 1. Covered by the row-variant snapshot set. |
| Task list row variant: Active + Ready | OK | Phase 1. Covered by the row-variant snapshot set. |
| Task list row variant: Active + Working | OK | Phase 1. Covered by the row-variant snapshot set. |
| Task list row variant: Active + Input | OK | Phase 1. Covered by the row-variant snapshot set. |
| Task list row variant: Paused | OK | Phase 1. Covered by the row-variant snapshot set. |
| Task list row variant: Attached | OK | Phase 1. Covered by the row-variant snapshot set. |
| Task list row variant: drift indicator | PARTIAL | Phase 1. `snapshot_task_row_with_drift_indicator` needs a canonical drift flag; right now it is too generic. |

New fix-up surfaces that still do not have an explicit snapshot:

- `rename_failed` drift as rendered in the task row/details surface.
- `worktree_missing` drift, if that flag remains part of the model.
- The task-list Linear count badge (`redesign.md` §5 left column).

Phase 3's stub-data approach is otherwise sufficient for shipping Linear render snapshots without the real data layer. The remaining gap there is scheduling: the explicit `detail_tab_linear` snapshot is still delayed to Phase 4.

## New issues

### N1: Runtime badge matrix still collapses session presence and worker liveness
- WHAT: `redesign.md` defines `Ready` as "session exists, worker process alive, not busy", but the matrix has no worker-liveness column. `busy-detection-plan.md` then hard-codes `session + !has_active_process -> Ready`, which contradicts the redesign and leaves dead-Claude sessions with no deterministic badge.
- WHERE: `redesign.md` §2 Runtime state + Badge derivation matrix; `busy-detection-plan.md` §Marker -> redesign badge mapping and §Test plan.
- SUGGESTION: Add an explicit `worker_alive`/`has_active_process` input to the matrix and rewrite the busy-detection test cases against that matrix.

### N2: The drift "partition" is neither exhaustive nor exclusive
- WHAT: `redesign-notes.md` still lists `worktree_missing` as a drift flag, but `redesign.md`'s partition table never assigns it to a channel. The same table also says a closed-task `cleanup_failed` worktree can render in both channels, so the design no longer has exactly-one-channel ownership.
- WHERE: `redesign-notes.md` Phase 1 drift flags; `redesign.md` §Drift surface partition.
- SUGGESTION: Replace the current prose with one truth table: every drift flag/case, exactly where it renders, and whether dual-render is allowed.

### N3: The migration cutover cannot be atomic as written
- WHAT: The migration says first launch must inspect existing `.orch/runs`, but the cutover then renames `.orch.tmp/` to `.orch/`. That target already exists in the migration story, so the promised single atomic flip does not actually work. The crash-safety text also stops at `fsync each file`; it never fsyncs the temp dir, parent dir, or final marker write.
- WHERE: `redesign.md` §First-run import and §Idempotent cutover.
- SUGGESTION: Stage only the new store under a fresh subpath (for example `.orch/store.v2/`), fsync the temp files + temp dir + parent dir, then flip one authoritative marker/pointer.

### N4: Phase 4 still hides a rename-specific coupling and an incomplete recovery story
- WHAT: Linear data-layer work and session rename are grouped only to "migrate once", but rename no longer depends on Linear. That creates schedule coupling for unrelated work. Separately, the rename spec says the record is "not updated" on partial failure, which loses track of any external rename step that already succeeded.
- WHERE: `redesign-notes.md` §Phase 4; `redesign.md` §tmux rename rules.
- SUGGESTION: Split Phase 4 into `4a rename` and `4b Linear data` under the same store version, and specify rename recovery as either compensating rollback or persisted `old_name/new_name` drift state.

## Verdict

Another iteration is needed before implementation; the runtime/drift truth tables and the migration/rename recovery path are still not reliable enough to code from.
