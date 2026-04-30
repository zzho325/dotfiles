# Redesign Review

## Snapshot coverage matrix

| Surface | Status | Test name / gap |
|---|---|---|
| Three-pane layout (base render) | GAP | `snapshot_three_pane_base`; fixture: 3 tasks, populated details/log panes, no overlays. Only generic snapshot intent exists in `redesign.md:607-608` and `redesign-notes.md:61-65`; no explicit fixture is listed. |
| Three-pane layout (list focus) | GAP | `snapshot_three_pane_list_focus`; fixture: same base data, left pane focused, right panes subdued. Surface: `redesign.md:465-519`. |
| Three-pane layout (details focus) | GAP | `snapshot_three_pane_details_focus`; fixture: same base data, details pane focused, list/log unfocused. Surface: `redesign.md:485-519`. |
| Three-pane layout (log focus) | GAP | `snapshot_three_pane_log_focus`; fixture: same base data, log pane focused, list/details unfocused. Surface: `redesign.md:497-519`. |
| Detail tab: Overview | GAP | `snapshot_detail_tab_overview`; fixture: selected task with task file, lifecycle, worktree, last activity. Surface: `redesign.md:487-493, 268-275`. |
| Detail tab: PRs | GAP | `snapshot_detail_tab_prs`; fixture: 2 linked PRs, one fully enriched, one link-only/stale. Surface: `redesign.md:487-495, 372-385`. |
| Detail tab: Linear | GAP | `snapshot_detail_tab_linear`; fixture: linked issue present, Linear tab selected. Surface: `redesign.md:487-493, 297-351`. |
| Detail tab: Panes | GAP | `snapshot_detail_tab_panes`; fixture: 3 tmux panes, active pane highlight, last-selected pane marker. Surface: `redesign.md:487-493, 277-284`. |
| Linear panel: anchor + sub-issues (default) | GAP | `snapshot_linear_anchor_subissues`; fixture: 1 anchor issue with 3 child issues. Surface: `redesign.md:318-323`; fuller mockup in `linear-tui-design.md:419-442`. |
| Linear panel: multi-link (stacked anchors) | GAP | `snapshot_linear_multi_link`; fixture: 2-3 linked anchors, stacked top-to-bottom. Surface: `redesign.md:322-323`. |
| Linear panel: empty (no linked issue) | GAP | `snapshot_linear_empty`; fixture: `links.linear_issues=[]`, include linking guidance. Surface: `redesign.md:324-324`; fuller mockup in `linear-tui-design.md:522-538`. |
| Linear panel: disconnected (flat cached list fallback) | GAP | `snapshot_linear_disconnected`; fixture: API unavailable or key missing, 2 cached links, flat list fallback. Surface: `redesign.md:308-309, 325-326`; fuller mockup in `linear-tui-design.md:540-560`. |
| Linear panel: stale-warm header tint (>5 min stale) | GAP | `snapshot_linear_stale_warm`; fixture: cached data older than 5 min, stale header tint, stale age text. Surface: `redesign.md:323-323`; fuller mockup in `linear-tui-design.md:562-582`. |
| Log pane: wrapped lines | GAP | `snapshot_log_wrapped_lines`; fixture: long log lines that must wrap with blank lines preserved. Surface: `redesign.md:497-509, 286-293`. |
| Log pane: scroll preserved when not at bottom | GAP | `snapshot_log_scroll_preserved`; fixture: off-bottom scroll offset with new lines appended; snapshot should show unchanged viewport. Surface: `redesign.md:505-510`. |
| Status badge: Detached | GAP | `snapshot_status_detached`; fixture: `desired_state=Active`, no tmux session, no drift overlay. Surface: `redesign.md:166-173`. |
| Status badge: Ready | GAP | `snapshot_status_ready`; fixture: active task, worker alive, not busy. Surface: `redesign.md:166-173`. |
| Status badge: Working | GAP | `snapshot_status_working`; fixture: active task, fresh busy marker. Surface: `redesign.md:166-195`. |
| Status badge: Input | GAP | `snapshot_status_input`; fixture: `needs_input=true`. Surface: `redesign.md:166-173`. |
| Status badge: Attached | GAP | `snapshot_status_attached`; fixture: selected task attached in tmux. Surface: `redesign.md:166-173`. |
| Status badge: Error | GAP | `snapshot_status_error`; fixture: explicit runtime disagreement case. Surface defines badge but not the rendered case in `redesign.md:173-173`; needs a canonical fixture. |
| Drift overlay: orphan worktree | GAP | `snapshot_drift_orphan_worktree`; fixture: unbound clean worktree surfaced by reconciler warning/overlay. Surface: `redesign.md:405-410`. |
| Drift overlay: cleanup_failed | GAP | `snapshot_drift_cleanup_failed`; fixture: closed task whose worktree remove failed. Surface: `redesign.md:106-110, 237-244, 402-404`. |
| Drift overlay: session_missing | GAP | `snapshot_drift_session_missing`; fixture: `desired_state=Active` with missing tmux session and drift marker visible. Surface: `redesign.md:106-110, 239-241, 480-480`. |
| In-app pane switching: focus indicator on `Panes` tab | GAP | `snapshot_panes_tab_focus_indicator`; fixture: Panes tab selected and focused at tab bar level. Surface: `redesign.md:270-281, 487-493, 511-519`. |
| In-app pane switching: pane selection | GAP | `snapshot_panes_tab_selection`; fixture: 3 tmux panes, second pane active, previous/next affordance reflected. Surface: `redesign.md:277-284, 492-492`. |
| Stranded-worktree overlay | GAP | `snapshot_stranded_worktrees_overlay`; fixture: dirty orphan worktree with inspect/attach/force-remove affordances. Surface: `redesign.md:409-415`. |
| Empty / initial state (no tasks) | GAP | `snapshot_empty_state_no_tasks`; fixture: zero tasks, empty details/log panes, first-run copy. No render spec in the redesign docs. |
| Key help overlay (`?`) | GAP | `snapshot_key_help_overlay`; fixture: base layout with compact help overlay open. Key exists in `redesign.md:250-252`, but the overlay render is not specified. |
| Task list row variant: New | GAP | `snapshot_task_row_new`; fixture: `desired_state=New`, no runtime session, zero links. Surface: `redesign.md:147-159, 473-480`. |
| Task list row variant: Active + Ready | GAP | `snapshot_task_row_active_ready`; fixture: active task with Ready badge and no drift. Surface: `redesign.md:166-173, 473-480`. |
| Task list row variant: Active + Working | GAP | `snapshot_task_row_active_working`; fixture: active task with Working badge and counts. Surface: `redesign.md:166-195, 473-480`. |
| Task list row variant: Active + Input | GAP | `snapshot_task_row_active_input`; fixture: active task with Input badge. Surface: `redesign.md:166-173, 473-480`. |
| Task list row variant: Paused | GAP | `snapshot_task_row_paused`; fixture: `desired_state=Paused`, no live session. Surface: `redesign.md:147-159, 473-480`. |
| Task list row variant: Attached | GAP | `snapshot_task_row_attached`; fixture: active task with Attached badge. Surface: `redesign.md:166-173, 473-480`. |
| Task list row variant: drift indicator | GAP | `snapshot_task_row_with_drift_indicator`; fixture: active or closed task with drift marker plus counts. Surface: `redesign.md:237-244, 473-480`. |

