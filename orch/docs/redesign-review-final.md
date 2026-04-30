# Final Review

## Implementation vs design

### F1: Lifecycle still mutates from runtime
- WHAT: The redesign’s core rule was “persist desired state, derive runtime separately.” The implementation still auto-pauses tasks when tmux is missing, so runtime drift mutates lifecycle. `derive_status` is also still legacy `TaskMeta`-based and returns `Idle` instead of the redesign’s `Detached`/matrix behavior.
- WHERE: `src/main.rs:733-735`, `src/state.rs:329-367`, `src/state.rs:423-437`
- SEVERITY: blocker
- SUGGESTED FIX: Remove `auto_pause_orphaned` from the polling path. Drive badges from `TaskRecord.desired_state` + live observations + drift flags, and never rewrite lifecycle state from tmux absence alone.

### F2: v2 store is not actually authoritative
- WHAT: `store.version=v2` does not make the new store authoritative for task population or ordering. Open task enumeration still comes from `~/tasks/*.md`, and ordering still comes from legacy `.state/order.json`. `Registry.open_order`/`closed_order` are never the live source of truth, so closed-history/state-machine promises are not honored.
- WHERE: `src/state.rs:112-127`, `src/state.rs:385-399`, `src/state.rs:688-700`, `src/tui3.rs:213-279`
- SEVERITY: blocker
- SUGGESTED FIX: When `Store::is_authoritative()` is true, load tasks and ordering from `Store`/`Registry`, update `Registry.open_order` on reorder, and use `closed_order` for history views.

### F3: Busy detection breaks the per-session contract
- WHAT: The hooks key markers by session id, but orch ignores session id and scans all fresh markers by `cwd`. Any Claude session in the same worktree can therefore make the task look `Working`. The marker `pid` is also the hook process PID, not Claude’s PID as promised.
- WHERE: `claude/.claude/hooks/orch-busy-start.sh:27-34`, `src/state.rs:275-305`
- SEVERITY: important
- SUGGESTED FIX: Correlate tmux pane -> Claude session id and read the matching marker by sid. Either write the real Claude PID or drop `pid` from the contract until it is correct.

### F4: `orch close` is not v2-safe
- WHAT: `close` never persists `desired_state=Closed`, never moves the record from `open_order` to `closed_order`, never sets `archived_task_file`, ignores `cleanup_on_close`, and archives to `done/<slug>.md` instead of `done/<id>-<slug>.md`. It also keeps going after archive failure.
- WHERE: `src/main.rs:629-683`, `src/state.rs:129-145`, `src/state.rs:472-497`
- SEVERITY: blocker
- SUGGESTED FIX: Implement close against `TaskRecord` first: persist `Closed` + archive path, update registry order, then run tmux/worktree cleanup, and stop destructive follow-on steps if archiving fails.

### F5: Migration is weaker than the promised cutover
- WHAT: The migration writes task files under `store.v2.tmp/tasks/` but only fsyncs the files and `store.v2.tmp/`, not the `tasks/` directory itself. It also does not preserve live tmux-only work as `Active` when legacy state is missing/divergent; it mostly ignores live tmux beyond `session_missing`.
- WHERE: `src/store.rs:424-475`, `src/store.rs:553-629`
- SEVERITY: important
- SUGGESTED FIX: Fsync `store.v2.tmp/tasks/` after writing task records, and fold live tmux inspection into record creation so live sessions win when legacy state disagrees.

### F6: The TUI omits promised task identity and drift surfaces
- WHAT: The left pane does not render durable `#id` or a separate drift glyph, and the selected task loses `HL_LOW` once focus leaves the list. That is materially short of the redesign’s row contract and contributes to the navigation ambiguity.
- WHERE: `src/tui3.rs:124-132`, `src/tui3.rs:225-279`, `src/tui3.rs:521-571`
- SEVERITY: important
- SUGGESTED FIX: Carry `id` and drift info into `TaskView`, render them in the list, and keep the selected-row background independent of focus.

