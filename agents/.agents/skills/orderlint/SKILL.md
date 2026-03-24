---
name: orderlint
description: Check Go function ordering in the current branch. Reports when a callee appears before its caller in the same file. Use when reviewing function order, checking ordering violations, or visualizing call graphs.
allowed-tools:
  - Bash
  - Read
  - Grep
  - Glob
---

<objective>

Run the orderlint Go analyzer to check function ordering or visualize call graphs. If A calls B in the same file, A should appear before B.

**Usage:**
- `/orderlint` — lint changed packages on the current branch
- `/orderlint ./pkg/some/package/...` — lint specific packages
- `/orderlint graph ./pkg/some/package/...` — show ASCII call tree
- `/orderlint all ./...` — lint everything (noisy on large repos)

</objective>

<instructions>

## Determine mode

Parse the user's arguments:
- No args or no subcommand → **diff mode** (only changed packages vs origin/main)
- `graph` subcommand → **graph mode**
- `all` subcommand → **full mode** (all specified packages, including pre-existing violations)
- A package path without subcommand → **diff mode** on that package

## Diff mode (default)

Run orderlint only on packages containing files changed on the current branch:

```bash
orderlint-diff origin/main ./...
```

Or if the user specified packages:

```bash
orderlint-diff origin/main ./pkg/specific/...
```

If no violations are found on changed files, report success.

If violations are found, show them and suggest fixes. For each violation like:
```
file.go:10:1: helper (line 10) should appear after Handler (line 25) — Handler calls helper
```
Explain: "Move `helper` below `Handler` — `Handler` calls it, so it should come after."

## Graph mode

```bash
orderlint -graph ./pkg/specific/...
```

Show the output to the user. Explain the tree structure:
- Root-level = entry points (uncalled functions)
- Indented = callees in DFS order
- `✗` = ordering violation (callee appears before its caller)
- `↩` = already shown earlier in the tree (avoids duplication)

## Full mode

```bash
orderlint ./pkg/specific/...
```

Show all violations. Warn the user that this includes pre-existing violations, not just their changes.

## Tools

| Command | What it does |
|---------|-------------|
| `orderlint ./pkg/...` | Lint packages, report all ordering violations |
| `orderlint -graph ./pkg/...` | Print ASCII call tree per file |
| `orderlint-diff <rev> ./pkg/...` | Lint only files changed since `<rev>` |

Source code: `~/tools/orderlint/`
Binary: `~/bin/orderlint`

## What it checks

1. **Call ordering** — if A calls B in the same file, A must appear before B
2. **Cycle detection** — mutual recursion flagged as warning, not error
3. **Test helpers** — unexported helpers in `_test.go` files should appear after all `Test*`/`Benchmark*` functions
4. **Multiple callers** — if both A and C call B, B must appear after whichever caller comes first by position

</instructions>
