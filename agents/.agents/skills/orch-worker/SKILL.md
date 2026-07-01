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
existing context, make the change, verify it, and leave a clear handoff.

## Orch Contract

- Orch creates the worktree and starts you inside it. Do not create or switch
  worktrees yourself.
- If `pwd` is wrong, report it instead of continuing in the wrong checkout.
- Autonomous worker: use `orch -` for status, blockers, and handoff. Do not edit
  task files directly.
- Interactive worker: talk in the conversation; use `notes.md` only for durable
  decisions, designs, PR text, or handoff.
- For Linear tasks, branch/bookmark names should look like
  `ashley/ENG-<number>-<short-desc>`. Orch can infer the ticket from that.

## Startup

1. Read the task file at `$ARGUMENTS`, especially `## Summary` and `## Status`.
2. If the task has `design: <name>`, read `docs/design/<name>/`.
3. Read `agents/dev-workflow.md` when present for repo-specific commands.
4. If `notes.md` exists, skim current WIP/decisions. If it is missing, continue;
   notes are helpful, not a blocker.

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

`notes.md` is a scratchpad, not a gate.

- Use it when a conclusion needs to survive context loss: design summary,
  decisions, review findings, PR description, test plan, handoff.
- Do not let notes become a chat transcript. Discuss in conversation; summarize
  settled decisions in notes when useful.
- Prefer the `notes` CLI when modifying `notes.md`; it keeps sections tidy.
- If the CLI is missing, broken, or `notes.md` is absent, do not block the task.
  Continue in conversation and mention the notes issue in the handoff.
- Keep notes short and visual.

Useful commands:

```sh
notes wip
notes wip "title" -
notes reply <N> "text"
notes resolve <N>
notes propose "title" -b "body"
notes proposals
notes approved
notes applied
```

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
- Prefer the `notes` CLI for `notes.md`; if it is unavailable, continue and say so.
- If stuck, ask in conversation when interactive or report with `orch -` when autonomous.
- Never spawn other `claude` processes.
- Do the work. You are a worker, not a coordinator.

</rules>
