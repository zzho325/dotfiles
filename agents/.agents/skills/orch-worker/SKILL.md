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

<objective>

Complete the development task described in the task file passed as $ARGUMENTS.

Read the task file, understand what's being asked, check for resume state, and execute.

</objective>

<process>

## Phase 1: Read the Task

1. Read the task file at `$ARGUMENTS` (e.g. `~/tasks/foo.md`).
2. Read the `## Summary` and `## Status` sections to understand where things left off.

## Phase 2: Worktree Setup

The orchestrator pre-creates a worktree and spawns you inside it. Trust the
contract; don't run `git`/`wt` yourself to create one. If `pwd` is somehow
`$ORCH_REPO/main` or otherwise wrong, surface it via notes.md (interactive)
or `orch -` (autonomous); don't paper over it.

When making a bookmark for a task that has a Linear ticket, name it
`ashley/ENG-<number>-<short-desc>`. Orch's auto-scan reads bookmark names on
the worktree's stack and links the ticket; you don't need to run
`orch linear add`.

## Phase 3: Gather Context & Determine Mode

1. If the task has a `design:` line, read `docs/design/<name>/` in the repo for
   project context (especially `DESIGN.md`, `PLAN.md`, and any tickets).
2. Read `agents/dev-workflow.md` in the repo for technical commands (lint, test, build).
3. Check for a `notes.md` in the repo root — if it exists, read it for WIP context.

**Interactive mode** (user is present in conversation):
- Communicate through `notes.md` in the repo root (see Communication section).
- Do NOT use `orch -` to report to orchestrator.
- Surface decisions and questions in notes.md, wait for user input.

**Autonomous mode** (spawned by orchestrator in tmux):
- Use `orch -` to send updates. Never edit task files directly.
- Report immediately after: worktree created, design ready, blocked.

## Phase 4: Design Before Implementing

Before writing any code, study the reference implementation and present a design
for approval:

1. **Find the reference** — identify the closest existing pattern (e.g.,
   monthly_payout.go for a payout endpoint). Read it end-to-end.
2. **Present field-by-field mapping** — for each param/field, show what the
   reference does and what you propose. Flag differences.
3. **Surface decisions** — don't assume. Present choices for: validation approach,
   error types, optional vs required fields, helper reuse, transaction scope.
4. **Error audit** — for every `return nil, err` in the handler, identify whether
   it could 500 and propose a typed error or justify letting middleware handle it.
5. **Helper check** — when duplicating logic from a reference, flag it: "This
   pattern exists in X, should I extract a helper?"
6. **Get approval** — only implement after alignment on the design.

**Tip: ASCII tree visualizations.** When the user asks to see the structure of
code, tests, or files, present it as an indented tree with one-line annotations:
- Code call graph: `funcName(args)` with `[I/O, cached]` / `[pure]` annotations
- Test plan: `TestFunc → case_name — description`
- File layout: paths with purpose annotations

Example (test plan):
```
validate_test.go
├── TestVerifySchema
│   ├── valid_ccd_credit              — standard CCD credit passes
│   ├── negative_amount               — amount < 0
│   └── missing_entry_class_code      — nil SEC
└── TestValidateRecords (integration)
    ├── happy_path                    — 3 valid, aggregates correct
    └── mixed_valid_invalid           — 2 valid + 1 invalid
```

## Codex

When the user asks to use codex (e.g. "get codex review", "ask codex",
"check with codex"), invoke the `/codex` skill:
- Review: `/codex https://github.com/column/column/pull/25827`
- Question: `/codex "is this the right approach for X?"`

After codex returns, write each finding to notes.md with your
response (agree/disagree/already handled) under each item:
```
notes propose "Codex review" -b "1. <finding>\n   → <your response>\n\n2. <finding>\n   → <your response>"
```
**Never auto-fix from codex feedback** — always discuss first.

### Background PR Review

For PR reviews, prefer orch's background review mailbox when the user wants the
review to run while you keep working:

```
orch review start https://github.com/column/column/pull/25827
orch review list
orch review show <id> --consume
```

`orch review start` returns immediately. A Codex hook will later add context
like:

```
A background PR review is ready: ... Run `orch review show <id> --consume` ...
```

When that happens:

1. Run exactly the suggested `orch review show <id> --consume`.
2. Present each finding in `notes.md` as a proposal, with your response under
   it: agree, disagree, already handled, or needs inspection.
3. Do not auto-fix review feedback. Wait for the user to stamp/approve the
   proposal.

Use `/codex` directly for non-PR questions, quick second opinions, or when the
user explicitly asks for a foreground Codex answer.

## Phase 5: Execute

1. **Implement** — write code (design already approved in Phase 4)
2. **Verify** — build, lint, test before committing:
   - `go build ./affected/packages/...`
   - `golangci-lint run --allow-parallel-runners ./affected/packages/...`
   - `ENV=test go test -v ./affected/packages/... -run '^TestName$' -count=1`
   Record results in notes.md.
3. **Commit** — `jj describe` with format `type(area): description`
   (type = fix/feat/refactor, no ticket numbers).
4. **Push** — `jj git push`
5. **Propose PR** — after pushing, propose branch, title, and PR description
   in notes.md. Do NOT create the PR yet.
6. **PR** — only after the user stamps the proposal. Create with `gh pr create`.
7. **Review** — address feedback, push fixes

</process>

## Communication (Interactive Mode)

**Two channels: conversation for discussion, notes.md for conclusions.**

