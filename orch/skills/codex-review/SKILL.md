---
name: codex-review
description: Get a codex code review on a PR. Presents feedback as proposals — never auto-fixes.
allowed-tools:
  - Bash
  - Read
---

<objective>

Run codex to review a PR and present the feedback as proposals for the user
to approve or reject. Never auto-fix anything.

</objective>

<process>

## 1. Run codex

Pass the PR link (from `$ARGUMENTS`) as a prompt to codex:

```bash
codex exec review "$ARGUMENTS" -o /tmp/codex-review-output.md 2>/dev/null
```

If no argument given, ask the user for a PR link.

`-o` writes the final review to a file. Stderr (verbose log) is discarded.

## 2. Read the review

```bash
cat /tmp/codex-review-output.md
```

## 3. Post as PR comment

Extract the PR number from the link and post:

```bash
gh pr comment <number> --body "$(cat /tmp/codex-review-output.md)"
```

## 4. Present proposals

For each finding in the review, present it as a proposal — either with a
proposed fix or an explanation of why you disagree. Group related findings.

If in interactive mode with `notes.md`, use `notes propose` for actionable
items.

## 5. Wait

Do NOT implement changes. Wait for user approval.

</process>

<rules>

- **Never auto-fix.** Present findings as proposals first.
- **Post to PR comment** so the user sees it on GitHub.
- **Be honest about disagreements** with codex findings.

</rules>
