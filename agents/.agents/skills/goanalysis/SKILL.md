---
name: goanalysis
description: Go static analysis tools — function ordering lint (orderlint) and diff-annotated call graph for PR review (goreview). Use for checking ordering violations, visualizing call trees, or generating review graphs.
allowed-tools:
  - Bash
  - Read
  - Grep
  - Glob
---

<objective>

Run Go static analysis tools: **orderlint** checks function ordering, **goreview** renders diff-annotated call graphs for code review.

**Usage:**
- `/goanalysis lint ./pkg/some/package/...` — orderlint on specific packages
- `/goanalysis lint graph ./pkg/some/package/...` — orderlint ASCII call tree
- `/goanalysis review ./pkg/some/package/...` — goreview call graph
- `/goanalysis review --diff origin/main ./pkg/some/package/...` — goreview with diff markers
- `/goanalysis review --diff origin/main --changes-only ./pkg/some/package/...` — only changed subtrees
- `/goanalysis guide` — review guide for current branch's PR (auto-detects packages)
- `/goanalysis guide <PR number>` — review guide for a specific PR

A package path is always required.

</objective>

<instructions>

## Determine mode

Parse the user's arguments:
- No args → show usage help (package path is required)
- `lint <packages>` → **orderlint** (see orderlint modes below)
- `review <packages>` → **goreview**
- A bare package path without subcommand → **orderlint** on that package

## Orderlint

Checks that functions appear in call-order: if A calls B in the same file, A should be defined before B.

### Orderlint diff mode

Lint only violations introduced on the current branch:

```bash
orderlint-diff origin/main ./pkg/specific/...
```

If violations found, show them and suggest fixes. For each violation like:
```
file.go:10:1: helper (line 10) should appear after Handler (line 25) — Handler calls helper
```
Explain: "Move `helper` below `Handler` — `Handler` calls it, so it should come after."

### Orderlint graph mode

```bash
orderlint -graph ./pkg/specific/...
```

Show the output. Explain the tree:
- Root-level = entry points (uncalled functions)
- Indented = callees in DFS order
- `✗` = ordering violation (callee appears before its caller)
- `↩` = already shown earlier (avoids duplication)

### Orderlint full mode

```bash
orderlint ./pkg/specific/...
```

Show all violations. Warn this includes pre-existing violations, not just changes.

### What orderlint checks

1. **Call ordering** — if A calls B in the same file, A must appear before B
2. **Cycle detection** — mutual recursion flagged as warning, not error
3. **Test helpers** — unexported helpers in `_test.go` should appear after all `Test*`/`Benchmark*`
4. **Multiple callers** — if both A and C call B, B must appear after whichever caller comes first

## Goreview

Renders a diff-annotated call graph for a Go package — useful for understanding what new code does during PR review.

### Basic usage

```bash
goreview ./pkg/some/package/...
```

Shows the full call graph: exported functions as roots, unexported callees indented below, methods grouped under receiver type.

### With diff annotations

```bash
goreview --diff origin/main ./pkg/some/package/...
```

Adds markers: `+` = new function, `~` = modified, ` ` = unchanged context.

### Changes only (for PR review)

```bash
goreview --diff origin/main --changes-only ./pkg/some/package/...
```

Filters to only subtrees containing new/modified functions. Use this for PRs in large packages where unchanged code would bury the changes.

### Depth limiting

```bash
goreview --depth 2 ./pkg/some/package/...
```

Limits call tree to N levels deep — useful for large packages.

### Reading the output

- Top-level items = exported functions not called by others (API surface)
- Type headers group methods by receiver (`TokenSource`, `APIClient`, etc.)
- `[unexported]` = internal helper, not part of public API
- `↩` = already shown earlier (cycle or shared callee)
- Signatures show param names and short type names (`*rsa.PrivateKey` not `*crypto/rsa.PrivateKey`)

## Guide mode

When the user runs `/goanalysis guide` or `/goanalysis guide <PR number>`, generate a review guide
that helps reviewers understand the PR quickly.

### Step 1: Gather context

If a PR number is given, use `gh pr diff <number>` and `gh pr view <number> --json files`.
Otherwise, detect the current branch's PR via `gh pr view`, or fall back to `git diff main...HEAD`.

Extract the list of changed Go packages from the changed files.

### Step 2: Run tools

For each changed Go package:
```bash
goreview --diff origin/main --changes-only ./pkg/changed/package/...
```

For the full set of changed files:
```bash
orderlint-diff origin/main ./pkg/changed/package/...
```

If the PR branch is not checked out locally, fetch it (`gh pr checkout <number>` or
`git fetch origin <branch>`) before running the tools.

### Step 3: Read and understand

Read the changed files and surrounding code to understand:
- What the PR does (one sentence)
- How the pieces fit together (reading order for a reviewer)
- Key design decisions and their rationale
- Caller/callee relationships beyond what goreview shows

### Step 4: Output the review guide

Output in this exact format:

```
## Review guide

**What this does:** <One sentence summary — plain, no drama.>

**How to read this PR:**

1. **Description** (`path/to/file.go: FunctionOrType`) — what it does, why it matters,
   key details a reviewer should notice.

2. **Description** (`another/file.go: AnotherFunction`) — same format. Order by
   recommended reading order, not alphabetical.

3. ...

**Call graph:**
<goreview output in a code block, one section per package>

**Key design decisions:**
- **Decision** — rationale. Why this approach over alternatives.
- ...
```

### Guidelines

- **"How to read"** entries should follow the data/control flow, not file order.
  Start with the entry point, then follow the call chain.
- Keep each entry to 1-2 sentences. Flag non-obvious things (error types,
  short-circuit behavior, stubs for future PRs).
- **Call graph** is the raw goreview output — don't manually redraw it.
- **Key design decisions** should explain choices a reviewer might question
  (why flat params vs struct, why unexported, why this error type, etc.).
- If orderlint found violations, add an **Orderlint** section after the call graph.
- A reviewer should understand the PR's shape in 30 seconds from this guide.

## Tools

| Command | What it does |
|---------|-------------|
| `orderlint ./pkg/...` | Lint packages, report all ordering violations |
| `orderlint -graph ./pkg/...` | Print ASCII call tree per file (intra-file) |
| `orderlint-diff <rev> ./pkg/...` | Lint only files changed since `<rev>` |
| `goreview ./pkg/...` | Diff-annotated call graph (cross-file, per package) |
| `goreview --diff <rev> ./pkg/...` | Call graph with new/modified markers |
| `goreview --diff <rev> --changes-only ./pkg/...` | Only subtrees with changes |
| `goreview --depth N ./pkg/...` | Depth-limited call graph |

Both `--diff <rev>` and `orderlint-diff <rev>` accept git refs or jj bookmarks — jj is auto-detected.

### Orderlint baseline

`orderlint` auto-reads `.orderlintbaseline` at the git/jj repo root. Each line is `filename:FuncName` to suppress known-accepted violations (e.g., CRUD ordering). Supports `#` comments. Override path with `-baseline=<path>`.

Source: `~/dotfiles/tools/orderlint/`, `~/dotfiles/tools/goreview/`
Binaries: `~/bin/orderlint`, `~/bin/goreview`

</instructions>
