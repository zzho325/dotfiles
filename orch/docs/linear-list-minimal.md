# Linear List View — Minimal

Round 3. The user called rounds 1 and 2 "still messy" and asked for a
minimal renderer that fits **sub-issues inline on the same page**. This doc
specifies the list view only — the detail view from `linear-deep-design.md`
stays exactly as it is, and the two-zone focus model from
`tui-nav-redesign.md` is not up for revision.

The whole list is **one flat row stream**. Parents and sub-issues are peers
under `j/k`. Project headers appear only when there are 2+ projects. Every
row is exactly one line.

## Mockup A — single project, 12 issues, 2 with sub-issues

Real shape from the user's `batch-creation` task data. 30 rows, comfortable.

```
 Overview  ·  PRs  ·  ▎Linear  ·  Panes
────────────────────────────────────────────────────────────────────────────────
 ▸ ENG-27958  ◐  Batch transfer creation: BA-grouping shared DB transactions
   │  ENG-28114  ◐  Two-pass ledger for batch deadlock prevention
   │  ENG-28115  ○  Preflight checks outside batch txn
   └  ENG-28116  ●  Skeleton batch txn + sandbox load test
   ENG-26405  ●  ACH Execution                                          + 4 sub
   ENG-26407  ●  ACH CSV                                                + 3 sub
   ENG-27134  ●  Patch moov-ach fork: preserve entries on validation error
   ENG-26400  ●  Workflow skeleton — state machine, signal, expiry
   ENG-26418  ●  ACH CSV parser
   ENG-26402  ●  Activity: validate records MVP
   ENG-26401  ●  Activity: create transfers
   ENG-26403  ●  Activity: parse NACHA file
   ENG-26404  ●  Workflow
   ENG-27959  ○  Batch transfer creation: chunked deploy resilience
   ENG-27960  ·  Batch transfer creation: idempotency on retry

 j/k move · Enter open · t expand · o browser
```

Notes on what's visible vs. dropped (rationale in §4):

- `[Bulk Payments]` title prefix stripped — it's the project name repeated
  on every row. Project name itself is hidden (only one project).
- Cursor is `▸` in `LOVE`; non-cursored rows have leading 3-space indent.
- State glyph carries the state — no "In Progress" / "Done" word.
- One sub-issue group is expanded inline (the cursored parent or
  user-pinned via `t`). Other parents collapse to `+ N sub` hint in `MUTED`
  at the right of the row.
- No age, no assignee, no priority badge, no project tag, no separators.

## Mockup B — multi-project, 2 issues across 2 projects

Project headers re-appear only when there's more than one. Subtle, single
line, no count, no rule below.

```
 Overview  ·  PRs  ·  ▎Linear  ·  Panes
────────────────────────────────────────────────────────────────────────────────
 bulk payments
 ▸ ENG-27958  ◐  Batch transfer creation: BA-grouping shared DB transactions
   │  ENG-28114  ◐  Two-pass ledger for batch deadlock prevention
   │  ENG-28115  ○  Preflight checks outside batch txn
   └  ENG-28116  ●  Skeleton batch txn + sandbox load test

 ai tooling
   ENG-25762  ◐  Remote Agent Chat Server                              + 8 sub

 j/k move · Enter open · t expand · o browser
```

Header style: lowercased project name in `IRIS`, no count, no
`·`-separator, no underline, single blank line above (none below). The
issues underneath provide the visual block.

## Per-row layout spec

There are exactly three row kinds. Every row is one line. No row wraps.

### Parent row (cursor target)

```
 ▸ ENG-27958  ◐  Batch transfer creation: BA-grouping shared DB transactions
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
 │  │           │  │                                                  │
 │  │           │  title (TEXT, truncated to fit, no `…` mid-word)    + N sub
 │  │           state-glyph (state-color) — single char                   ↑
 │  key (IRIS, fixed-width to col 14)                                     │
 cursor: " ▸ " LOVE if focused & selected, else "   "          MUTED, only
                                                               on collapsed
                                                               parent w/ children
```

Field order, fixed columns where it matters:

| Col | Width | Field | Style |
|---|---|---|---|
| 0–2 | 3 | cursor `" ▸ "` or `"   "` | LOVE if cursored & focused |
| 3–11 | 9 | issue key, padded | IRIS |
| 12–13 | 2 | spaces | — |
| 14 | 1 | state glyph | state-color (FOAM/PINE/GOLD/IRIS/LOVE/MUTED) |
| 15–16 | 2 | spaces | — |
| 17–N | flex | title (prefix `[Project]` stripped if matches enclosing project) | TEXT |
| right | flex | `+ N sub` if `children.len() > 0` AND collapsed | MUTED |

Title truncation: hard-cut at `area.width - 17 - rightcol_width - 1`,
append `…`. No mid-word break beyond that — just slice.

