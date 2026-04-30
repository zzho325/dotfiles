# Linear Panel — TUI Design (Exploratory, Superseded)

> **Status:** Exploratory. The chosen minimal Linear panel lives in
> `redesign.md` §4 (Linear). This document is preserved as the broader
> exploration the redesign was distilled from — useful for understanding what
> was considered (scope cycling, transition modal, comment composer,
> multi-link strip, project/cycle/chain views) and explicitly cut. **Do not
> implement from this document.** Implementation reference is `redesign.md`.

---

Design proposal for the `Linear` tab in the right details pane. Read-mostly,
hierarchy-aware, opinionated cuts on write ops. Mockups assume ~80 cols × 28
rows unless noted.

## 1. Design rules

1. **Persisted links are floor.** `links.linear_issues[]` always renders. The
   cache only enriches.
2. **Hierarchy is real.** Sub-issue / parent / project / cycle aren't
   decorations — they're how the user navigates Linear daily.
3. **Default frame is the linked issue's neighborhood**, not the whole
   workspace. Wider scope is opt-in.
4. **Personal tool.** Cut every write op that doesn't earn its keymap row.

## 2. Linear hierarchy → orch model

```
Workspace
  ├─ Initiative (cross-team)
  │    └─ Project ────┐
  ├─ Team             │
  │    ├─ Project ◄───┘  (project belongs to team, can roll up to initiative)
  │    │    ├─ Milestone
  │    │    └─ Issue
  │    │         ├─ Sub-issue
  │    │         │    └─ Sub-sub-issue
  │    │         └─ Sub-issue
  │    └─ Cycle (sprint window)
  │         └─ Issue        (issue can be in cycle AND project)
  └─ Issue (orphan, no project, no cycle)
```

orch only persists `linear_issues[]` — the rest is reconstructed from cache.
Tree edges (parent/child, project membership, cycle membership) live in cache,
keyed by issue ID.

## 3. Tree rendering

### Glyphs and indentation

| Token | Use |
|-------|-----|
| `▼` | expanded node with children |
| `▶` | collapsed node with children |
| `·` | leaf (no children) |
| `├─` `└─` | tree connectors (only when ≥2 levels deep) |
| `★` | the linked issue (anchor of the view) |
| `◆` | currently focused issue in tree |
| `·` after key | unread comments / activity since last view |

Indentation: 2 cols per level. Connectors only rendered for depth ≥ 2 to keep
shallow trees airy.

```
▼ ★ ENG-1234  Migrate batch import    ◐ In Progress
  ├─ ▶ · ENG-1235  Schema rollout      ● Done
  ├─ ▼ · ENG-1236  Worker rewrite      ◐ In Progress
  │    └─ · ENG-1240  Add busy hooks   ○ Todo
  └─ · ENG-1237  Backfill              ○ Todo
```

### State glyphs (Rosé Pine Dawn)

