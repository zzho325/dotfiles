# Orch Implementation Status

Snapshot of what shipped from `redesign.md` / `redesign-notes.md` between
2026-04-30 and now. Pair this with `redesign.md` (the contract) and
`redesign-notes.md` (the phase plan with snapshot deliverables) to know
what is live, what is deferred, and where to look in the code.

## What's live

```
P0  Stability quick wins ............ ✅
P1  Persistence + FSM foundation .... ✅ (slices A-E; F deferred)
P2  PR display layer ................ ✅ (folded into P1+P3 by side effect)
P3  TUI rewrite + minimal Linear .... ✅
P4a Session rename .................. deferred
P4b Linear data layer ............... deferred
P5  Pane state polish ............... deferred
```

10 commits, ~5800 LOC, 91 tests passing.

## P0 — Stability quick wins

Commit: `1c72963 feat(orch): hook-driven busy detection, gc, close (P0)`

| Piece | What changed | Code |
|---|---|---|
| **Busy detection** | Replaced `tmux capture-pane` + pane-hash with marker files written by Claude Code hooks. `Working` flips within one poll tick of `UserPromptSubmit`; `Ready` flips within one tick of `Stop`. Stale markers (>30 min) auto-swept on startup + every 5 min. Tunable via `ORCH_BUSY_STALE_SECS`. | `state.rs::is_worktree_busy`, `state.rs::sweep_stale_markers`, `claude/.claude/hooks/orch-busy-{start,stop}.sh` |
| **`orch gc`** | Scans `$ORCH_REPO/task-*` for worktrees whose `~/tasks/<name>.md` is gone. Removes via `git worktree remove`; falls back to `rm -rf` for already-disowned dirs. Dirty trees warn with the override command. | `main.rs::find_orphan_worktrees`, `main.rs::cmd_gc` |
| **`orch close <name>`** | Kills tmux, archives `~/tasks/<name>.md` to `done/`, runs `git worktree remove` with retry/warning, drops `.state/<name>.json`. Each step warns on failure but does not abort. | `main.rs::cmd_close` |

Hook contract (per `docs/busy-detection-plan.md`):

```
$XDG_RUNTIME_DIR/orch/busy/<sid>   (falls back to /tmp/orch/busy on macOS)
  content: {"cwd": "...", "started_at": "<RFC3339>", "pid": <pid>}
  written:  UserPromptSubmit hook
  removed:  Stop / SessionEnd hooks
  swept:    orch on startup + every 5 min if mtime > stale_secs
```

orch reads the cwd from each fresh marker and matches against
`meta.worktree` (also accepting subdirectories). Avoids the
`CLAUDE_SESSION_ID`-env-var availability concern in the original plan.

## P1 — Persistence + FSM foundation

5 of 6 slices shipped. Slice F (background reconciler + stranded overlay
UI) deferred — `orch gc` covers the immediate pain, and Phase 3's TUI
rewrite would have thrown out any UI work anyway.

### Slice A: data structs (`8ce9c66`)

New `src/store.rs` with the v2 store data model. ~635 LOC, 12 tests.

```rust
TaskRecord {
    id: TaskId, slug, title, task_file, archived_task_file,
    created_at, started_at, paused_at, closed_at, updated_at,
    desired_state: DesiredState,            // New | Active | Paused | Closed
    attention: AttentionInfo,
    worktree: WorktreeInfo,                 // path, base_ref, cleanup_on_close
    tmux: TmuxInfo,                         // session_name, last_known_pane_id, rename_in_flight
    agent: AgentInfo,                       // mode, worker_kind
    links: Links,                           // prs, linear_issues, notes_urls
    drift: DriftFlags,                      // session_missing, worker_dead, ...
}

Registry {
    version: "v2",
    next_task_id, open_order, closed_order,
}
```

Layout on disk:

```
~/tasks/.orch/
├── runs/                   # untouched by migration
├── store.v2.tmp/           # staging (deleted on crash recovery)
├── store.v2/               # post-cutover authoritative
│   ├── registry.json
│   └── tasks/<id>.json
├── store.version           # one-line "v2"; absence = legacy mode
└── cache/
    ├── status.json
    ├── prs.json
    └── lease.json
```

`Store` handle has injectable `orch_root` so tests stay isolated from
the user's real `~/tasks/.orch/`.

### Slice B: gated read path (`02c0bf7`)

`load_task_meta()` checks `Store::default().is_authoritative()`; when
the marker exists it reads `TaskRecord` by slug and flattens via
`TaskMeta::from_record`. Without the marker, falls through to legacy
`.state/<name>.json`. Inert in production until slice C creates the
v2 store.

### Slice C: idempotent migration (`514adc2`)