- **Discuss in conversation** — design questions, back-and-forth, Q&A, trade-offs.
  Don't put discussion threads in notes.md — they accumulate and make it unreadable.
- **Write conclusions in notes.md** — finalized designs, decisions, PR details.
  When alignment is reached, resolve old threads and write a fresh thread with
  the up-to-date state. notes.md should always be readable as a standalone doc.
- **When user leaves questions/comments in notes.md** — respond in conversation,
  then update notes.md with the clean conclusion. Don't reply inline in notes.md
  unless it's a brief factual annotation.

## notes.md

Use `notes.md` in the repo root as a shared scratchpad for conclusions, designs,
and code proposals. Keep it clean — resolve stale threads, don't let Q&A accumulate.

**NEVER use Edit or Write on `notes.md`. Always use the `notes` CLI
(`~/bin/notes`) for every interaction with it — creating threads, replying,
resolving, proposing, stamping, deleting, cleanup.** The CLI enforces structure
the Edit tool will silently break. If a command you need doesn't exist, tell
the user — do not fall back to Edit.

**Write concisely, prefer visuals. The user has trouble reading long text.**
Every thread, proposal, or reply should be as short as the content allows — no
filler, no producty framing, no "here's what I'll do" preamble. Use any visual
form that makes the content faster to parse than prose:

- Tables for comparisons, before/after, field mappings, status summaries
- ASCII trees for call graphs, file layouts, test plans
- Flow/state diagrams for control flow, state machines, decision logic
- Sequence/ladder diagrams for request flows, interactions
- Fenced code blocks for diffs, EXPLAIN output, signatures
- Bullet lists over paragraphs

If you catch yourself writing more than 2-3 sentences of prose to explain
something, stop and ask whether a table, diagram, or diff block would convey it
more clearly. Pick whichever visual form fits — don't default to one.

Command reference:

```
notes wip                # list WIP threads
notes wip "title"        # add WIP thread (title only)
notes wip "title" -      # add WIP thread (pipe body via stdin)
notes reply <N> "text"   # append to WIP thread N
notes reply <N> "text" - # append text + stdin to thread N
notes resolve <N>        # move thread N to Done
notes resolve all        # resolve all WIP threads
notes done               # list Done summaries
notes propose "desc"              # add proposal (title only)
notes propose "desc" -b "body"   # add proposal with body
notes update <N> "desc"          # update proposal N description
notes update <N> "desc" -b "body"  # update desc + replace body
notes proposals          # list proposals with [x]/[ ] status
notes stamp <N>          # mark proposal N as approved [x]
notes delete <N>         # delete proposal N
notes approved           # list only stamped [x] proposals
notes applied            # move [x] proposals to Done
notes -f other.md wip    # operate on a different notes file
```

**Thread hygiene:**
- **Resolve after user acknowledges** — user stamps, says "ok", asks a
  follow-up, or moves to a new topic = thread is done. Run
  `notes resolve <N>` in the same message.
- **After applying stamped proposals** — run `notes applied` to clean up.
- **Keep WIP small** — run `notes wip` and resolve acknowledged ones
  before adding more.
- **Clean up old proposals** — before proposing new changes, run
  `notes proposals` and delete or resolve any stale ones. Old unreviewed
  proposals clutter the file and confuse the user.

**notes.md and jj:** `notes.md` must be gitignored AND untracked by jj. If it
disappears when switching revisions, a revision is still tracking it — check with
`jj file list -r <rev> | grep notes` and untrack with `jj file untrack notes.md`.

## Code Approval Workflow

**Do not write code until the user approves.** This applies to all code edits
(Write, Edit). Reading files, searching, and updating notes.md do not require
approval.

1. Research — read code, check patterns, gather context (tools are fine).
2. Propose — use `notes propose "file.go: description"` to add proposals.
   For multi-line body: `printf 'detail line 1\ndetail line 2' | notes propose "title" -`
3. Wait — user marks `[x]` in vim or adds inline comments.
4. Check — run `notes approved` to see what's ready.
5. Implement — apply all approved items.
6. Clean up — run `notes applied` to move stamped proposals to Done.

**Proposal granularity:**
- **One change at a time.** When a task has multiple changes, propose and get
  approval for the first one before moving to the next.
- **Code first, tests second.** Propose the logic change as one proposal. After
  it's approved and implemented, propose the test changes as a separate follow-up.
- **Be detailed.** Show exact struct field placement, function signatures, and
  code snippets with surrounding context so the user can evaluate precisely what
  will change. Reference line numbers and follow existing ordering conventions
  (e.g., match the model's field order in related structs).

**Approval means the user stamps/marks items `[x]`.** If the user asks a question
about a proposed change (e.g. "should we also check X?"), that is NOT approval to
implement — respond in notes.md and wait for the stamp. Only implement when items
are explicitly marked `[x]`.

## jj (Jujutsu)

If the repo uses jj (check for `.jj/` directory), invoke the `/jj` skill for the
full reference on bookmarks, stacked PRs, absorb, and rebase workflows.

<rules>

- **NEVER write, edit, or create files under `~/tasks/`.** The orchestrator is the sole writer to task files.
- **NEVER use Edit or Write on `notes.md`.** Always use the `notes` CLI for every notes.md interaction, no exceptions.
- If you're stuck or need input, surface it in notes.md (interactive) or `orch -` (autonomous).
- Never spawn other `claude` processes.
- Do the work. You are a worker, not a coordinator.

</rules>
