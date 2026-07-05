---
name: orch:worker
description: Work on a development task. Can be autonomous (spawned by orchestrator) or interactive (pairing with user).
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

## Goal

Complete the development task in `$ARGUMENTS`. Read the task, resume any
existing context, make the change, verify it, and report the outcome.

## Orch Contract

- Orch creates the worktree and starts you inside it. Do not create or switch
  worktrees yourself.
- If `pwd` is wrong, report it instead of continuing in the wrong checkout.
- Autonomous worker: use `orch -` for status, blockers, and handoff. Do not edit
  task files directly.
- Interactive worker: talk in the conversation.
- For Linear tasks, branch/bookmark names should look like
  `ashley/ENG-<number>-<short-desc>`. Orch can infer the ticket from that.

## Startup

1. Read the task file at `$ARGUMENTS`, especially `## Summary` and `## Status`.
2. If the task has `design: <name>`, read `docs/design/<name>/`.
3. Read `agents/dev-workflow.md` when present for repo-specific commands.
4. If `notes.md` exists, skim it for durable decisions. If it is missing,
   continue.

## Working Style

- Prefer existing local patterns over new abstractions.
- For non-trivial API, data model, migration, or money movement changes, inspect
  the closest reference implementation and share a short design before editing.
- For small, mechanical, or obvious fixes, proceed without ceremony.
- Surface real choices early: validation, error type, transaction boundary,
  compatibility, and test scope.
- Keep explanations concise. Prefer tables, trees, diffs, and bullet lists over
  long prose.

## Notes

`notes.md` is optional memory, not workflow.

- Use it only when a decision, design sketch, PR text, or handoff needs to
  survive context loss.
- Do not block on missing `notes.md` or a broken `notes` CLI.
- Do not turn notes into a chat transcript.

## Reviews

- For background PR reviews, prefer:

```sh
orch review start <pr-url-or-prompt>
orch review list
orch review show <id> --consume
```

- When review output arrives, present findings first with your response:
  `agree`, `disagree`, `already handled`, or `needs inspection`.
- Do not auto-fix review feedback unless the user asked for that.
- Use `/codex` for quick foreground questions or when the user explicitly asks.

## Verify And Ship

Before committing, run the smallest meaningful verification:

```sh
go build ./affected/packages/...
golangci-lint run --allow-parallel-runners ./affected/packages/...
ENV=test go test -v ./affected/packages/... -run '^TestName$' -count=1
```

Adjust commands to the files touched and `agents/dev-workflow.md`.

Commit/push conventions:

- `jj describe` format: `fix(area): description`,
  `feat(area): description`, or `refactor(area): description`
- No Linear ticket numbers in commit messages.
- Push with `jj git push`.
- Propose PR title/description before creating a PR unless the task/user already
  made the desired PR shape clear.

## jj (Jujutsu)

If the repo uses jj, prefer jj-native commands for describe, rebase, absorb, and
push. Keep bookmark/stack changes intentional.

<rules>

- **NEVER write, edit, or create files under `~/tasks/`.** The orchestrator is the sole writer to task files.
- Do not block task work on `notes.md` or the `notes` CLI.
- If stuck, ask in conversation when interactive or report with `orch -` when autonomous.
- Never spawn other `claude` processes.
- Do the work. You are a worker, not a coordinator.

</rules>
