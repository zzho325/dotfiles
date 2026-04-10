---
name: codex
description: Run codex for questions, second opinions, or PR review. Default uses `codex exec`. Use `/codex review <PR-link>` for structured PR review.
allowed-tools:
  - Bash
  - Read
---

<objective>

Run codex and present the output. Never auto-fix anything.

</objective>

<process>

## 1. Parse arguments

- `/codex review <PR-link>` → use `codex exec review`
- `/codex <anything else>` → use `codex exec`

## 2. Run codex

**Review:**
```bash
codex exec review "<PR-link>" -o /tmp/codex-output.md 2>/dev/null
```

**Default:**
```bash
codex exec "<prompt>" -o /tmp/codex-output.md 2>/dev/null
```

`-o` captures the final response. Stderr is discarded.

## 3. Read and present

```bash
cat /tmp/codex-output.md
```

For reviews: post as PR comment via `gh pr comment`, then present
each finding as a proposal (with proposed fix or disagreement).

For questions: present the answer. If it suggests code changes,
present as proposals — don't auto-apply.

## 4. Wait

Do NOT implement changes. Wait for user approval.

</process>

<rules>

- **Never auto-fix.** Present findings as proposals first.
- **Reviews: post to PR comment** so the user sees it on GitHub.
- **Be honest about disagreements** with codex findings.

</rules>
