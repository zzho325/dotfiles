# Linear Panel — Deep View

Supersedes the Linear section in `redesign.md` §4 and the linked-list view
currently in `tui3.rs::render_tab_linear`. The minimal Phase 4b panel earned
its keep, but flat lines + bare key/state/assignee throw away most of what
Linear knows about an issue. This doc upgrades the Linear tab from "list
+ open in browser" to a real read surface — description, hierarchy, project
context — without breaking the two-zone focus contract.

## Summary

- **Linear tab gains a stack-state.** A boolean `linear_view` on `App`:
  `List` (default, multi-issue rollup) or `Detail(key)` (one issue, full
  page). `Enter` from `List` pushes `Detail`; `Esc` pops `Detail` back to
  `List`. Pop-then-pop returns focus to the task list, matching the
  canonical `Esc` rule.
- **No new tab, no new pane, no third focus level.** The "deep view" is
  inline within the Linear tab body. The Right zone keeps its 4-tab strip;
  number keys still jump tabs from anywhere.
- **List rows go from 2 lines → 2 lines, but earn their pixels.** Priority
  badge, state glyph, age, and a `▸` cursor land on row 1; project + sub-
  issue count on row 2. Same vertical density, real signal.
- **Detail view is one screen, not a tree.** ~80×20 fits identity, project
  breadcrumb, parent link, sub-issue list with state, and ~6 lines of
  description. Comments and labels intentionally out of v1.
