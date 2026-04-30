# Redesign Review — Iteration 4 (Final)

## Findings status

| Finding | Status | Note |
|---|---|---|
| F1 | RESOLVED | `redesign.md` now has a total badge matrix with `worker_alive`, so lifecycle/runtime rendering is deterministic. |
| F2 | RESOLVED | `busy-detection-plan.md` now scopes itself to busy-marker mechanics and explicitly says `has_active_process` feeds `worker_alive`. |
| F3 | RESOLVED | The naming contract is coherent: `<slug>` session/worktree names, collision suffixes, and staged rename semantics are specified consistently. |
| F4 | RESOLVED | `linear-tui-design.md` is clearly marked exploratory/superseded and `redesign.md` is the implementation source. |
| F5 | RESOLVED | The drift partition now assigns each case to a concrete channel, with `cleanup_failed` explicitly called out as dual-render. |
| P1 | RESOLVED | Phase 0 stays pre-store and explicitly avoids session renames. |
| P2 | RESOLVED | Migration now stages under `.orch/store.v2.tmp/`, fsyncs, atomically renames to `.orch/store.v2/`, and gates authority on `store.version=v2`. |
| P3 | RESOLVED | Phase 3 owns the TUI shell and stub-driven Linear render; Phase 4b is only the real Linear data layer. |
| P4 | PARTIAL | Snapshot placement is now correct and most fixtures are concrete, but a small number of snapshot fixtures are still underspecified. |
| P5 | RESOLVED | Pane-state fields are reserved in Phase 1 before Phase 5 restore behavior. |
| Q1 | RESOLVED | The normative Linear doc boundary is explicit. |
| Q2 | RESOLVED | Badge precedence is now settled by the matrix. |
| Q3 | RESOLVED | Slug-collision and rename-retry behavior are explicit. |
| Q4 | RESOLVED | The zero-task three-pane state is spelled out concretely. |
| Q5 | RESOLVED | Phase 3 now ships a real stub-driven `Linear` tab. |
| N1 | RESOLVED | `worker_alive` is a first-class matrix input in both redesign and busy-detection docs. |
| N2 | RESOLVED | The drift table now covers `worktree_missing` and makes the `cleanup_failed` exception explicit. |
| N3 | RESOLVED | The cutover story now uses a versioned subdir and durable marker flow rather than renaming all of `.orch/`. |
| N4 | RESOLVED | Rename is split into Phase 4a and uses `rename_in_flight` for partial-failure recovery. |
| M1 | PARTIAL | The keymap conflict is gone, but `redesign.md` still has stale rename/resume key references (`retry R`, `r resume`) that contradict the final `R`/`M` bindings. |
| M2 | RESOLVED | The impossible `Active + cleanup_failed` row is gone; the dual-render case now has separate per-task and overlay snapshots. |

## Snapshot coverage

- PARTIAL: `snapshot_detail_tab_overview` still does not pin the full Overview fixture from the doc alone; it names lifecycle/worktree/last_activity but not the other rendered Overview fields.
- PARTIAL: `snapshot_key_help_overlay` is still too vague; the overlay contents are not concretely enumerated anywhere, only that `?` opens a compact help overlay.

## New issues

### X1: Stale key references remain after the `R` -> `M` rename split

`redesign.md` still says rename failure can be retried with `R`, and the drift/error prose still says `r` resumes recovery actions, while the final task-list keymap says `R` = resume and `M` = modify slug.

## Verdict

Iteration 5 needed.