`Store::migrate_from_legacy(tasks_dir)` runs the cutover with explicit
fsync at every step. Crash recovery via the `store.version` marker:
without it, leftover `store.v2.tmp/` and `store.v2/` are discarded and
migration re-runs.

```
1. Discard leftover .orch/store.v2.tmp/ and partial .orch/store.v2/.
2. Read .state/*.json + order.json + live tmux + task .md files.
3. Stage records under .orch/store.v2.tmp/{registry,tasks/<id>}.json,
   fsync each + the tmp dir.
4. Atomic rename store.v2.tmp -> store.v2.
5. fsync .orch/.
6. Write store.version=v2, fsync, fsync .orch/.
```

ID assignment: `order.json` entries first (in order), then remaining
`.md` task files alphabetically. Field mapping:

| Legacy → v2 | Notes |
|---|---|
| `file stem` → `slug` | |
| `.state/<n>.json.session` → `tmux.session_name` | |
| `.state/<n>.json.worktree` → `worktree.path` | |
| `.state/<n>.json.prs` → `links.prs` | source=Migration |
| `.state/<n>.json.needs_input` → `attention.needs_input` | |
| `.state/<n>.json.paused=true` → `desired_state=Paused` | |
| has session/worktree → `desired_state=Active` | else `New` |
| persisted session not in live tmux → `drift.session_missing=true` | |

### Slice D: bi-write + daemon migration (`ec55381`)

`save_task_meta` mirrors writes to v2 when authoritative. Loads the
existing `TaskRecord` by slug, calls `apply_task_meta_to_record` to
update only the TaskMeta-derived fields (preserving drift, agent
mode, manual PR link provenance). `Closed` records are not flipped
back to Active by stale TaskMeta saves.

`cmd_daemon` calls `Store::migrate_from_legacy` at startup. Idempotent
— short-circuits when the marker already exists.

### Slice E: Error badge for dead-worker (`ad53d89`)

Per `redesign.md` §2 matrix: `Active` session with no
claude/node/codex process is now `Error` (rendered in `LOVE` red),
distinct from `Ready` (worker alive and idle). Resume re-spawns
either way; the badge now reflects reality.

Was reading `Ready` before, masking the failure. ~3 lines of state.rs
+ 1 enum variant + 4 render sites updated.

## P3 — TUI rewrite + minimal Linear render

Commit: `b47ed53 feat(orch): three-pane TUI rewrite (Phase 3)`,
`5e49d01 fix(orch): TUI vertical separator + render-debug command`

New `src/tui3.rs` — ~1700 LOC, 24 snapshot/key tests. Replaces the
flat-list legacy TUI behind `ORCH_TUI=legacy` (rollback path).

```
┌──────────────┬──────────────────────────────────────┐
│ tasks list   │ Overview · PRs · Linear · Panes      │
│              │ ─────────────────────────────────── │
│  task rows   │ <selected tab content>              │
│              ├──────────────────────────────────────┤
│              │ log: latest run output (wrapped)     │
└──────────────┴──────────────────────────────────────┘
```

Behaviors:

- **Pane focus** cycles via `Tab` / `Shift-Tab` (List → Details → Log).
  Focused pane gets `LOVE` color on its header.