- **Hierarchy via `j/k` + `u`/Enter, not arrow keys.** `j/k` walks the
  detail-view rows (parent → sub-issue 1..N → URL). `Enter` on a sub-issue
  pushes another `Detail` for that key (history kept in a small stack so
  `Esc` walks back up). `u` jumps to parent. `p` jumps to project (opens
  in browser — orch doesn't render projects).
- **GraphQL: one round-trip, fetch parent + children + project + cycle +
  description in the same query that already fetches title/state.**
  Children's children NOT fetched — drilling into a sub-issue triggers a
  fresh fetch for that key (cheap, lazy, avoids quadratic blowup).
- **Cache: `CachedLinear` becomes superset.** New fields are `Option<…>` so
  v0 cache files round-trip. No version bump needed.
- **Writes stay cut.** No transition, no comment composer, no labels.
  `o` opens browser. The bar is "is it cheaper than `cmd-T` + URL?" —
  almost nothing clears it.

## Why a stack-state and not a new tab / overlay / split

| Option | Why rejected |
|---|---|
| 5th detail tab "Detail" | Tabs are global to the right zone but Detail is per-issue; the tab strip would be lying about what's persistent |
| Modal overlay (centered) | Breaks the "log is always visible" property — user loses the Right zone context they came from |
| Split Linear tab into top tree + bottom detail block | Forces a third focus level: top tree, bottom detail, and parent zone focus all need cursor rules |
| **Stack-state inside Linear tab** | Reuses Right zone, reuses `Esc`, reuses `j/k`. One drill-in path, one drill-out path. |

The drill stack is bounded — Linear hierarchy in practice is 2-3 levels
deep, and the view re-fetches per push so memory cost is one extra cached
issue per drill. `Esc` from depth N pops to depth N-1; from depth 0 it
returns focus to the list per the canonical rule.

## Detail-view mockup (80 cols × 20 rows)

```
 Overview  ·  PRs  ·  ▎Linear  ·  Panes
────────────────────────────────────────────────────────────────────────────────
 ENG-29535  ·  P2  ◑ In Review  ·  4d ago
 Liquidity blotter: fix TCH prefund double-count in position report

 ▌ Project   batch-imports  ›  q2-hardening
 ▌ Parent    ENG-29151  Investigate ALERT
 ▌ Cycle     2026-W18 (ends Sun)
 ▌ Assignee  @ashley

 Description ─────────────────────────────────────────────────────────
   In the position report we double-count TCH prefund when the wire
   leg has already settled but the chunk processor hasn't flushed
   the seasoned-PR rollup. Repro: stage a TCH wire on Fri, observe
   blotter on Mon morning before 9am ET.

 Sub-issues (3) ──────────────────────────────────────────────────────
 ▸ ENG-30210  ◐  Tighten name normalize           @ashley
   ENG-30444  ●  Investigate ALERT                @robin
   ENG-30445  ○  Add unit tests                   —

 j/k navigate · Enter drill in · u parent · p project · o browser · Esc back
```

Layout fixed to 20 rows by trimming description to fit:

| Row | Content |
|---|---|
| 1-2 | tab strip (existing) |
| 3 | identity: key · priority · state-glyph + state · updated-age |
| 4 | title (wrap to 2 lines if needed; truncate at 2) |
| 5 | (blank) |
| 6 | project breadcrumb |
| 7 | parent link |
| 8 | cycle |
| 9 | assignee |
| 10 | (blank) |
| 11 | "Description ──" header |
| 12-15 | description body, wrapped at panel width, truncated with `…` |
| 16 | "Sub-issues (N) ──" header |
| 17-19 | up to 3 sub-issue rows; if more, last row shows `… and N more` |
| 20 | keymap footer |

When the description is long it eats into sub-issues. When sub-issues
exceed 3, description shrinks to 3 lines. Hard cap on rows — no
scrolling inside the detail view in v1. If the user wants the full
description, that's `o` (browser).

Color treatment:
- key in `IRIS`, priority badge in `LOVE` for P0/P1, `GOLD` for P2,
  `MUTED` for P3/P4 (or hidden)
- state glyph color via `linear_state_color()` (existing)
- breadcrumb separators `›` in `MUTED`, names in `TEXT`
- sub-issue cursor `▸` in `LOVE` when row focused, hidden otherwise;
  HL_LOW background on cursored row

## List-view mockup (80 cols, 2 rows per ticket)

Single-link case (most common):

```
 ▎Linear
────────────────────────────────────────────────────────────────────────────────
 ▸ ENG-29535  P2  ◑ In Review · 4d
   Liquidity blotter: fix TCH prefund double-count in position report
   batch-imports  ·  3 sub-issues  ·  @ashley

   ENG-30444  P3  ● Done · 2h
   Investigate ALERT
   batch-imports  ·  no sub-issues  ·  @robin

 j/k navigate · Enter detail · o browser
```

Two rows per ticket, blank line between for breathing room. Row 1 is
identity-dense (key, priority, state, age). Row 2 is title. Row 3 is
context (project, sub-issue count, assignee), `SUBTLE`.

Why two rows + context, not one or three:
- Today is **two rows already** (`bullet+key+title` then `state+assignee`).
  The cost is identical.
- Adding a third "context" row would push to 3 rows × N tickets, but the
  common case is N=1 or N=2 linked issues per task, so the panel still
  fits comfortably.
- Three rows lets us drop the redundant " · " separators that today's
  one-row state line uses.

Empty state and disconnected state unchanged from current `render_tab_linear`.

## Drill-down state machine

```
                       ┌──────────────────────────┐
                       │   Right focus, Linear    │
                       │   linear_view = List     │
                       └────────┬─────────────────┘
                                │ Enter on cursored issue
                                ▼
                       ┌──────────────────────────┐
                       │   linear_view =          │
            ┌──Esc────▶│   Detail{ stack: [K0] }  │
            │          └────────┬─────────────────┘
            │                   │ Enter on cursored sub-issue Ki
            │                   ▼
            │          ┌──────────────────────────┐
            │          │   linear_view =          │
            │  Esc     │   Detail{ stack:         │
            └──────────│     [K0, K1, ... Kn] }   │
                       └────────┬─────────────────┘
                                │ u  (parent of stack.top())
                                ▼  resolves Ki.parent_key, pushes
                       (back to Detail with deeper or sibling stack)

  Esc from Detail with stack.len()==1   →  linear_view = List
  Esc from List                          →  focus = List (zone A)
  Esc from List zone                     →  quit (canonical)
```

`u` pushes parent onto the stack. `Esc` pops. So a flow like
"drill in → drill into sub → up to parent → up to grandparent" never
loses history — the stack reconstructs the path. Stack capped at 8 (deep
enough for any real Linear hierarchy; prevents runaway).

## Keymap delta vs. current `tui3.rs::handle_right_key`

Only the Linear arm changes. PRs / Panes / Overview unchanged.

```
Tab::Linear, key:
  KeyCode::Char('j') | Down  →  cursor++ (within current view)
  KeyCode::Char('k') | Up    →  cursor--
  KeyCode::Enter             →  if List:        push Detail(cursor.key)
                                if Detail:      cursor row determines target:
                                                  · sub-issue row → push Detail(sub.key)
                                                  · parent row    → push Detail(parent.key)
                                                  · URL row       → open in browser
                                                  · project row   → open project URL
                                                  · other         → no-op
  KeyCode::Esc               →  if Detail:      pop stack; if empty → List
                                if List:        focus = Pane::List      (canonical)
  KeyCode::Char('u')         →  Detail only: push Detail(stack.top().parent_key) if Some
  KeyCode::Char('p')         →  open project URL in browser (List or Detail)
  KeyCode::Char('o')         →  open cursored issue URL in browser
                                (List uses cursor key; Detail uses stack.top())
  KeyCode::Char('r')         →  refresh — fetch stack.top() + children fresh
                                                          (List: refresh all linked)
```

`Esc` semantics matter — they must layer cleanly with the canonical
focus-zone `Esc`. Order in `handle_key`:

1. modal (help, message) — already handled
2. focus == Right && tab == Linear && linear_view == Detail → pop stack
3. focus == Right (anywhere else) → focus = List
4. focus == List → quit

The new step 2 inserts before the existing step 3. No other tab needs
this treatment because Detail is a Linear-only sub-state.

`h`/`l` for tab cycling stays — they work at any depth and just cycle
the outer tab strip, exiting the detail view by crossing tab boundary.
Cleaner than trapping them.

## GraphQL query

Single query, batched per task per refresh tick. Replaces the current
`fetch_many` selection set:

```graphql
query($id: String!) {
  issue(id: $id) {
    identifier
    title
    description           # markdown source, render as wrapped text
    priority
    priorityLabel
    state { name type }
    assignee  { displayName }
    labels    { nodes { name color } }      # cached, not rendered v1
    parent    { identifier title }
    children  {
      nodes { identifier title state { name type } assignee { displayName } }
    }
    project   { id name slugId }
    cycle     { name endsAt }
    branchName
    url
    updatedAt
  }
}
```

`fetch_many` uses the same selection via aliased fragments
(`i0: issue(id: ...) { ...IssueFields }`).

Round-trip cost per refresh tick:
- 1 query for all linked issues (existing)
- Each linked issue's children come **embedded** — no second round-trip
  for the default detail view
- Drilling into a sub-issue triggers exactly one additional `fetch_issue`
  for that key, debounced at 30s

Children-of-children deliberately NOT fetched. The detail view shows
direct sub-issues only with their state badge; descending one more level
re-fetches that node. Avoids exponential payload growth on drill-in.

## CachedLinear shape

Fields added; everything optional with `#[serde(default)]` so v0 cache
files load cleanly. No version bump.

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedLinear {
    // Existing.
    pub identifier:    String,
    pub title:         String,
    pub state:         String,
    pub state_kind:    String,
    pub assignee:      String,
    pub fetched_at:    u64,

    // New: identity + display.
    #[serde(default)] pub description:    String,            // markdown source
    #[serde(default)] pub priority:       u8,                // 0=none, 1=urgent, 2=high, 3=med, 4=low
    #[serde(default)] pub priority_label: String,
    #[serde(default)] pub url:            String,
    #[serde(default)] pub branch_name:    String,
    #[serde(default)] pub updated_at:     String,            // ISO; rendered as relative

    // New: hierarchy.
    #[serde(default)] pub parent_key:     Option<String>,    // identifier, e.g. "ENG-29151"
    #[serde(default)] pub parent_title:   Option<String>,    // denormalized to skip a lookup
    #[serde(default)] pub children:       Vec<CachedChild>,

    // New: project / cycle.
    #[serde(default)] pub project:        Option<CachedProject>,
    #[serde(default)] pub cycle_name:     Option<String>,
    #[serde(default)] pub cycle_ends_at:  Option<String>,

    // New: labels (cached, not rendered v1 — defer label chip strip).
    #[serde(default)] pub labels:         Vec<CachedLabel>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedChild {
    pub identifier: String,
    pub title:      String,
    pub state:      String,
    pub state_kind: String,
    pub assignee:   String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedProject {
    pub id:      String,
    pub name:    String,
    pub slug_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedLabel {
    pub name:  String,
    pub color: String,    // hex like "#ea9d34"
}
```

Backward compat: existing `linear.json` files load with new fields zeroed.
On first refresh after upgrade, the daemon writes the enriched shape.
`disconnected` flag on `LinearCache` unchanged.

Project-level metadata (initiative, lead, milestones) NOT cached. The
detail view shows project name + slug only; pressing `p` opens the
project URL in browser. Caching projects requires its own map and
refresh cadence — out of scope for v1.

## Worked flows

### Flow 1 — "See description and sub-issues for ENG-29535"

```
state                                 keys     result
─────────────────────────────────────  ──────  ─────────────────────────────
list focused, task on ENG-29535         (init)
jump to Linear tab                      3       right zone, Linear list view,
                                                cursor on ENG-29535 (or 1st)
drill into the issue                    Enter   linear_view = Detail{[ENG-29535]}
                                                fetches description + children
                                                if cache > 30s
read description, scroll sub-issues     j j     cursor moves down sub-issue list
back to list view                       Esc     linear_view = List
back to task list                       Esc     focus = Pane::List
```

5 keys to read deep, 1 key per drill-out. Total cost is 3 + Enter + Esc + Esc
= 5 keystrokes round trip.

### Flow 2 — "From a sub-issue, jump to the parent project's other issues"

orch doesn't render project trees in-TUI (cut). Project drill-out goes to
browser:

```
state                                 keys     result
─────────────────────────────────────  ──────  ─────────────────────────────
in Detail view of ENG-30210             (init)  cursor on description
                                                (or wherever)
open project in browser                 p       opens https://linear.app/
                                                <workspace>/project/<slug_id>
                                                in browser; orch stays put
back to list                            Esc Esc Detail → List → list zone
```

This is the explicit cut-line. Linear's web project view is dense (board,
roadmap, milestones, status updates) — replicating any of it in 80×20 is
a losing trade. The browser is one keystroke away. The TUI's job is the
read-quick + drill-into-sub-issues flow.

If "go up one level" is what's wanted (parent issue, not project):

```
state                                 keys     result
─────────────────────────────────────  ──────  ─────────────────────────────
in Detail view of sub-issue ENG-30210   (init)
jump to parent issue                    u       push Detail(ENG-29151)
                                                — works iff parent_key is Some
                                                in cache
```

`u` is silent no-op when `parent_key` is `None` (top-level issue). No toast
for that — it's expected.

### Flow 3 — "Mark this issue as in-progress"

**Decision: defer to browser.** Press `o` to open the issue, click the
state pill there.

Rationale (restated, not re-litigated):
- State transitions need a state list per team workflow → either fetch &
  cache workflows (extra GraphQL surface) or hardcode 6 canonical states
  and accept Linear API rejections
- A transition flow would need multiple modal states: state list,
  optimistic ack, and revert; each one is a failure surface
- The user already has `cmd-T` Linear browser muscle memory; the cost of
  switching to browser for a state change is < the cost of building +
  maintaining the modal
- One-user tool: no risk of stomping someone else's transition mid-flight

If transitions become high-frequency later, revisit. Until then:

```
state                                 keys     result
─────────────────────────────────────  ──────  ─────────────────────────────
on the issue (List or Detail)           (init)
open in browser                         o       opens https://linear.app/…
                                                in default browser
(transition in browser)
back in orch, refresh                   r       force re-fetch this key,
                                                cache catches new state
```

`o` works from both List and Detail; in List it uses the cursored key, in
Detail it uses `stack.top()`.

## What stays cut

Listed with explicit rationale so this doesn't get re-asked.

| Cut | Why |
|---|---|
| State transition modal | See Flow 3. Browser is one key away. |
| Comment composer | Multi-line input + draft persistence + offline queue is a real surface. The handoff flow it served (write a comment to brief the next worker) is solved better by `tasks/foo.md` notes — orch already opens those. |
| Label editing | Low frequency, high UI cost (color picker, multi-select). Browser. |
| Assign / reassign | Single-user tool; assignee almost always = self. |
| Priority change | Set once at issue creation, rarely flipped mid-flight. Browser. |
| Sub-issue creation | Real value, but needs a buffer + parent-link logic + workflow inheritance. Defer; not blocked by this design. |
| Initiative / milestone view | Linear's roadmap surface is rich; replicating in TUI is a losing trade. `p` opens project; initiative is one click further in browser. |
| Cycle scope (issues in same cycle) | Useful 1×/sprint. Linear's cycle view is good enough. |
| Tree-of-issues drill (recursive children) | Caching + rendering arbitrary depth is a real engineering chunk for ~2 levels of practical depth. v1 shows direct children; drilling into one re-fetches its children. |
| `/` filter on linked issues | Nearly always the user has 1-2 linked issues per task. Filter has no work to do. |
| Multi-link strip with `1`-`9` numbered jump | The tasks-list-row shows `L<N>` count; the rare 2+ link case lists them top-to-bottom in the panel. Numbered strip was overkill. |
| Stale/age tinting (LOVE on > 5 min) | Low signal. Refresh runs every 2 min anyway. The relative-age string ("4d ago", "12s ago") in the header is enough. |
| Bidirectional PR ↔ Linear surfacing | Cache cost (Linear → PR URLs) is non-zero, render value low — the user is already on the PR tab when they want a PR. |
| Notifications / unread badge | Linear's notification API is a separate surface. Out of scope. |

## Implementation cost (rough)

For sizing — not a plan.

- `LinearCache` shape: ~30 LOC + serde
- `linear.rs::fetch_*`: query rewrite, ~40 LOC delta
- `tui3.rs`: new `linear_view: LinearView` field, render branches for
  List/Detail, ~150 LOC including layout math
- key handler: ~30 LOC delta in `handle_right_key` Linear arm
- snapshot tests: 4 new (`snapshot_linear_list_v2`,
  `snapshot_linear_detail`, `snapshot_linear_detail_drill`,
  `snapshot_linear_detail_no_parent`)

Net diff roughly **+250 LOC, -50 LOC** in `tui3.rs` + cache + linear,
with no breaking change to existing snapshots beyond `snapshot_detail_tab_linear`
and `snapshot_linear_anchor_subissues` which need re-baselining for the
new list shape.