### Sub-issue row (cursor target, inline under parent when expanded)

```
   │  ENG-28114  ◐  Two-pass ledger for batch deadlock prevention
   └  ENG-28116  ●  Skeleton batch txn + sandbox load test
```

Same row anatomy, but column 0–4 is the tree glyph instead of cursor:

| Col | Content |
|---|---|
| 0–2 | `"   "` (always — cursor on a sub-issue is a row-bg only, see §3) |
| 3 | `"│"` for non-last sibling, `"└"` for last sibling |
| 4 | `" "` |
| 5–7 | `"  "` then state glyph alignment continues as parent |

…actually simpler: align the sub-issue's `key` to the **same column as
parent keys** (col 3), so the eye scans one column of `ENG-…` keys
top-to-bottom. The tree glyph lives in the cursor slot:

```
col: 0123456789012345678
     " ▸ ENG-27958  ◐  …"   parent, cursored
     "   ENG-26405  ●  …"   parent, not cursored
     " │ ENG-28114  ◐  …"   sub-issue, mid-list
     " └ ENG-28116  ●  …"   sub-issue, last child
     " ▸ ENG-28114  ◐  …"   sub-issue, cursored — `▸` replaces tree glyph
                            BUT bg = HL_LOW so the parent linkage is still
                            implied by indent + position
```

When the cursor lands on a sub-issue we **drop the tree glyph for the
cursored row only** and use the standard `▸`. Tree continuity is preserved
by the rows above/below; the cursor is ephemeral, the tree isn't.

Sub-issue title: `[Bulk Payments]`-style prefix is stripped (same rule as
parent rows). `+ N sub` is never shown on a sub-issue row — grandchildren
aren't cached, so we can't honestly count them.

### Project header row (only when 2+ projects)

```
 ai tooling
```

| Col | Content |
|---|---|
| 0 | space |
| 1–N | lowercased project name | IRIS |

Single blank line above (skipped on the very first one). No blank below.
No count, no separator, no rule. If a row has no project, it sorts last
under a `(no project)` header in MUTED.

## Cursor + drill behavior

`j/k` walks the **flat row stream**: parent rows and visible sub-issue
rows are equal peers. Project headers are skipped.

```
state                                key       result
─────────────────────────────────── ───────  ───────────────────────────────
on parent ENG-27958 (expanded)       j        cursor → first sub-issue row
                                              ENG-28114
on ENG-28114                         j        cursor → ENG-28115 (next sub)
on ENG-28116 (last sub)              j        cursor → next parent (ENG-26405)
on ENG-26405 (collapsed parent       j        cursor → next parent
  with children)                              (sub-issues stay collapsed)
on any row                           Enter    push Detail(row.key) — same
                                              path for parent and sub-issue
on a parent row                      t        toggle expand/collapse THIS
                                              parent's children inline
on a sub-issue row                   t        no-op
on any row                           o        open row.key in browser
```

### Auto-expand rule

Picking which parent is "expanded" without a manual toggle:

- **The parent under the cursor auto-expands** while the cursor is on it,
  if it has 1–6 children.
- Other parents stay collapsed (showing `+ N sub`).
- `t` pins/unpins a parent's expansion so it stays open even when the
  cursor leaves it. Pinned state lives in `LinearView::List { expanded:
  HashSet<String> }`.
- A parent with **7+ children** does NOT auto-expand. Press `t` to opt
  in; render shows first 6 + `└ + N more` (final row, MUTED, not a cursor
  target — Enter on it does nothing; user drills into the parent for the
  full list).

This means in the common case the user just walks `j` down the list and
each parent reveals its children as they pass through. No fold/unfold
ceremony unless they have a 10-sub-issue mega-issue.

### No number-key shortcuts

Considered `1`–`9` to jump to the Nth row. Rejected: `1`-`4` are already
detail-tab jumps in the global keymap (`tui-nav-redesign.md` §3). The
visible row count is small enough (~12-20) that `j j j j` is not the
bottleneck.

## What's dropped from the current renderer

| Cut | Why |
|---|---|
| Per-issue identity row separate from title row | Two rows per issue across 12 issues = 24+ rows of chrome before sub-issues. One row carries key + state-glyph + title fine at 80 cols. |
| State name as text ("In Progress", "Done") | The glyph (`◐ ● ○ · ⊘`) is unambiguous at a glance and saves 8–12 cols. State color reinforces. |
| Updated-age (`4d ago`, `12h ago`) | Refresh runs every 2 min; "when was this touched" is detail-view info. List is for "what is here". |
| Assignee (`@ashley`) | Single-user tool; assignee = self ~95% of the time. Other-assigned issues read fine without — the rare reassignment is detail-view info. |
| Priority badge (`P0`–`P3`) on every row | Priority steers attention only when high; detail view shows it. List-view priority badges added clutter without reordering rows. |
| Project name on every issue row | Either there's one project (drop) or there's a header above (drop, redundant). |
| Sub-issue count text (`3 sub-issues`) | Now shown either as expanded inline rows or as `+ N sub` in MUTED on the right edge of a collapsed parent. |
| `· · ·` separator chains in the trailer | No trailer means no separators. |
| Project header trailing `· N` count | Count is implied by the rows below; the header is just a label. |
| Blank line between every issue | Issues are now 1 row, not 2. Blank lines between every row would double the height for nothing. Single blank only between projects. |
| `[Project Name]` title prefix | Linear's auto-namespacing duplicates the project header. Strip when prefix matches enclosing project (case-insensitive `[…]` match). |