### F7: PR and Linear tabs are render-only, not the promised control surface
- WHAT: The redesign promised `o` open, `r` refresh, and item navigation in detail tabs. The implementation has no PR cursor, no Linear cursor, no `o`, no `r`, and no `j/k` handling for details. The help overlay even claims `j/k` works in the right pane, but the key handler does not implement it.
- WHERE: `src/tui3.rs:693-776`, `src/tui3.rs:898-921`, `src/tui3.rs:1098-1144`
- SEVERITY: important
- SUGGESTED FIX: Either wire per-tab cursor/open/refresh behavior, or explicitly label PRs/Linear as read-only until that lands and fix the help text to match reality.

### F8: `\` does not do what the design says
- WHAT: In the Panes tab, `\` is documented as “jump back to the last pane used,” but the code jumps to the currently active pane because no last-used pane state is wired.
- WHERE: `src/tui3.rs:1122-1129`
- SEVERITY: nit
- SUGGESTED FIX: Rename the behavior in UI/docs to “jump to active pane” until `last_known_pane_id` is actually persisted and used.

## IMPLEMENTATION.md accuracy

### A1: The doc overstates v2 cutover
- WHAT: The doc says reads come from the new store and the gated read path is live, but only slug-level `TaskMeta` flattening uses v2. Ordering, task enumeration, closed history, and close semantics are still legacy-driven.
- WHERE: `docs/IMPLEMENTATION.md:94-107`, `docs/IMPLEMENTATION.md:221-223`, `src/state.rs:129-145`, `src/state.rs:688-700`, `src/tui3.rs:213-279`
- SEVERITY: important
- SUGGESTED FIX: Rewrite the status doc to say v2 is only partially authoritative today, or finish moving order/task enumeration/close to the store before claiming the cutover shipped.

### A2: Several “pain points addressed” rows are overclaimed
- WHAT: “Number gaps” is not fixed in the user-facing TUI because durable ids are not shown and registry order is unused. “Frequent start/close uncodified” is overstated because `close` is still legacy/v2-incomplete. “Not stable” is also overstated because busy detection is still per-cwd and lifecycle still auto-mutates on session loss.
- WHERE: `docs/IMPLEMENTATION.md:225-236`, `src/tui3.rs:521-571`, `src/main.rs:629-683`, `src/state.rs:275-305`, `src/state.rs:423-437`
- SEVERITY: important
- SUGGESTED FIX: Downgrade those rows to partial until the actual user-visible fixes land.

### A3: The doc mixes code status with unverifiable production assertions
- WHAT: “daemon running with the new binary,” “migrated 8 tasks,” “marker hooks symlinked,” and the live PID/heartbeat claims are operational statements that cannot be verified from code review. The file tree also points at `orch/claude/.claude/hooks/`, but the hooks actually live under the sibling `claude/` package.
- WHERE: `docs/IMPLEMENTATION.md:215-223`, `docs/IMPLEMENTATION.md:256-279`, `docs/IMPLEMENTATION.md:281-289`
- SEVERITY: important
- SUGGESTED FIX: Split code-backed status from runtime deployment notes, and fix the hook path references.

### A4: The P2 provenance/discovery claim is not true yet
- WHAT: The doc claims `links.prs[]` now carries source provenance and that manual/branch-discovered links are live. In practice, discovery is still `jj op log`-based only, there is no markdown scan, and `pr add` writes new links through `TaskMeta`, which turns them into `Migration` links unless they were already manual.
- WHERE: `docs/IMPLEMENTATION.md:200-213`, `src/main.rs:979-984`, `src/state.rs:502-558`, `src/state.rs:562-674`
- SEVERITY: important
- SUGGESTED FIX: Either mark P2 partial, or add v2-aware PR add/discovery paths that write `LinkSource::Manual`/`BranchDiscovery`/`MarkdownScan` directly.

## Nav redesign coherence

### N1: The proposal’s diagnosis is partly based on stale reads of `tui3.rs`
- WHAT: The current TUI already has direct `1/2/3/4` tab jump and already renders a vertical `│` divider. The help overlay also does not list `s p R x M n W o`. The real mismatch is different: the help overlay wrongly claims right-pane `j/k` navigation, while the code still uses `[` `]` `\`.
- WHERE: `docs/tui-nav-redesign.md:29-30`, `docs/tui-nav-redesign.md:50-55`, `docs/tui-nav-redesign.md:62`, `src/tui3.rs:483-489`, `src/tui3.rs:898-921`, `src/tui3.rs:1043-1058`, `src/tui3.rs:1098-1144`
- SEVERITY: important
- SUGGESTED FIX: Update the diagnosis to describe the actual current surface before using it as the argument for the redesign.

### N2: One “delete this from tui3.rs” item points at code that does not exist
- WHAT: The proposal says to delete the help-overlay section that lists `s/p/R/x/M/n/W/o` “as if they worked.” The current help overlay has no such section, so that bullet is pointing at stale/nonexistent code.
- WHERE: `docs/tui-nav-redesign.md:356-360`, `src/tui3.rs:898-921`
- SEVERITY: nit
- SUGGESTED FIX: Replace that bullet with the real cleanup target: the overlay’s false `j/k` right-pane guidance and its mismatch with actual detail-key handling.

### N3: `Esc` semantics need one canonical statement
- WHAT: Section 2 says `Esc` “always returns focus to zone A and resets to Overview,” the keymap says `Esc` from list quits, and the help mockup only says `Esc focus list`. The intended model is sensible, but the doc does not state it once, cleanly, and consistently.
- WHERE: `docs/tui-nav-redesign.md:52-55`, `docs/tui-nav-redesign.md:74-83`, `docs/tui-nav-redesign.md:214-215`
- SEVERITY: important
- SUGGESTED FIX: Put the full rule in one place and mirror it everywhere: modal -> cancel, right -> list, list -> quit, plus whether `Esc` resets the tab to Overview.

## Cross-cutting smells

### S1: Tests are not fully isolated from the user’s live state
- WHAT: Store tests use `Store::at(...)`, but several state tests write into the real `~/tasks/.state/` path and also go through `save_task_meta`, which can mirror into the real v2 store if `store.version=v2` is present. Other tests mutate process-global env vars, despite assuming single-threaded execution.
- WHERE: `src/store.rs:656-661`, `src/state.rs:472-497`, `src/state.rs:737-767`, `src/state.rs:917-943`, `src/state.rs:1089-1096`
- SEVERITY: important
- SUGGESTED FIX: Add a test-only tasks root override, route all state/cache tests through temp dirs, and serialize or avoid env-var mutation.

### S2: Shared busy markers can leak across the user’s live sessions
- WHAT: Because busy markers live in a shared runtime dir and orch matches them by cwd, any non-orch Claude session in the same worktree can flip a task to `Working`. That is exactly the kind of leak the per-session design was meant to prevent.
- WHERE: `src/state.rs:279-305`, `claude/.claude/hooks/orch-busy-start.sh:27-34`
- SEVERITY: important
- SUGGESTED FIX: Stop using cwd as the join key; use session-id correlation so unrelated local Claude sessions cannot affect orch state.

### S3: “Warn and continue” is too permissive for archive failure
- WHAT: If archiving the task markdown fails, `cmd_close` still proceeds to worktree cleanup and legacy state deletion. That can destroy the only durable task handle while the archive step failed.
- WHERE: `src/main.rs:647-680`
- SEVERITY: important
- SUGGESTED FIX: Keep warn-and-continue for tmux/worktree cleanup, but treat archive failure as a hard stop before any further destructive cleanup.

### S4: The hook scripts intentionally hide malformed-input failures
- WHAT: `set -eu` will surface real shell/Python execution errors, but the Python blocks swallow JSON-parse failure and missing `session_id` with `exit 0`. That makes hook-payload regressions silent.
- WHERE: `claude/.claude/hooks/orch-busy-start.sh:18-26`, `claude/.claude/hooks/orch-busy-stop.sh:12-20`
- SEVERITY: nit
- SUGGESTED FIX: Keep the fast/no-stdout behavior, but log malformed payloads to stderr or a debug file under an opt-in env var so contract breaks are diagnosable.

## Verdict

The implementation is not production-ready as claimed. The largest defects are structural, not polish: the new store is only partially authoritative, the lifecycle model still mutates from runtime, busy detection violates the per-session contract, and `orch close` does not preserve the v2 state/history guarantees. The nav redesign is directionally sound and is a real simplification over the current 3-focus-cycle model, but the proposal needs one cleanup pass before implementation because parts of its diagnosis are stale and its `Esc` rules are not stated consistently.
