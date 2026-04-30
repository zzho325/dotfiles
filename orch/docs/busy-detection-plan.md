# orch: hook-driven busy detection

## Problem

Current Ready/Working detection hashes `tmux capture-pane` output + cursor
position and compares across polls (`state.rs:179-267`). Under Claude Code's
`prefersReducedMotion: true` + empty `spinnerVerbs`, the visible frame is
stable while Claude is working — hash unchanged → false **Ready**.

Cursor-position fallback was added for this exact case but is insufficient
because reduced motion also suppresses cursor movement during API waits.

## Goal

Replace pane-hash detection with a marker file written/removed by Claude Code
hooks. Stop calling `tmux capture-pane` entirely. This plan owns *only* the
busy-marker mechanics — the surrounding state model is the redesign's
`desired_state` + runtime-badge derivation (`redesign.md` §2). Where this
plan's older vocab (`Idle` / `Ready` / `Working`) appears below, treat it as
shorthand for the redesign's badges.

**Success criteria**
- `Working` is reported for the full lifetime of a Claude turn, including the
  API-wait window before the first token streams.
- `Ready` is reported within one poll tick after `Stop` fires.
- No `tmux capture-pane` call remains in orch.
- Markers do not leak: orphaned markers are cleaned up automatically.

## Design

### Marker contract

Claude Code hooks write / remove a per-session marker:

- Path: `$XDG_RUNTIME_DIR/orch/busy/<session_id>` (fallback `/tmp/orch/busy/<sid>` on macOS)
- Content: one line of JSON `{"cwd": "...", "started_at": "<RFC3339>", "pid": <claude_pid>}`
- Lifecycle:
  - `UserPromptSubmit` → create marker
  - `Stop` → delete marker
  - `SessionEnd` → delete any marker for this session_id (belt-and-suspenders)

Key design choice: **session_id**, not cwd. A user may run a second Claude
session in the same worktree (manual debug, etc.); keying by session_id keeps
them independent.

orch correlates tmux session → claude session_id via the **pane env var**
`CLAUDE_SESSION_ID`, which Claude Code exports per session. orch queries it
with `tmux show-environment -t <pane> CLAUDE_SESSION_ID` (cached per poll).

### Staleness guard

If Claude crashes between `UserPromptSubmit` and `Stop`, the marker sticks.
Two defenses:
1. Marker mtime > 30 min → orch treats as stale, reports `Ready`, and best-
   effort `rm`s it.
2. orch on startup sweeps `/tmp/orch/busy/*` older than 30 min.

30 min chosen as comfortably larger than the longest turn we'd expect in
practice. Tunable via `ORCH_BUSY_STALE_SECS` env var.

### Marker -> redesign badge mapping

This plan's responsibility ends at "is the marker fresh". The redesign's
badge derivation (see `redesign.md` §2 *Badge derivation matrix*) consumes
that signal. Specifically:

- `busy_marker(session_id)` fresh -> `Working` row in the matrix
- `busy_marker` absent or stale -> `Ready` row (modulo `Attached` / `Input`
  taking higher precedence)

`has_active_process` (via `tmux list-panes` `pane_current_command` check)
feeds the matrix's `worker_alive` column — it's the "is claude even
running" sanity check, distinct from `session present` (which only asks
whether the tmux session itself exists). It does not need `capture-pane`.

### What gets ripped out

- `TmuxSession.pane_hash` field
- The capture-pane branch of `load_pane_info` (lines 210-226)
- `prev_hashes` parameter to `load_tasks` and its caller in `tui.rs`
- Hash-comparison logic in `derive_status` (lines 258-266)

### What gets added

- `TmuxSession.claude_session_id: Option<String>`
- `fn busy_marker_path(sid: &str) -> PathBuf`
- `fn is_busy(sid: &str, stale_secs: u64) -> bool` — exists + mtime fresh
- Startup sweep in `main.rs` (or wherever orch boots)
- `hooks/orch-busy.sh` script + settings.json registrations

## Implementation steps

### Phase 1 — hook scripts (pure addition, no orch change)

**a)** Create `~/dotfiles/claude/.claude/hooks/orch-busy-start.sh`:
```sh
#!/bin/sh
set -eu
dir="${XDG_RUNTIME_DIR:-/tmp}/orch/busy"
mkdir -p "$dir"
# Hook JSON is on stdin; session_id at .session_id, cwd at .cwd.
# Avoid jq dep: use python3 (always present on macOS).
python3 - <<'PY'
import json, os, pathlib, sys, time
d = json.load(sys.stdin)
sid = d.get("session_id")
if not sid:
    sys.exit(0)
p = pathlib.Path(os.environ.get("XDG_RUNTIME_DIR", "/tmp")) / "orch" / "busy" / sid
p.parent.mkdir(parents=True, exist_ok=True)
p.write_text(json.dumps({
    "cwd": d.get("cwd", ""),
    "started_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    "pid": os.getpid(),
}))
PY
```

**b)** `orch-busy-stop.sh` — mirror, but `p.unlink(missing_ok=True)`.

**c)** Register in `~/dotfiles/claude/.claude/settings.json`:
```json
"hooks": {
  "UserPromptSubmit": [{"hooks": [{"type": "command",
    "command": "~/.claude/hooks/orch-busy-start.sh"}]}],
  "Stop":             [{"hooks": [{"type": "command",
    "command": "~/.claude/hooks/orch-busy-stop.sh"}]}],
  "SessionEnd":       [{"hooks": [{"type": "command",
    "command": "~/.claude/hooks/orch-busy-stop.sh"}]}]
}
```