## Design coherence findings

### F1: Runtime badge contract is incomplete
- WHAT: The redesign separates persisted `desired_state` from derived `runtime_state`, but it never defines the precedence between runtime badges and drift flags. `Active + no session` could be `Detached` or `Error`; `Paused + session still alive` could render as `Attached`/`Working` plus drift, or as `Error`; `Closed + worktree still present` is called drift but not mapped to a badge. That leaves both implementation and snapshots non-deterministic.
- WHERE: `redesign.md` §2 / lines 164-177, 236-244, 473-480.
- SUGGESTION: Add a single derivation matrix: `(desired_state, session presence, attached, busy marker, needs_input, drift flags) -> badge + drift marker + row copy`.

### F2: Busy-detection plan still speaks the pre-redesign status model
- WHAT: The busy-detection plan says "keep everything else about the state model unchanged" and its derivation box still returns `Paused`/`Idle`/`Attached`/`Input`/`Ready`/`Working`. The redesign replaces that with `desired_state` plus runtime badges including `Detached` and `Error`. The two docs are not aligned on the status vocabulary or precedence rules.
- WHERE: `busy-detection-plan.md` / lines 15-17, 58-67; `redesign.md` §2 / lines 164-177.
- SUGGESTION: Update `busy-detection-plan.md` to say it only owns busy-marker mechanics, and restate derivation in redesign-era terms.

### F3: tmux session naming contradicts itself
- WHAT: One section says start/resume creates tmux sessions named `<slug>` only. Later the tmux section says session names are "derived from durable task ID". The migration section then says tmux session names are display-friendly slugs that can be renamed without changing identity. Those are three different contracts.
- WHERE: `redesign.md` §2 / lines 203-206; `redesign.md` §4 / lines 427-429; `redesign.md` §6 / lines 594-600; `redesign-notes.md` / lines 29-33.
- SUGGESTION: Pick one naming rule and state collision handling for duplicate/renamed slugs. If the answer is `<slug>`, delete all "derived from durable task ID" wording.

