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

Use a descriptive filename to avoid collisions between concurrent runs.

**Review:**
```bash
OUT=/tmp/codex-review-<PR-number>.md
LOG=/tmp/codex-review-<PR-number>.log
codex exec review "<PR-link>" -o "$OUT" < /dev/null 2>"$LOG"
```

**Default:**
```bash
OUT=/tmp/codex-<short-topic>.md
LOG=/tmp/codex-<short-topic>.log
codex exec "<prompt>" -o "$OUT" < /dev/null 2>"$LOG"
```

`-o` captures the final response. `$LOG` captures the work log (tool calls, reasoning).
`< /dev/null` prevents stdin hang.

## 3. Read and present

```bash
cat "$OUT"
```

If the output is empty or codex failed, check the work log:
```bash
cat "$LOG"
```

Clean up after presenting:
```bash
rm -f "$OUT" "$LOG"
```

Present findings as proposals. For each finding, add your response
(agree, disagree, already handled) underneath. Write to notes.md:
```
notes propose "Codex review" -b "1. <finding>\n   → <your response>\n\n2. ..."
```
Do NOT post as PR comment — just present in conversation.
Do NOT implement changes — wait for user approval.

</process>

<rules>

- **Never auto-fix.** Present findings as proposals first.
- **Never post to PR comments** unless the user explicitly asks.
- **Be honest about disagreements** with codex findings.

</rules>