## What stays cut even though tempting

| Tempting cut not made | Why we resisted |
|---|---|
| Add cycle name on each row | Detail view's job. Cycle is per-issue, varies, and would force a 2nd line back. |
| Color-code rows by state-kind background | Tried in v2 internally — colored backgrounds across 15 rows looked like a bug report. State-glyph color is enough. |
| Show labels (chip strip) | Variable-width, multi-color chips at row-end fight with `+ N sub`. Defer to detail view. Cached but not rendered, per `linear-deep-design.md`. |
| Show a PR-link badge next to issues with attached PRs | Cross-cache lookup; PR tab already does this. Keep tabs orthogonal. |
| `/` filter | 12 rows fit on screen. Filter has no work to do. |
| Group by state ("Done" collapsed by default) | Sub-issue grouping already adds a hierarchy layer; a second one (state) would conflict. State glyph already lets the eye skip done items. |
| Numbered jump strip (`1`–`9`) | Conflicts with global tab-jump keys. `j/k` is fast enough at this row count. |
| Show parent breadcrumb when a sub-issue is rendered without its parent | Doesn't happen — sub-issues only render under their parent, never standalone. |
| "Loading…" placeholder text per row | The cache is read synchronously and renders last-known data; first-fetch is rare and brief. A single global "stale" badge in the tab strip header is sufficient (already exists). |
| A second cursor for "selected sub-issue under selected parent" | Two cursors = two focus levels = the bug from `tui-nav-redesign.md` §1. One flat cursor over all rows. |

## Edge cases

| Case | Behavior |
|---|---|
| Parent with 0 children | Renders as a normal one-line row. No `+ N sub` trailer. `t` is a no-op. |
| Parent with 1–6 children, cursored | Auto-expanded inline. Tree glyphs `│` / `└`. |
| Parent with 1–6 children, not cursored | Collapsed. Right edge shows `+ N sub` in MUTED. |
| Parent with 7+ children | Never auto-expands. `t` pins; on expand, first 6 rows + 1 row `└ + N more` (MUTED, not a cursor target). User must Enter the parent to see the full list in the detail view. |
| Cached children with their own children | Not visible. Grandchildren aren't cached (`linear-deep-design.md` §GraphQL). The grandchild count isn't shown either — we don't know it. |
| Issue with no project | Sorts to the end. If 2+ projects exist, sits under header `(no project)` in MUTED. If only one project total and one orphan, no header — visually consistent with single-project case. |
| Title longer than row width | Hard truncate with `…`. No wrap. The detail view (Enter) shows the full title. |
| Cache empty / first run | Each row renders with key + `·` glyph in MUTED + key as title placeholder. No "loading…" string per row. The tab-strip shows the global stale/disconnected indicator. |
| Issue not on Linear (in `not_found`) | Single dim row: `   ENG-99999  ⊘  (not on Linear)` in MUTED. Cursor lands on it; Enter is no-op. |
| `disconnected = true` (refresh failed) | Render last-known data unchanged. Tab strip shows `Linear (stale)` per existing convention. |
| Very long project name | Truncate header at row width with `…`. |
| Single linked issue, no children | One row. Footer keymap line still renders. |
| 30+ linked issues | List flows past the panel and uses the existing scrolling for the right pane (existing behavior; cursor stays in viewport). |

## State stored in `App`

```
LinearView::List {
    cursor: usize,                      // index into flat row stream,
                                        // not into task.linear[]
    expanded: HashSet<String>,          // pinned-open parent keys
}
```

The cursor walks a derived `Vec<Row>` built each frame from
`task.linear` + `cache.issues` + `expanded`. `Row` is parent-vs-sub at
render time; cursor stores an index, so resize doesn't invalidate it
(clamped to `rows.len()-1`).

`Enter` pulls `rows[cursor].key` regardless of row kind. `t` flips
`expanded.contains(rows[cursor].key)` only when the row is a parent with
children.

## Footer keymap

Single line, MUTED, only when focused:

```
 j/k move · Enter open · t expand · o browser
```

Drops `navigate · detail` from current footer — shorter verbs, room for
`t expand`. `Esc` is global and not advertised here (per nav redesign).