### F4: The Linear docs do not make the normative boundary clear
- WHAT: `redesign.md` explicitly cuts scope cycling, transition modal, comment composer, and numbered multi-link strip. `linear-tui-design.md` still presents those as the recommended interaction model. Because the redesign still links to that file, an implementer can reasonably build the larger surface and a reviewer cannot tell which snapshots are required.
- WHERE: `redesign.md` §4 / lines 299-304, 322-336; `linear-tui-design.md` / lines 101-124, 180-230, 244-266.
- SUGGESTION: Mark `linear-tui-design.md` as exploratory/superseded for implementation, or prepend a short "non-normative except where copied into redesign.md" note.

### F5: Worktree-drift surfaces are split but not partitioned
- WHAT: The redesign has per-task drift flags (`cleanup_failed`, `session_missing`, `worktree_missing`) and also a global stranded-worktree overlay for dirty unbound worktrees. It does not define which leftover worktree cases belong only on the task row vs only in the global overlay vs both. That is visible-user behavior, not just implementation detail.
- WHERE: `redesign.md` §1 / lines 106-110; `redesign.md` §4 / lines 403-415; `redesign-notes.md` / lines 14-19.
- SUGGESTION: Define two buckets explicitly: task-bound drift and unbound reconciler findings. State whether clean orphans auto-remove silently, show a transient notice, or still get a reviewable overlay row.

## Phase plan findings

### P1: Phase 0 is not "no data model change" in rollout terms
- WHAT: The session-name change is not just a UI tweak. The legacy store persists full tmux session names, and current matching/rename behavior assumes `task-<slug>` plus optional numeric prefix. Shipping `<slug>`-only sessions without a compatibility step risks breaking jump/pause/status for in-flight tasks.
- WHERE: `redesign-notes.md` / lines 8-35; current behavior in `src/main.rs:434-518`, `src/tui.rs:382-423`, `src/state.rs:163-175`.
- SUGGESTION: Either keep Phase 0 to worktree GC + busy markers only, or explicitly include a compatibility rollout for stored/live session names.

### P2: Phase 1 migration needs an idempotent cutover story
- WHAT: The docs say first launch writes `registry.json` plus per-task records and leaves legacy `.state/` readable for one release, but they do not say how a partial migration recovers if the process dies mid-write. Without a completion marker or staged rename, repeated launches can allocate IDs twice or import only a subset of tasks.
- WHERE: `redesign.md` §6 / lines 531-586; `redesign-notes.md` / lines 41-51.
- SUGGESTION: Stage the new store under a temp dir, write all task files atomically, then flip a single "migration complete" marker before the new reader becomes authoritative.

### P3: Phase 3 and Phase 4 are coupled more tightly than the notes admit
- WHAT: Phase 3 says the TUI rewrite ships `Overview · PRs · Linear · Panes` tabs. Phase 4 later adds the actual Linear auth/discovery/cache/rendering. That means either Phase 3 ships a placeholder `Linear` tab, or Phase 4 is partially blocking Phase 3.
- WHERE: `redesign-notes.md` / lines 59-77.
- SUGGESTION: Make the dependency explicit: either move minimal Linear render into Phase 3, or state that Phase 3 ships a stub `Linear` tab with only placeholder/empty messaging.

### P4: Snapshot work is scheduled too late and too vaguely
- WHAT: Phase 0 and Phase 1 both change TUI-visible behavior: orphan warnings, drift surfacing, and status semantics. But snapshots appear only in Phase 3, and only as a generic "`insta`" bullet. That misses the review goal that every TUI change be inspectable from snapshots as it lands.
- WHERE: `redesign-notes.md` / lines 14-18, 41-49, 61-65.
- SUGGESTION: Add explicit snapshot deliverables to each phase that changes the TUI, with fixture names committed in the phase plan before implementation starts.

### P5: Phase 5 hides a data-shape dependency that should be reserved earlier
- WHAT: Phase 5 persists `last_known_pane_id` for resume restore, but the phase notes do not say when the new store starts recording pane IDs/titles. If Phase 3 ships the `Panes` tab and in-app pane switching before that persistence exists, restore behavior will be absent or ad hoc.
- WHERE: `redesign.md` §1 / lines 93-97; `redesign-notes.md` / lines 79-82.
- SUGGESTION: Reserve pane fields in Phase 1 and state whether Phase 3 writes them opportunistically, even if restore-on-resume waits until Phase 5.

## Open questions

- Q1: Which Linear doc is authoritative for implementation and review: the minimal surface in `redesign.md`, or the richer interaction model in `linear-tui-design.md`?
- Q2: What is the canonical precedence table for `Detached` / `Ready` / `Working` / `Input` / `Attached` / `Error` when drift flags are also present?
- Q3: If tmux sessions and worktree paths are `<slug>` only, what is the collision policy for duplicate slugs or task renames?
- Q4: What should the zero-task screen render in the three-pane layout on first launch?
- Q5: Does Phase 3 ship a placeholder `Linear` tab, or is minimal Linear rendering pulled forward into the TUI rewrite phase?
