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
codex exec review "<PR-link>" -o /tmp/codex-output.md < /dev/null 2>/dev/null
```

**Default:**
```bash
codex exec "<prompt>" -o /tmp/codex-output.md < /dev/null 2>/dev/null
```

`-o` captures the final response. `< /dev/null` prevents stdin hang. Stderr is discarded.

## 3. Read and present

```bash
cat /tmp/codex-output.md
```

Present findings as proposals (with proposed fix or disagreement).
Do NOT post as PR comment — just present in conversation.
Do NOT implement changes — wait for user approval.

</process>

<rules>

- **Never auto-fix.** Present findings as proposals first.
- **Never post to PR comments** unless the user explicitly asks.
- **Be honest about disagreements** with codex findings.

</rules>