| State | Glyph | Color (RPD) |
|-------|-------|-------------|
| Backlog | `○` | `subtle` (#797593) |
| Todo | `○` | `text` (#575279) |
| In Progress | `◐` | `gold` (#ea9d34) |
| In Review | `◑` | `iris` (#907aa9) |
| Done | `●` | `pine` (#286983) |
| Canceled | `⊘` | `muted` (#9893a5) |
| Triage | `?` | `love` (#b4637a) |

Priority: optional `!` `!!` `!!!` prefix on the title, only for P1/P2/P0.
Skip P3/P4 — too much noise.

### Color treatment

- Anchor row (`★`): `love` foreground, no background.
- Focused row (`◆`): `HL_LOW` background.
- Selected scope ancestor (project/cycle of the anchor): `iris` foreground on
  the row's title, dimmed connectors.
- Done/Canceled rows: dimmed to `muted` so they recede.
- Updated within last hour: `rose` (#d7827e) accent on the state glyph only.

### Truncation

Title column = `pane_width - prefix_width - state_width - 2`. Prefix =
indent + connector + glyph + key. Single-line ellipsis with `…`. Never wrap
titles in tree mode — wrapping breaks the visual indentation.

If the issue key + ` ` + 8 chars of title don't fit, drop the key and rely on
focus to show it in the detail block below.

## 4. Scope / framing

### Scope levels

The panel has one selected **scope** that determines what's in the tree. Cycle
through scopes with `S`.

| Scope | Tree contents | When useful |
|-------|---------------|-------------|
| `Issue` (default) | anchor + its sub-issues, recursive | flow #1 — sub-issues are work units |
| `Family` | anchor + parent + all siblings + own sub-issues | flow #2 — sibling context |
| `Project` | whole project tree, anchor highlighted | "what else is shipping with this" |
| `Cycle` | issues in same cycle, grouped by status | sprint check-in |
| `Chain` | initiative → project → … → anchor (linear breadcrumb only) | flow #4 — strategic context |

Default scope is `Issue` for a single-issue link, `Project` when an orch task
links 2+ issues in the same project, `Family` otherwise.

### Multi-issue link tasks

When `links.linear_issues.len() > 1`:

- Top of panel shows a **link strip** (one row per linked issue, keyed
  `1` `2` `3`…).
- Number key jumps the tree to that anchor; tree still uses the active scope.
- If all linked issues share a project, default scope switches to `Project`.
- If they span projects, scope is `Issue` and the strip is the only way to
  switch anchors.

```
Linked: [1] ★ENG-1234  [2] ENG-1240  [3] DESIGN-77
─────────────────────────────────────────────────────
▼ ★ ENG-1234 …
```

### Multi-project case

Tree groups by project at depth 0:

```
▼ project: Batch Import Hardening
  └─ ▼ ★ ENG-1234 …
▼ project: Design System v2
  └─ · DESIGN-77 …
```

## 5. Layout within the tab

The Linear tab itself splits into **tree (top, ~60%)** and **detail block
(bottom, ~40%)**. Both share the right-top details pane.

```
┌─ Linear ───────────────────────────── scope: Issue   ⟳ 12s ago ┐
│ Linked: [1] ★ENG-1234                                          │
│ ────────────────────────────────────────────────────────────── │
│ ▼ ★ ENG-1234  Migrate batch import         ◐ In Progress       │
│   ├─ ▶ · ENG-1235  Schema rollout          ● Done              │
│   ├─ ▼ ◆ ENG-1236  Worker rewrite          ◐ In Progress       │
│   │    └─ · ENG-1240  Add busy hooks       ○ Todo              │
│   └─ · ENG-1237  Backfill                  ○ Todo              │
│                                                                │
│ ────────────────────────────────────────────────────────────── │
│ ENG-1236 · Worker rewrite                                      │
│ State: In Progress    Assignee: @ashley    P2  est: 5          │
│ Project: Batch Import Hardening   Cycle: 2026-W18              │
│ Parent: ENG-1234   Updated: 2h ago                             │
│                                                                │
│ ▌ Latest comment (4h ago) — @robin                             │
│ ▌ Pushed first cut, looking at the worker registration path…   │
│                                                                │
│ [o] open  [t] transition  [c] comment  [r] refresh  [?] keys   │
└────────────────────────────────────────────────────────────────┘
```

Detail block content (focus-dependent):

- **Issue focused:** identity row, project/cycle/parent, latest comment, last
  transition.
- **Project focused:** project name, lead, status, milestone progress bar.
- **Initiative focused:** name, target date, owner, rolled-up status counts.

## 6. Navigation

### Within the Linear tab

| Key | Action |
|-----|--------|
| `j` / `k` | next / prev visible row |
| `J` / `K` | jump to next / prev sibling at same depth |
| `h` | collapse current node (or jump to parent if already collapsed/leaf) |
| `l` | expand current node (or drill into first child if already expanded) |
| `gg` / `G` | top / bottom of tree |
| `*` | jump to anchor (the `★` issue) |
| `u` | jump to parent |
| `S` | cycle scope: Issue → Family → Project → Cycle → Chain → Issue |
| `1`…`9` | jump to N-th linked issue (multi-link case) |
| `Tab` | toggle focus between tree and detail block |
| `/` | filter tree (substring match on title + key) |
| `f` | toggle "hide Done/Canceled" |

### Focus handoff with the rest of the TUI

- `Tab` / `Shift-Tab` at the panel root cycles through panes (list → details
  → log) per the global keymap.
- Inside the Linear tab, `Tab` is local: tree ↔ detail block.
- `h` / `l` at the **details tab bar** still switches tabs (Overview ↔ PRs ↔
  Linear ↔ Panes). Inside the Linear tab tree, `h` / `l` are tree controls —
  collapsing the root with `h` again pops focus back to the tab bar.

This is the most contentious key choice. Alternatives considered:

| Option | Pro | Con |
|--------|-----|-----|
| `h`/`l` collapse-or-tab-switch | matches vim feel, no new keys | overload risk |
| `<` / `>` for tab switch, `h`/`l` for tree | clean | breaks global keymap |
| `H` / `L` for tab switch | no overload | shadows top/bot in some terminals |

**Recommendation:** keep `h`/`l` overloaded. Collapse-then-pop is what vim
file trees do (e.g. NvimTree), the user already knows it.

## 7. Operations

Keep the surface tiny. Every write op needs a clear "I'd otherwise switch
to the browser" justification.

### Included

| Op | Key | Why kept |
|----|-----|----------|
| Open in browser | `o` | Always needed — Linear's web UI is richer for any deep work |
| State transition | `t` | The single most-used Linear write. Confirmation is fast |
| Comment | `c` | Worker handoff (flow #3) needs this; otherwise context-switch tax is high |
| Refresh | `r` | Network model demands it |
| Toggle hide-done | `f` | View, not write — included for cheapness |

### Cut

| Op | Why cut |
|----|---------|
| Assign | Single-user tool — assignee is almost always the user |
| Set priority | Rarely changes mid-flight; do it in browser |
| Edit title/description | Heavy editor surface, low value |
| Create sub-issue | Real value, but enough UI to deserve its own design pass — defer |
| Link/unlink to orch task | Belongs on the **Overview** tab, not Linear tab |
| Set estimate / labels / cycle / project | All low-frequency; web UI |
| Delete | Never |

### Transition flow (`t`)

Single popup over the detail block:

```
┌─ Transition ENG-1236 ─────────────────────┐
│  ◐ In Progress  → ?                       │
│                                           │
│  [1] ○ Todo                               │
│  [2] ◐ In Progress      (current)         │
│  [3] ◑ In Review                          │
│  [4] ● Done                               │
│  [5] ⊘ Canceled                           │
│                                           │
│  Esc: cancel    1-5: select               │
└───────────────────────────────────────────┘
```

- Number key fires immediately, no second confirmation. Reasoning: state
  transitions are reversible in Linear and the popup itself is the
  confirmation.
- States are loaded from the issue's team workflow (cached). If team workflow
  is unknown, fall back to the canonical 6 above and let Linear API reject.
- On API error: keep the cache optimistic-write for 5s, then revert with a
  toast in the log pane.

### Comment flow (`c`)

Opens a modal with a multi-line input. `Ctrl-S` to submit, `Esc` to cancel.
Pre-fills with the user's last drafted comment if the modal was previously
canceled (drafts persisted in `.orch/cache/linear-drafts.json` keyed by issue
ID — survives crashes).

```
┌─ Comment on ENG-1236 ──────────────────────────────────┐
│ Handing this off to the remote worker. Latest WIP on   │
│ branch ashley/eng-1236-worker. Open question: should   │
│ we co-locate the busy marker emit with the existing… █ │
│                                                        │
│ Ctrl-S: post     Esc: cancel (saves draft)             │
└────────────────────────────────────────────────────────┘
```

### Open (`o`)

If anchor focused, opens that issue. If a project/initiative node focused,
opens that. If multiple linked issues, anchor wins by default; press `1`…`9`
first then `o` to open a specific one.

### What's intentionally NOT a popup

- State transition is the only modal that fires on a single keystroke. Comment
  needs the buffer. Everything else is in-place.

## 8. Refresh & cache

### Cache shape

`linear.json` is a single file with three top-level maps and a per-team
metadata block:

```
{
  "version": 1,
  "fetched_at": "2026-04-30T17:42:11Z",
  "issues": {
    "ENG-1234": {
      "id": "uuid…",
      "key": "ENG-1234",
      "title": "Migrate batch import",
      "state": { "name": "In Progress", "type": "started" },
      "assignee": "ashley",
      "priority": 2,
      "estimate": 5,
      "parent_key": null,
      "child_keys": ["ENG-1235", "ENG-1236", "ENG-1237"],
      "project_id": "proj-uuid",
      "cycle_id": "cycle-uuid",
      "milestone_id": null,
      "team_key": "ENG",
      "url": "https://linear.app/…",
      "updated_at": "2026-04-30T15:42:00Z",
      "fetched_at": "2026-04-30T17:42:11Z",
      "latest_comment": { "author": "robin", "body": "…", "at": "…" }
    }
  },
  "projects": {
    "proj-uuid": {
      "name": "Batch Import Hardening",
      "team_key": "ENG",
      "initiative_id": "init-uuid",
      "lead": "ashley",
      "status": "in_progress",
      "issue_keys": ["ENG-1234", "ENG-1235", …],
      "milestone_ids": [],
      "fetched_at": "…"
    }
  },
  "initiatives": {
    "init-uuid": { "name": "Q2 Hardening", "project_ids": [...], "fetched_at": "…" }
  },
  "cycles": {
    "cycle-uuid": { "name": "2026-W18", "team_key": "ENG", "starts_at": …, "ends_at": …, "issue_keys": [...] }
  },
  "team_workflows": {
    "ENG": { "states": [...] }
  }
}
```

Tree edges live as `parent_key` + `child_keys` on the issue (denormalized both
ways). This means rendering an issue's subtree is one map lookup per node —
no GraphQL recursion at render time.

### Per-fetch granularity

| Trigger | Fetch |
|---------|-------|
| Daemon tick (every 2 min) | All `linked_issues[]` across open tasks, plus their parents and children, in a single GraphQL query |
| Task selection, cache > 30s | The task's linked issues + immediate family (parent + children + project metadata) |
| Manual `r` | Same as task selection but force-bypasses staleness check |
| Scope switch to `Project` | If `project.issue_keys` is missing or stale > 5 min, fetch full project issue list |
| Scope switch to `Cycle` | If cycle issue list missing or stale > 5 min, fetch |
| Scope switch to `Chain` | Project → initiative lookup; cheap |

GraphQL batching: one query per refresh tick, fan-out via fragments. Avoid
N+1 by always fetching `parent`, `children { id }`, `project { id }`,
`cycle { id }` in the issue fragment.

### Stale-edge behavior

When the user expands a node and `child_keys` is missing or older than 10
min, render with a **dim hint**:

```
▼ ENG-1234  Migrate batch import         ◐ In Progress
  └─ ⟳ loading sub-issues…  (last seen 14 min ago)
```

If the API call fails:

```
▼ ENG-1234  Migrate batch import         ◐ In Progress
  └─ ⚠ stale (last seen 14 min ago)  [r to retry]
```

Never blank out — always show last-known children with a stale badge.

### Header status

Top-right of the panel:

```
scope: Issue   ⟳ 12s ago        (fresh, < 30s)
scope: Issue   ⟳ 4m ago         (warm, dimmed)
scope: Issue   ⚠ 18m ago        (stale, love color)
scope: Issue   ⊘ disconnected   (no key / API down)
```

## 9. Edge cases

| Case | Behavior |
|------|----------|
| Orphaned issue (no project) | Tree just shows it; `Chain` scope shows only the issue |
| Cross-team issue | Team key prefix differentiates (`ENG-1234` vs `DESIGN-77`); no special grouping unless multiple linked across teams |
| Issue in cycle AND project | Default scope is `Issue`. Detail block shows both. `S` cycles between Project and Cycle scopes. Pick whichever you're using right now |
| Very deep sub-issue chain | Indent caps at 6 levels; deeper levels render flat with a `…N more` breadcrumb on the parent. Use `l` to descend further (re-roots tree at that node) |
| Linked issue is closed/canceled | Render with dimmed text + `⊘` glyph. Don't auto-unlink. Detail block shows close reason if present |
| User unlinks mid-cycle | Tree empties to "no linked Linear issues". Cache entry preserved for 24h in case of relink |
| All linked issues canceled | Tree shows them dimmed, detail block shows a "all linked work canceled — consider closing this orch task" hint |
| Parent issue not in cache | Render the issue's tree without parent; show `parent: ENG-1234 (not loaded)` in detail block; trigger fetch on next tick |
| Anchor moved (parent reassigned in Linear) | Cache update naturally fixes it on next refresh; tree may briefly show old structure |

## 10. Mockups

### A. Default — single linked issue with sub-issues

```
┌─ Linear ───────────────────────────── scope: Issue   ⟳ 12s ago ┐
│ Linked: [1] ★ENG-1234                                          │
│ ────────────────────────────────────────────────────────────── │
│ ▼ ★ ENG-1234  Migrate batch import         ◐ In Progress       │
│   ├─ ▶ · ENG-1235  Schema rollout          ● Done              │
│   ├─ ▼ ◆ ENG-1236  Worker rewrite          ◐ In Progress       │
│   │    └─ · ENG-1240  Add busy hooks       ○ Todo              │
│   └─ · ENG-1237  Backfill                  ○ Todo              │
│                                                                │
│ ────────────────────────────────────────────────────────────── │
│ ENG-1236 · Worker rewrite                                      │
│ State: In Progress    Assignee: @ashley    P2  est: 5          │
│ Project: Batch Import Hardening   Cycle: 2026-W18              │
│ Parent: ENG-1234   Updated: 2h ago                             │
│                                                                │
│ ▌ Latest (4h ago) — @robin                                     │
│ ▌ Pushed first cut, looking at the worker registration path…   │
│                                                                │
│ [o] open  [t] transition  [c] comment  [r] refresh  [?] keys   │
└────────────────────────────────────────────────────────────────┘
```

### B. Expanded — initiative → project → issue → sub-issues (`S` to Chain, then expand)

```
┌─ Linear ──────────────────────────── scope: Chain   ⟳ 41s ago ┐
│ Linked: [1] ★ENG-1234                                         │
│ ───────────────────────────────────────────────────────────── │
│ ▼ initiative: Q2 Hardening                                    │
│   ▼ project: Batch Import Hardening      ◐ in_progress  3/8   │
│     ▼ ★ ENG-1234  Migrate batch import   ◐ In Progress        │
│       ├─ · ENG-1235  Schema rollout      ● Done               │
│       ├─ ▼ ◆ ENG-1236  Worker rewrite    ◐ In Progress        │
│       │    └─ · ENG-1240  Add busy hooks ○ Todo               │
│       └─ · ENG-1237  Backfill            ○ Todo               │
│                                                               │
│ ───────────────────────────────────────────────────────────── │
│ ENG-1236 · Worker rewrite                                     │
│ (as above)                                                    │
└───────────────────────────────────────────────────────────────┘
```

### C. Multi-issue link, multi-project

```
┌─ Linear ───────────────────────────── scope: Issue   ⟳ 8s ago ┐
│ Linked: [1] ★ENG-1234  [2] ENG-1240  [3] DESIGN-77            │
│ ───────────────────────────────────────────────────────────── │
│ ▼ project: Batch Import Hardening                             │
│   ├─ ▼ ★ ENG-1234  Migrate batch import  ◐ In Progress        │
│   │    └─ · ENG-1240  Add busy hooks     ○ Todo  ←linked      │
│   └─ · ENG-1235  Schema rollout          ● Done               │
│ ▼ project: Design System v2                                   │
│   └─ · ◆ DESIGN-77  Token rename pass    ◑ In Review          │
│                                                               │
│ ───────────────────────────────────────────────────────────── │
│ DESIGN-77 · Token rename pass                                 │
│ State: In Review      Assignee: @ashley   P3                  │
│ Project: Design System v2   Cycle: —                          │
│ Updated: 1d ago                                               │
└───────────────────────────────────────────────────────────────┘
```

The `←linked` annotation only appears in scopes wider than `Issue` and only
when the row is one of the linked issues but not the current anchor. It
disambiguates "which of these am I working on right now."

### D. Transition flow

```
┌─ Linear ───────────────────────────── scope: Issue   ⟳ 12s ago ┐
│ Linked: [1] ★ENG-1234                                          │
│ ────────────────────────────────────────────────────────────── │
│ ▼ ★ ENG-1234  Migrate batch import         ◐ In Progress       │
│   └─ ▼ ◆ ENG-1236  Worker rewrite          ◐ In Progress       │
│                                                                │
│   ┌─ Transition ENG-1236 ────────────────┐                     │
│   │  ◐ In Progress  → ?                  │                     │
│   │                                      │                     │
│   │  [1] ○ Todo                          │                     │
│   │  [2] ◐ In Progress      (current)    │                     │
│   │  [3] ◑ In Review                     │                     │
│   │  [4] ● Done                          │                     │
│   │  [5] ⊘ Canceled                      │                     │
│   │                                      │                     │
│   │  Esc: cancel    1-5: select          │                     │
│   └──────────────────────────────────────┘                     │
└────────────────────────────────────────────────────────────────┘
```

After `3` is pressed:

```
│ ▼ ★ ENG-1234  Migrate batch import         ◐ In Progress       │
│   └─ ▼ ◆ ENG-1236  Worker rewrite          ◑ In Review  ⟳      │
```

The trailing `⟳` denotes optimistic update pending API ack. Cleared on next
refresh tick, or replaced with `⚠` on failure.

### E. Empty / no linked issues

```
┌─ Linear ─────────────────────────────────────── ⟳ 12s ago ┐
│                                                           │
│   No linked Linear issues.                                │
│                                                           │
│   Auto-discovery scans task title + slug + markdown for   │
│   issue keys (e.g. ENG-1234). Add manually with:          │
│                                                           │
│     orch link <task> <issue-key>                          │
│                                                           │
│   Or paste a Linear URL into the task markdown and run    │
│   refresh (r).                                            │
│                                                           │
└───────────────────────────────────────────────────────────┘
```

### F. Disconnected (no LINEAR_API_KEY or API down)

```
┌─ Linear ────────────────────────── ⊘ disconnected ─────────┐
│ Linked: [1] ENG-1234  [2] DESIGN-77                        │
│ ────────────────────────────────────────────────────────── │
│ · ENG-1234   (cached title, state unknown)                 │
│ · DESIGN-77  (cached title, state unknown)                 │
│                                                            │
│ ────────────────────────────────────────────────────────── │
│ Linear API unreachable.                                    │
│ Last successful refresh: 2h ago.                           │
│ Reason: LINEAR_API_KEY not set                             │
│                                                            │
│ Persisted links still rendered. Run `r` after fixing.      │
└────────────────────────────────────────────────────────────┘
```

Note: tree is flat in this state — no edges, no states. The cache is
authoritative for what we have, and we have only the keys + last-known
titles.

### G. Stale cache (warm)

```
┌─ Linear ───────────────────────── scope: Issue   ⟳ 14m ago ┐
│ Linked: [1] ★ENG-1234                                      │
│ ────────────────────────────────────────────────────────── │
│ ▼ ★ ENG-1234  Migrate batch import       ◐ In Progress     │
│   ├─ · ENG-1235  Schema rollout          ● Done            │
│   ├─ ▼ ◆ ENG-1236  Worker rewrite        ◐ In Progress     │
│   │    └─ ⟳ loading sub-issues…  (last seen 14m ago)       │
│   └─ · ENG-1237  Backfill                ○ Todo            │
│                                                            │
│ ────────────────────────────────────────────────────────── │
│ ENG-1236 · Worker rewrite                                  │
│ (cached 14m ago)                                           │
│ State: In Progress    Assignee: @ashley    P2              │
└────────────────────────────────────────────────────────────┘
```

The header `⟳ 14m ago` renders in `love` to flag staleness. Auto-refresh on
focus enter if > 30s, but render stale data first — never block on network.

## 11. Open questions / future work

- **Sub-issue creation:** highest-value cut. If kept, it'd be a one-line
  modal pinned to the parent. Defer to a v2 design once the read flows are
  validated.
- **Notifications:** Linear's `unread` per-issue could feed the count badge
  in the task list. Needs a separate fetch endpoint; defer.
- **Search:** the `/` filter is local-only. A workspace search ("issues
  matching X") is a different surface — probably belongs in a command palette,
  not the panel.
- **Bidirectional link surfacing:** "this Linear issue references PR #4567 —
  jump to PRs tab?" — nice but not essential. Cache the PR URLs Linear knows
  about and add a one-line `Linked PRs:` row in the detail block when
  present.
- **Estimate / cycle progress:** showing cycle progress bar in `Cycle` scope
  would be cheap — issue count by state is already in cache. Worth doing in
  the same pass.

## 12. Summary of opinionated cuts

| Decision | Rationale |
|----------|-----------|
| Default scope = `Issue`, not whole workspace | The user's most common ask is "show me my sub-issues" |
| 5 scopes via `S` cycle, not separate keys | Discoverability via single key; muscle memory cheap |
| Write ops: open, transition, comment only | Anything else is a browser-tier interaction |
| Tree renders even when cache is stale or API is down | Persisted links are the floor |
| `h`/`l` overloaded for tree + tab switch (collapse-then-pop) | Vim convention; alternatives worse |
| Multi-link case uses `1`…`9` strip rather than picker modal | One keystroke beats two |
| Optimistic write for state transitions | Revert on failure with toast — feels instant |
| Issue parent/child denormalized in cache | Render is map lookups, not GraphQL recursion |
| Initiative scope is breadcrumb only, not full tree | Initiatives have too many projects to render usefully |
