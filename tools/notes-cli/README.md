# notes-cli

CLI for managing `notes.md` scratchpad threads and proposals.
Used by the `orch-worker` skill for structured agent-human communication.

## Install

```bash
cd ~/dotfiles/tools/notes-cli && go build -o ~/bin/notes .
```

## Commands

### Threads (WIP / Done)

```bash
notes wip              # list WIP threads (number + first line)
notes resolve <N>      # move WIP thread N to Done
notes resolve all      # resolve all WIP threads
notes done             # list Done thread summaries
```

### Proposals

```bash
notes propose "desc"              # add proposal (title only)
notes propose "desc" -b "body"   # add proposal with body (-b flag)
notes proposals        # list all proposals with [x]/[ ] status
notes delete "N"       # delete proposal N (no move to Done)
notes approved         # list only stamped [x] proposals
notes applied          # move stamped [x] proposals to Done, clean up
```

## File format

Expects `notes.md` in the current directory with this structure:

```markdown
### WIP

1. user question
   > worker response

### Proposed changes
Mark [x] to approve, add comment to discuss.

- [ ] **P1** description
- [x] **P2** description

### Done

<details><summary>Resolved: topic</summary>
...
</details>
```

The CLI parses numbered items under `### WIP` and `- [ ] **PN**` lines
under `### Proposed changes`. `### Done` holds collapsed resolved items.