- **Detail tabs** switch via `h` / `l` when Details is focused.
- **In-app pane switching** (`[`, `]`, `\`) works inside the Panes tab.
  `Enter` attaches via `tmux switch-client + select-pane`. No more
  `tmux prefix+o`.
- **Log wraps** long lines (no truncation). `j/k`/`PgUp`/`PgDn` scroll.
  `G` re-pins follow_bottom; any other scroll disables it so the
  viewport stays where you put it.
- **Linear tab** renders from a stub built from
  `TaskRecord.links.linear_issues`. Slice 4b will replace the stub
  with the live cache.
- **Zero-task screen** keeps the three-pane layout intact: list shows
  `no tasks · n to create`, details `select a task`, log placeholder.
- **`?` toggles** the help overlay (any key dismisses).

Snapshot tests cover: three-pane base + 3 focus variants, all 4 detail
tabs, log wrap, log scroll preserve, pane-tab focus indicator, pane
selection, empty state, help overlay, Linear anchor+sub-issues, Linear
empty, key cycling, navigation, message input, total_wrapped_rows
math.

`orch render-debug --width N --height M [--tab T] [--focus F]` dumps
the rendered TUI to stdout via ratatui's TestBackend. Useful for
diagnosing layout without an interactive terminal.

## P2 — PR display layer

No dedicated commit — folded into P1 + P3 by side effect.

- `links.prs[]` persisted on `TaskRecord` with `source` provenance
  (Manual / BranchDiscovery / MarkdownScan / Migration). Slice C
  populates this from legacy on migration.
- `gh` cache (`cache::CachedPr`) enriches but never gates display.
- Phase 3's PRs tab renders `links.prs[]` directly; if the gh cache
  hasn't enriched, the PR row shows just `#NNN` with `· ci · review`
  default-state markers.

PR-discovery scan on branch change is still via `state::reconcile_prs`
(unchanged from pre-redesign — uses `jj op log` to detect pushes).

## What is live in production right now

| Surface | Where |
|---|---|
| `orch daemon` | running with the new binary; migrated 8 tasks to `~/tasks/.orch/store.v2/` |
| Marker hooks | symlinked into `~/.claude/hooks/`; fire on every Claude turn for every session |
| `~/tasks/.orch/store.version` | `v2` — reads come from the new store, writes mirror to both |
| TUI | new three-pane layout default; `ORCH_TUI=legacy orch` rolls back |
| Subcommands | `gc` (orphan worktrees), `close` (kill+archive+remove-wt), `render-debug` (dump TUI to stdout) |

## Pain points addressed (from original list)

| Pain | Status |
|---|---|
| "Not stable" | ✅ busy markers replace pane-hash; `worker_dead` → `Error` badge |
| "PR not show up" | ✅ TaskRecord owns `links.prs`; tab renders from links, not from gh-cache |
| "Switching needs `tmux prefix+o`" | ✅ Panes tab + `[`, `]`, `\` |
| "Number gaps" | ✅ data model has durable `task_id`; TUI shows current ordering |
| "Frequent start/close uncodified" | ✅ `orch close` shipped (kill + archive + remove-wt) |
| "Architecture from oldest forms" | ✅ full redesign + foundation slices |
| "Log truncated" | ✅ Phase 3 log wraps + scroll-preserves |
| "Linear integration" | partial — data model ready, stub render in TUI; Phase 4b ships real API |

## Deferred work

| Phase | Scope | Why deferred |
|---|---|---|
| **P1F** | Background reconciler + stranded-overlay UI | `orch gc` covers immediate pain; UI overlay would be thrown away by P3's TUI rewrite |
| **P4a** | Session rename + slug rules (`<slug>` only, drop `task-` prefix, staged 3-step rename op via `M` key) | Risky — renames live tmux sessions mid-flight. Needs careful rollout the user should drive. |
| **P4b** | Linear data layer (`LINEAR_API_KEY` env, GraphQL queries, cache, refresh, count badges, `linear add/rm` commands) | Pure addition but multi-hour. Stub in P3 demonstrates the rendering path. |
| **P5** | Pane state polish (persist + restore `last_known_pane_id` on resume) | Small win that benefits from P4a settling first. |

## Open questions (still in `redesign-notes.md`)

- Orphan worktree with uncommitted changes: stranded-overlay default
  is `rm -rf` — should `force-remove` first stash? Currently we just
  warn loudly via `git worktree remove`'s default error.
- Phase 1 migration trigger: auto on first launch (current default)
  vs explicit `orch migrate`. The `store.version` marker provides
  the safety net; auto seems fine.

## Files to know

```
orch/
├── docs/
│   ├── redesign.md              # contract (v2 store, FSM, matrix, etc.)
│   ├── redesign-notes.md        # phased plan + snapshot deliverables
│   ├── busy-detection-plan.md   # marker mechanics
│   ├── linear-tui-design.md     # exploratory; superseded
│   ├── redesign-review*.md      # codex review iterations 1-6
│   └── IMPLEMENTATION.md        # this file
├── src/
│   ├── main.rs                  # CLI, daemon, gc, close, render-debug
│   ├── state.rs                 # TaskMeta (legacy), TaskStatus, derive_status, busy markers
│   ├── store.rs                 # v2: TaskRecord, Registry, Store, migration
│   ├── tui.rs                   # legacy TUI (kept behind ORCH_TUI=legacy)
│   ├── tui3.rs                  # new three-pane TUI (default)
│   ├── cache.rs                 # status + PR caches written by daemon
│   ├── gh.rs                    # PR data fetching
│   └── runs.rs                  # orchestrator run history
└── claude/.claude/hooks/
    ├── orch-busy-start.sh       # marker write on UserPromptSubmit
    └── orch-busy-stop.sh        # marker remove on Stop / SessionEnd
```

## Validation

| Layer | Coverage |
|---|---|
| Unit tests | 91/91 pass (`cargo test`) — round-trips, migration, derive_status, marker logic, all snapshot tests |
| Hook smoke | Live tested: marker write → `working` flip; remove → `ready` flip; stale → `ready` despite present; subdir match; wrong cwd ignored |
| Migration smoke | 8 real tasks migrated on first daemon restart; `store.version=v2`; bi-write verified via `pr add/rm 99999` |
| TUI render | `render-debug` shows clean layout at 150×40 with vertical separator; all 4 detail tabs render; pane navigation works |
| Daemon | running pid 22692 (post-restart); `~/tasks/.orch/cache/status.json` updates every 2s |
