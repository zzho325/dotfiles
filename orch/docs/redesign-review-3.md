# Redesign Review — Iteration 3

## Findings status

| Finding | Status | Note |
|---|---|---|
| F1 | RESOLVED | The badge matrix now includes `worker_alive` and gives a deterministic badge + drift outcome for each lifecycle/runtime case. |
| F2 | PARTIAL | The busy-marker plan is mostly rewritten against the redesign matrix, but it still says `has_active_process` feeds the `session present` column instead of `worker_alive`. |
| F3 | RESOLVED | Session naming is consistently `<slug>` only, with collision suffixes and rename behavior spelled out in one place. |
| F4 | RESOLVED | `linear-tui-design.md` is clearly exploratory/superseded and `redesign.md` is the implementation reference. |
| F5 | RESOLVED | The old overlap is replaced by a truth table that assigns every drift case to one channel, with `cleanup_failed` explicitly dual-rendered. |
| P1 | RESOLVED | Phase 0 still avoids session renames and stays within the pre-store stability scope. |
| P2 | RESOLVED | Migration now stages under `.orch/store.v2.tmp/`, fsyncs the relevant files/directories, and gates authority on `store.version`. |
| P3 | RESOLVED | Phase 3 now owns the shell for `PRs` and `Linear`, and Phase 4b is only the real Linear data layer. |
| P4 | PARTIAL | The phase placement is fixed, but one drift snapshot is now impossible and several badge fixtures still rely on implied false/`n/a` inputs. |
| P5 | RESOLVED | Pane-state fields are still reserved in Phase 1 before restore-on-resume lands in Phase 5. |
| Q1 | RESOLVED | The authoritative Linear implementation doc boundary is explicit. |
| Q2 | RESOLVED | The matrix now settles `Detached` / `Ready` / `Working` / `Input` / `Attached` / `Error` precedence. |
| Q3 | RESOLVED | Slug collision handling and retryable rename semantics are explicit. |
| Q4 | RESOLVED | The zero-task three-pane state is specified concretely. |
| Q5 | RESOLVED | Phase 3 now ships a real stub-driven `Linear` tab instead of deferring the surface. |
| N1 | RESOLVED | `worker_alive` is now a first-class matrix input and the busy-detection tests are rewritten against it. |
| N2 | RESOLVED | The drift partition prose is replaced with a truth table that covers `worktree_missing` and makes the `cleanup_failed` exception explicit. |
| N3 | RESOLVED | The cutover now uses a versioned subdir plus marker flow that can recover cleanly from mid-migration crashes. |
| N4 | RESOLVED | Rename is split into Phase 4a and uses `rename_in_flight` to make partial completion visible and retryable. |

## Snapshot coverage

| Surface | Verdict | Note |
|---|---|---|
| Status badge fixtures | PARTIAL | `snapshot_status_attached`, `snapshot_status_input`, `snapshot_status_working`, and `snapshot_status_ready` still omit some non-triggering inputs (`attached=no`, `needs_input=false`, `busy=stale_or_absent`), so they are not fully self-contained fixtures yet. |
| Task-list drift row | GAP | `snapshot_task_row_with_drift_indicator` is specified as `Active + cleanup_failed + Ready`, but `cleanup_failed` is only defined for `Closed` tasks in the drift truth table. |
| Stranded overlay: `cleanup_failed` channel | GAP | The truth table says `cleanup_failed` also renders in the stranded overlay, but the only overlay snapshot scheduled is the unbound-dirty case. |

## New issues

### M1: `R` is bound to both resume and rename

`redesign.md` still uses `R` for `resume selected Paused task`, while the tmux rename section and Phase 4a also assign `R` to rename, so the key surface is no longer coherent.

### M2: The canonical drift-row fixture contradicts the new truth table

Iteration 3 rewrote drift ownership correctly, but the new Phase 1 fixture `snapshot_task_row_with_drift_indicator` still asks for an impossible `Active + cleanup_failed` row.

## Verdict

Another iteration is needed before implementation.
