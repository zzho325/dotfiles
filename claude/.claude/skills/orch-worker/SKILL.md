---
name: orch:worker
description: Autonomously complete a development task. Spawned by the orchestrator in a tmux session.
allowed-tools:
  - Read
  - Write
  - Edit
  - Bash
  - Glob
  - Grep
  - Task
  - Skill
  - WebFetch
  - WebSearch
---

<objective>

Autonomously complete the development task described in the task file passed as $ARGUMENTS.

Read the task file, understand what's being asked, check for resume state, and execute.

</objective>

<process>

## Phase 1: Read the Task

1. Read the task file at `$ARGUMENTS` (e.g. `~/tasks/foo.md`).
2. Read the `## Summary` and `## Status` sections to understand where things left off.
3. If the task has a `design:` line, read `$ORCH_REPO/.design/<name>/` for project context (especially `DESIGN.md`, `PLAN.md`, and any tickets).
4. Read `agents/dev-workflow.md` in the repo for technical commands (lint, test, build).

## Phase 2: Worktree Setup

**Always create a worktree** unless the task is purely reading code (no changes). If the task already has a `worktree:` line and the path exists, `cd` into it instead of creating a new one.

```bash
wt switch --create <feature-name> -y -C $ORCH_REPO
cd $ORCH_REPO/<feature-name>
```

If implementing against a ticket, also create a ticket branch inside the worktree: `git checkout -b ashley/ENG-<number>-<short-description>`

Always keep main up to date and rebase before starting:
```bash
git -C $ORCH_REPO/main pull --ff-only
# After creating worktree:
git -C $ORCH_REPO/ashley/<branch> rebase main
```

Report your worktree immediately after creating/switching to one.

## Phase 3: Design Before Implementing

Before writing any code, study the reference implementation and present a design for approval:

1. **Find the reference** — identify the closest existing pattern (e.g., monthly_payout.go for a payout endpoint). Read it end-to-end.
2. **Present field-by-field mapping** — for each param/field, show what the reference does and what you propose. Flag differences.
3. **Surface decisions** — don't assume. Present choices for: validation approach, error types, optional vs required fields, helper reuse, transaction scope.
4. **Error audit** — for every `return nil, err` in the handler, identify whether it could 500 and propose a typed error or justify letting middleware handle it.
5. **Helper check** — when duplicating logic from a reference, flag it: "This pattern exists in X, should I extract a helper?"
6. **Get approval** — only implement after alignment on the design.

## Phase 4: Execute

Follow the lifecycle:

1. **Scope** — understand the task, explore code
2. **Branch** — create worktree, report it
3. **Design** — study reference, present design, get approval (Phase 3)
4. **Implement** — write code, lint, test (see repo's `agents/dev-workflow.md`)
5. **Commit** — format: `area: ENG-<number> - description`
6. **Push** — `git push -u origin <branch>`
7. **PR** — only when the user is ready. Do NOT rush to create a PR before code is reviewed.
8. **Review** — address feedback, push fixes, notify orchestrator

## Communicating with the Orchestrator

**Always use `orch -`** to send updates. Never edit task files directly.

Report immediately after these events:
- Worktree created: `orch - "task-<name>: worktree $ORCH_REPO/ashley/<branch>"`
- Design created: `orch - "task-<name>: design <project-name>"`
- PR created: `orch - "task-<name>: PR created <url>, branch <branch>"`
- Review fixes pushed: `orch - "task-<name>: pushed review fixes"`
- Blocked or need input: `orch - "task-<name>: needs input: <question>"`
- Status update: `orch - "task-<name>: <what changed>"`

Your task name is derived from your tmux session name (e.g. `task-foo`).

</process>

<rules>

- **NEVER write, edit, or create files under `~/tasks/`.** The orchestrator is the sole writer to task files. Use `orch -` to communicate.
- If you're stuck or need input, report it via `orch -` and keep going on what you can.
- Never spawn other `claude` processes.
- Do the work. You are a worker, not a coordinator.

</rules>
