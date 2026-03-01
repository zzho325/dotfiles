---
name: design:retro
description: Run a retrospective on this session and surface improvement opportunities
allowed-tools:
  - Read
  - Bash
  - Write
  - Glob
  - Task
---

<objective>

Run a retrospective on the current session. Reviews the conversation for issues, mistakes, and improvement opportunities, then writes a structured summary.

**Creates:**
- `$DESIGN_DIR/<slug>/RETRO.md` — retrospective summary (if a design project exists)

This command can run at the end of any design phase or standalone after any session.

</objective>

<process>

## Phase 1: Setup

1. **Resolve the design directory (shared across worktrees):**
   ```bash
   DESIGN_DIR="$(git rev-parse --git-common-dir)/.design"
   echo "Design directory: $DESIGN_DIR"
   ```

2. **Check for existing design project:**
   ```bash
   ls -la $DESIGN_DIR/ 2>/dev/null || echo "No $DESIGN_DIR directory"
   ```

   If a `$DESIGN_DIR/` directory exists, identify the active project slug. If multiple projects exist, use the most recently modified one. Load `metadata.json` to understand which phases have been completed.

   If no `$DESIGN_DIR/` directory, that's fine — this command can run standalone after any session.

## Phase 2: Session Review

Display:
```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 DESIGN ► RETROSPECTIVE
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

**Review the entire conversation for issues in these categories:**

### Category 1: Getting Stuck / Asking for Input (HIGHEST PRIORITY)

The agent should be fully autonomous. Every time it stopped and waited for user input is a problem. Look for:

- Times you asked the user a question or requested confirmation
- Times you waited for user input before continuing
- Times you were unsure how to proceed and paused

For each instance, identify what caused the stoppage:
- Missing documentation (AGENTS.md, skill docs, etc.)
- Unclear ticket description
- Missing permissions
- Ambiguous instructions
- Tool limitation

### Category 2: Permission Prompts

Any time a tool use was blocked by a permission prompt. These slow down autonomous work and should be allowlisted. Look for:

- Bash commands that required approval
- File access that required approval
- Tool uses that were blocked or required confirmation

### Category 3: User Corrections

Times the user corrected or redirected the agent. Look for:

- Direct corrections ("No, do it this way instead")
- Redirections ("That's not what I meant")
- Repeated instructions (user had to say the same thing twice)

### Category 4: Process Mistakes

- Commits without pushing (every commit should be pushed)
- Not studying related work first (sibling PRs, parent branches, recently merged PRs)
- Not following explicit instructions
- Unauthorized git operations
- PR description out of sync after amending/force-pushing
- Not reading relevant documentation before starting

### Category 5: Recurring Errors

- Repeated failures of the same type
- Patterns of mistakes (not one-off errors)
- Missing knowledge that caused confusion

## Phase 3: Write RETRO.md

If a `$DESIGN_DIR/<slug>/` directory exists, write a retrospective summary.

**Categorize each issue:**
- `[Bug]` — Bugs in the tooling itself
- `[Docs]` — Documentation updates needed (AGENTS.md, skills, etc.)
- `[Allowlist]` — Permission prompts that blocked progress
- `[Feature]` — Feature requests for improvements

**Each issue MUST have enough context for someone to fix it without asking questions:**
1. **What happened** — What the agent was doing when the issue occurred
2. **What went wrong** — The specific error, correction, or problem
3. **How to fix it** — A concrete action (update AGENTS.md, fix bug, add to allowlist)

Skip non-actionable patterns (generic exit codes, user confirmations like "ok"/"yes", vague corrections without context).

Write to `$DESIGN_DIR/<slug>/RETRO.md`:

```markdown
# Retrospective

*Date: [date]*
*Session: [session description]*

## Issues Found

| # | Category | Issue | Severity |
|---|----------|-------|----------|
| 1 | [Docs/Bug/Allowlist/Feature] | [Brief description] | [High/Medium/Low] |
| 2 | ... | ... | ... |

## Details

### 1. [Issue title]

**Category:** [Docs/Bug/Allowlist/Feature]
**Severity:** [High/Medium/Low]

**What happened:** [Context]
**What went wrong:** [Problem]
**How to fix:** [Action]

### 2. ...

---
*Generated: [date]*
```

**Update metadata.json** (if it exists):

Read the existing metadata.json, then update:
- Set `"retro_complete": true`
- Append a new session entry: `{"phase": "retro", "session_id": "<SESSION_ID>", "timestamp": "<ISO timestamp>"}`

**Record the Claude session ID:**
```bash
ls -t ~/.claude/projects/$(echo "$PWD" | sed 's|/|-|g')/*.jsonl 2>/dev/null | head -1 | xargs basename | sed 's/\.jsonl$//'
```

## Phase 4: Done

Display completion:

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 DESIGN ► RETROSPECTIVE COMPLETE
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

**Issues found:** [N]

## Issues

| # | Category | Issue | Severity |
|---|----------|-------|----------|
| 1 | [Docs] | [Title] | [High] |
| 2 | [Allowlist] | [Title] | [Medium] |
| ... | ... | ... | ... |

[If RETRO.md was written:]
Retrospective saved to: `$DESIGN_DIR/<slug>/RETRO.md`

───────────────────────────────────────────────────────────────
```

</process>

<output>

- `$DESIGN_DIR/<slug>/RETRO.md` — retrospective summary (if design project exists)
- Updated `metadata.json` with retro_complete flag (if design project exists)

</output>

<success_criteria>

- [ ] Session reviewed for all issue categories
- [ ] Getting stuck / asking for input identified (highest priority)
- [ ] Permission prompts identified
- [ ] User corrections identified
- [ ] Process mistakes identified
- [ ] Recurring errors identified
- [ ] Non-actionable patterns skipped
- [ ] RETRO.md written with actionable issues (if design project exists)
- [ ] Each issue has full context (what happened, what went wrong, how to fix)
- [ ] metadata.json updated (if it exists)

</success_criteria>
</output>
</output>