**Validate phase 1 standalone**: start a claude session, submit a prompt, `ls
/tmp/orch/busy/` — marker present. Wait for completion, marker gone.

### Phase 2 — orch read path

**a)** `state.rs`:
- Add `claude_session_id: Option<String>` to `TmuxSession`
- New `fn load_pane_info` returns `(HashSet<session_name>, HashMap<session_name, claude_sid>)`
  - Single `tmux list-panes -a -F '#{session_name} #{pane_current_command} #{pane_id}'` call
  - For each worker pane, one `tmux show-environment -t <pane_id> CLAUDE_SESSION_ID`
    (falls back to None if unset; `show-environment` is cheap, local socket call)
- Drop all capture-pane + hash logic
- `derive_status` new signature: `(meta, sessions, busy_stale_secs) -> TaskStatus`
  - Implementation per the status derivation box above

**b)** `tui.rs` (and daemon equivalents): remove `prev_hashes` threading.

**c)** `main.rs` startup: one-shot sweep of `$BUSY_DIR/*` removing files with
mtime older than `ORCH_BUSY_STALE_SECS` (default 1800).

### Phase 3 — daemon parity

Daemon caches `status.json`. Its status computation path is the same
`derive_status` call — phase 2 changes cover it. Verify by reading
`cache.rs` / daemon polling loop and confirming no separate hash logic.

## Test plan

### Rust unit tests (`state.rs` tests module)

1. `is_busy` returns true for a fresh marker file
2. `is_busy` returns false for a marker older than `stale_secs`
3. `is_busy` returns false when file is absent
4. `derive_badge` (cases follow the matrix in `redesign.md` §2):
   - `Active` + no session → `Detached` + `session_missing` drift
   - `Paused` + no session → `Paused`, no drift
   - `Active` + session + worker_alive=no → `Error` + `worker_dead` drift
   - `Active` + session + worker_alive=yes + attached → `Attached`
   - `Active` + session + worker_alive=yes + needs_input → `Input`
   - `Active` + session + worker_alive=yes + fresh marker → `Working`
   - `Active` + session + worker_alive=yes + stale marker → `Ready`
   - `Active` + session + worker_alive=yes + no marker → `Ready`
   - `Paused` + session → `Paused` + `cleanup_pending` drift
5. `load_pane_info` parses `CLAUDE_SESSION_ID` correctly from mocked tmux
   output (use a small parse helper so this doesn't need a real tmux)

### Shell integration test (`tests/hook-smoke.sh`)

1. Mktemp a busy dir, point `XDG_RUNTIME_DIR` at it
2. Simulate hook input: `echo '{"session_id":"abc","cwd":"/tmp"}' | hooks/orch-busy-start.sh`
3. Assert `$dir/orch/busy/abc` exists and contains valid JSON
4. Run `orch-busy-stop.sh` with same input; assert file removed
5. Run start, `touch -A -003100 $file` (30min+1min old), run orch sweep,
   assert removed

### Manual end-to-end

1. Build orch, launch TUI
2. Spawn a worker: `orch new test-busy`, attach once, prompt Claude "say hi"
3. While Claude is mid-turn (submit a long task), orch TUI shows `Working`
4. After Stop fires, TUI flips to `Ready` within one poll (500ms-1s)
5. Kill the claude process inside the tmux session — TUI flips to `Error`
   with `worker_dead` drift within one poll
6. Kill the tmux session mid-turn (simulate crash) — marker leaks; within 30
   min (or force stale by editing `ORCH_BUSY_STALE_SECS=10`) TUI flips
   `Detached` with `session_missing` drift and the stale marker is swept

### Regression check

1. Run TUI against a pre-existing worker with no marker and no session_id
   env var → behaves as `Ready` when idle, no panic
2. Multiple workers in different sessions update independently

## Risks / open questions

1. **`CLAUDE_SESSION_ID` env var availability.** Confirm Claude Code actually
   sets this in the process env. If not, the session_id only comes from the
   hook payload — we'd need the hook to also write a cwd-keyed symlink
   (`/tmp/orch/busy/by-cwd/<slug> -> <sid>`) so orch can find the right
   marker from cwd. Fallback is straightforward; need to verify first.
2. **Hook exit codes.** Claude Code blocks on hook stdout/exit for some
   events. `UserPromptSubmit` in particular can be blocking. Ensure scripts
   exit 0 fast and write no stdout. Test with a deliberately broken hook to
   confirm it doesn't brick the session.
3. **Non-orch Claude sessions.** If user runs `claude` outside orch, its
   hooks still write markers. Harmless — orch only reads markers for sids it
   knows about.
4. **Python dependency in hook.** macOS ships python3. If a stripped-down
   environment doesn't have it, hook is a no-op (write fails silently). Could
   rewrite in pure sh + a tiny jq-or-awk parser, but python3 keeps it robust.
5. **Daemon polling cost.** Removing capture-pane removes an O(N_panes)
   shell-out per tick. Net win; confirm in benchmark if we care.
6. **Transition period.** Old orch binaries + new hooks: markers written,
   never read, eventually swept. Safe. New orch + old hooks: always Ready.
   Roll hooks first, orch second.

## Non-goals

- Changing `Input`, `Attached`, `Idle`, `Paused` detection
- Reworking the daemon's `status.json` schema
- Adding sub-states for "API wait" vs "streaming" — marker presence is
  enough; we just want reliable Working
