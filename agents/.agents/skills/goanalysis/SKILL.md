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
- `/goanalysis summarize ./pkg/some/package/...` — goreview + human-readable summary for PR comments

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

## Summarize mode

When the user runs `/goanalysis summarize <packages>`, generate a human-readable PR review comment:

1. Run `goreview --diff origin/main --changes-only <packages>` for each package
2. Interpret the call graph output and write a summary with:
   - **Call graph**: the raw tree output in a code block, one section per package
   - **What changed**: 2-3 sentences describing the new/modified functions and their purpose
   - **Design notes**: architectural observations — layer separation, validation boundaries, key patterns (idempotency, error wrapping, etc.)
3. Keep it concise — a reviewer should be able to understand the PR's shape in 30 seconds

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

Source: `~/dotfiles/tools/orderlint/`, `~/dotfiles/tools/goreview/`
Binaries: `~/bin/orderlint`, `~/bin/goreview`

</instructions>
