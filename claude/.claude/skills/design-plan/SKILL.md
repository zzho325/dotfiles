---
name: design:plan
description: Create implementation plan from DESIGN.md with workstreams, milestones, tasks, and dependencies
allowed-tools:
  - Read
  - Bash
  - Write
  - Glob
  - Task
  - AskUserQuestion
---

<objective>

Create implementation plan from design output. Takes DESIGN.md and produces PLAN.md with hierarchical breakdown (Workstreams → Milestones → Tasks), dependencies, and critical path analysis.

**Requires:**
- `$DESIGN_DIR/<slug>/DESIGN.md` — from `/design:design`

**Creates:**
- `$DESIGN_DIR/<slug>/PLAN.md` — hierarchical implementation plan

</objective>

<execution_context>

@.claude/design/templates/plan.md

</execution_context>

<process>

## Phase 1: Setup

**MANDATORY FIRST STEP — Verify project exists:**

1. **Resolve the design directory (shared across worktrees):**
   ```bash
   DESIGN_DIR="$(git rev-parse --git-common-dir)/.design"
   echo "Design directory: $DESIGN_DIR"
   ```

   Use `$DESIGN_DIR` for all project paths throughout this command.

2. **Find the project:**
   ```bash
   ls -la $DESIGN_DIR/ 2>/dev/null || echo "No $DESIGN_DIR directory"
   ```

   If no `$DESIGN_DIR/` directory:
   ```
   No project found. Run `/design:brainstorm` first to create a project.
   ```
   Exit command.

   If multiple projects exist, use AskUserQuestion:
   - header: "Select Project"
   - question: "Which project do you want to plan?"
   - options: [one option per project found]

3. **Verify DESIGN.md exists:**
   ```bash
   cat $DESIGN_DIR/<slug>/DESIGN.md
   ```

   If DESIGN.md doesn't exist:
   ```
   DESIGN.md not found for this project. Run `/design:design` first.
   ```
   Exit command.

4. **Load all context:**
   - Read PROJECT.md (requirements, goals)
   - Read DESIGN.md (components, requirement mapping)
   - Read DECISIONS.md (if exists - decision log with rationale)
   - Read CONTEXT.md (if exists - codebase patterns)

5. **Display summary:**

   ```
   ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    DESIGN ► PLANNING
   ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

   **Project:** [name from PROJECT.md]

   **Components from Design:**
   - [Component 1]
   - [Component 2]
   - ...

   **Requirements to cover:**
   - Must Have: [count]
   - Should Have: [count]
   ```

## Phase 2: Milestone Structure Selection

**Choose a structuring pattern before drafting the plan.**

Analyze DESIGN.md and PROJECT.md to determine which pattern (or combination) fits best. Consider:
- Does the project involve DB schema changes? → Recommend **Schema-first**
- Is it a small, focused change? → Recommend **Single milestone**
- Are there multiple independent features? → Recommend **Feature-sliced**
- Is it a layered system (DB → logic → API)? → Recommend **Horizontal layers**
- Does the project combine characteristics (e.g., schema changes + independent features)? → Recommend a **Hybrid** and describe the specific combination

**Before presenting the question, display your recommendation with reasoning:**

```
**Recommended structure: [Pattern name]**
[2-3 sentences explaining why this pattern fits the project. Reference specific
aspects of the design — e.g., "The design includes 3 new tables that should be
migrated independently before the API code depends on them."]
```

If recommending a hybrid, explain which patterns you're combining and why — e.g., "Schema-first for the migration milestones, then feature-sliced for the API endpoints."

Then use AskUserQuestion:

- header: "Plan Structure"
- question: "Which milestone structure do you want to use?"
- options (put the recommended one first, marked "(Recommended)"):
  - "Schema-first" — DB migrations ship independently before code changes. Best when schema changes need careful, isolated rollout.
  - "Feature-sliced" — Each feature/endpoint is a self-contained milestone (schema + code + tests). Best for independent features that can ship in any order.
  - "Horizontal layers" — Schema → Core logic → API layer → Cleanup. Best for layered systems where each layer builds on the previous.
  - "Single milestone" — Everything in one milestone. Best for small, focused projects with few tasks.

If the user selects a pattern, use it to guide how tasks are grouped into milestones in later phases.

If the user selects "Other," ask them to describe their preferred structure. They may want a hybrid of the above patterns or something entirely custom — use their description to guide milestone grouping.

## Phase 3: Pre-Planning Discussion

Use AskUserQuestion to understand preferences:

- header: "Planning Preferences"
- question: "Any constraints or preferences for the implementation order?"
- options:
  - "No preferences" — Let me suggest the best order
  - "Start with [specific area]" — I have a preference
  - "Sequential only" — No parallel work, one thing at a time

If user has preferences, ask inline for details.

**Clarify any ambiguities:**

If the design has unclear areas, ask about them now before breaking into tasks.

## Phase 4: Task Slicing Strategy

**MANDATORY — determine how work will be decomposed before breaking into tasks.**

@.claude/design/references/slicing-strategies.md

**Step 1: Analyze the types of changes involved.**

Review DESIGN.md and categorize the work:
- Does it involve database schema changes?
- Does it affect multiple API endpoints or operations?
- Does it add new fields/concerns across multiple surfaces?
- Is it building a new feature from scratch?

**Step 1b: Assess production data status for affected tables.**

**IMPORTANT — Before planning any backfill or data migration tasks, determine whether affected tables actually contain production data.**

For each table that the project modifies or adds columns to:

1. **Check if the table is new (created in this project or a recent unshipped project):**
   - Was the table introduced in a prior design project that hasn't shipped yet?
   - Search for merged PRs that create or populate the table: `gh pr list --search "CREATE TABLE <table_name>" --state merged`
   - Check if the feature that writes to this table is behind a feature flag or has been deployed

2. **Check for deploy/release evidence:**
   - Look for production deployment configs, feature flags, or release notes referencing the table
   - Check if any API endpoints that write to the table are live (merged and deployed)
   - Review CONTEXT.md or PROJECT.md for notes on feature status

3. **Classify each table:**
   - **Live in production (has data):** Table exists in production and is actively written to → backfill may be needed for schema changes
   - **Deployed but empty:** Table exists in production but no records have been written → no backfill needed
   - **Not yet deployed:** Table was created in a recent project that hasn't shipped → no backfill needed

4. **When uncertain, state the assumption explicitly** so the user can correct it before the plan is finalized. Example:
   ```
   ⚠️ ASSUMPTION: I'm assuming `card_tokens` has no production data because the VTS
   provisioning feature was built recently and hasn't shipped. If this is wrong,
   a backfill task will be needed. Please confirm.
   ```

**Step 2: Present analysis and recommend slicing strategies.**

Based on the analysis, recommend which slicing strategies apply. Display:

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 DESIGN ► TASK SLICING
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

**Types of changes detected:**
- [x] Database schema changes → [list tables/columns]
- [x] Multiple API endpoints → [list endpoints]
- [ ] Cross-cutting field additions
- [ ] New feature from scratch

**Production data status:** (for tables with schema changes)
- `table_name`: [Live — has production data | Deployed but empty | Not yet deployed]
- [If not yet deployed]: No backfill needed — feature not yet live

**Recommended slicing approach:**
1. **Schema-First** — migrate [tables] in separate tasks before code
2. **Verb/Endpoint Slicing** — one task per endpoint: [list]

**Default rules that will apply:**
- Schema migrations will be separate, independently deployable tasks
- Backfill tasks are only planned for tables with existing production data
- Each API endpoint gets its own task unless trivially mechanical
```

**Step 3: Get user confirmation on slicing approach.**

Use AskUserQuestion:
- header: "Slicing Strategy"
- question: "How should we decompose the tasks?"
- options:
  - "Use recommended approach" — Apply the strategies above
  - "Adjust" — I want to change the slicing approach
  - "Skip slicing guidance" — I'll handle decomposition myself

If user wants adjustments, discuss and agree on the approach before proceeding.

**Step 4: Record the chosen strategy.**

The agreed slicing strategy will be referenced during Phase 7 (Task Breakdown) and Phase 8 (Task Enrichment) to ensure tasks follow the chosen decomposition.

## Phase 5: Workstream Identification

> **Note:** Use the slicing strategy from Phase 4 to inform how workstreams are organized.

**Analyze components and requirements:**

Based on DESIGN.md components and PROJECT.md requirements, identify natural groupings for parallel work.

**Common workstream patterns:**
- By layer: Backend, Frontend, Infrastructure
- By feature area: Auth, Core Features, Integrations
- By component: Each major component is a workstream
- Single workstream: If project is small/focused

**Present workstreams:**

```
## Proposed Workstreams

1. **[Workstream 1]** - [what it covers]
   Components: [list]
   Requirements: [REQ-IDs]

2. **[Workstream 2]** - [what it covers]
   Components: [list]
   Requirements: [REQ-IDs]
```

Use AskUserQuestion:
- header: "Workstreams"
- question: "Does this workstream organization make sense?"
- options:
  - "Looks good" — Proceed
  - "Adjust" — Let me suggest changes

## Phase 6: Milestone Breakdown

For each workstream, identify milestones (shippable increments). **Use the structuring pattern selected in Phase 2** to guide how tasks are grouped into milestones.

**Milestone principles:**
- Each milestone is demoable/shippable
- Has clear exit criteria
- Builds on previous milestones
- Typically 3-7 tasks per milestone

**For each milestone, define:**
- Goal (what's shippable after)
- Exit criteria (how to verify it's done)
- Dependencies on other milestones

**Present milestones:**

```
### Workstream 1: [Name]

| Milestone | Goal | Depends on |
|-----------|------|------------|
| 1.1: [Name] | [goal] | - |
| 1.2: [Name] | [goal] | 1.1 |
```

Validate with user before proceeding to tasks.

## Phase 7: Task Breakdown

**IMPORTANT — Apply the slicing strategy chosen in Phase 4.**

For each milestone, identify tasks (one capability each), using the agreed slicing approach:

- If **Schema-First** was chosen: extract all database migrations into their own tasks at the front of the dependency chain. Code tasks depend on migration tasks but never contain migrations.
  - **Backfill check:** Only create backfill tasks for tables that have existing production data (determined in Phase 4, Step 1b). If a table was created in a recent project that hasn't shipped yet, or is deployed but empty, skip the backfill — note "no backfill needed — feature not yet live" in the migration task description. Do NOT assume existing data needs migration without evidence.
- If **Verb/Endpoint Slicing** was chosen: create one task per API endpoint or operation. Each task includes the full vertical (handler, service, store, tests).
- If **Field/Concern Slicing** was chosen: create one task per field or concern, traced end-to-end across all surfaces.
- If **Layer Slicing** was chosen: create tasks by architectural layer (model → store → handler → API).

Most projects combine strategies — e.g., Schema-First for migrations + Verb/Endpoint Slicing for API work.

**Co-locate utility functions with their first consumer:**

When the design involves shared utility functions (state transitions, helpers, validators, etc.), do NOT create a separate "write all helpers" task. Instead, each consumer task writes the utility function(s) it needs. Later tasks reuse functions written by earlier tasks — document this as a dependency. See the "Co-locate utility functions with their first consumer" heuristic in the slicing strategies reference.

**Task principles:**

A task delivers one new capability. Ask: *"What can we do after this task that we couldn't before?"*

| Good Task | Bad Task |
|-----------|----------|
| "Service runs with health check" | "Create main.go" |
| "Users can log in" | "Add auth middleware" |
| "Database queries work" | "Write SQL file" |
| "Deactivation migration deployed" | "Update schema and implement deactivate" |

**Right-sized tasks:**
- Half-day to 2 days of work
- Can be demonstrated
- Includes everything needed (code, config, tests)

**Task types:**
- **Build** - Implementation work
- **Spike** - Investigation
- **Docs** - Documentation
- **Manual** - Human action required

**Execution mode:**
- **Code** - Claude/agent can implement
- **Human** - Requires human action (terminal commands, config, deploy)

**For each task, identify:**
- Type and mode
- Dependencies on other tasks
- What it enables
- Whether it's on critical path

**Validate against slicing strategy** (skip if user chose "Skip slicing guidance" in Phase 4):
- If **Schema-First** was chosen: are schema migrations in separate tasks from code changes?
- If **Schema-First** was chosen: are backfill tasks only included for tables with verified production data? (No backfill for tables that are new or not yet deployed.)
- If **Verb/Endpoint Slicing** was chosen: does each API endpoint have its own task (unless trivially mechanical)?
- Are utility/helper functions co-located with their first consumer, not bundled into a separate "write all helpers" task?
- Does the decomposition match what was agreed in Phase 4?

## Phase 8: Task Enrichment

For each task, add full context:

**From PROJECT.md:**
- Why this matters (background)
- Requirements covered (REQ-IDs)

**From DESIGN.md and DECISIONS.md:**
- Files to modify
- Component it belongs to
- Technical approach
- Relevant decisions and their rationale (reference by decision ID, e.g., "per D-3")

**From CONTEXT.md (if exists):**
- Patterns to follow
- Existing code to reference

**Define acceptance criteria:**
- What can be demonstrated
- What tests should pass
- How to verify it works

### API Response Type Tasks — Field Specification

**For any task that creates or modifies an API response type**, the task description MUST include an explicit field list. Vague field descriptions like "all required fields" cause rework.

**Required for API response tasks:**

1. **Include an "API Fields" section** listing every field by name, type, and whether it's required or optional.

2. **Cross-reference from DESIGN.md.** If Phase 5b (API Field Specification) was completed during design, copy the approved field list directly. Reference the decision ID (e.g., "per D-5: CardToken fields").

3. **If field spec wasn't done in design**, do it now per task:
   - Identify the parent resource and read its API response type
   - Apply conventions: inherit parent IDs, expose `created_at` not `updated_at`, exclude internal fields
   - List fields explicitly

4. **Explicitly state excluded fields.** If the internal model has fields that are NOT exposed in the API (e.g., `platform_id`, `network_metadata`, `updated_at`), list them under "Fields NOT exposed" with the reason.

## Phase 9: Dependency Analysis

**Build dependency graph:**

For each task:
- What does it depend on?
- What does it enable?

**Identify critical path:**

The longest chain of dependent tasks. Delays here delay everything.

```
Critical path: 001 → 003 → 007 → 012 → 015
```

**Validate no circular dependencies.**

## Phase 10: User Review

Display the full plan:

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 DESIGN ► PLAN REVIEW
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

**Summary:**
- Workstreams: [count]
- Milestones: [count]
- Tasks: [count] ([X] Code, [Y] Human)

**Critical Path:** [count] tasks, estimated [X] sequential steps

[Show workstream/milestone/task structure]
```

Use AskUserQuestion:
- header: "Plan Review"
- question: "Does this plan look right?"
- options:
  - "Approve" — Write the plan
  - "Adjust tasks" — I want to change some tasks
  - "Adjust structure" — Change workstreams/milestones

If adjustments needed, loop back to relevant phase.

## Phase 11: Write PLAN.md

Synthesize into `$DESIGN_DIR/<slug>/PLAN.md` using the template.

**Include:**
- Summary table
- Critical path
- Dependency graph
- Workstreams with milestones
- Milestones with tasks
- Full task details (type, mode, dependencies, context, acceptance criteria)
- Traceability matrix
- Open questions

## Phase 12: Validate Dependencies

After writing PLAN.md, validate internal consistency of the dependency data.

**Step 1: Parse task dependency data from PLAN.md**

For each task in the plan:
- Extract task ID (e.g., 001, 002)
- Extract "Depends on" references
- Extract "Enables" references

**Step 2: Cross-check bidirectional consistency**

- If task A lists task B in "Depends on", verify task B lists task A in "Enables" (and vice versa)

**Step 3: Validate referenced tasks exist**

- Every task ID referenced in any dependency must correspond to an actual task in the plan

**Step 4: Check for circular dependencies**

- Walk the dependency graph and verify there are no cycles (e.g., A → B → C → A)

**Step 5: Validate critical path**

- The critical path should be a valid path through the dependency graph (each step follows an actual dependency edge)

**Reporting:**

If any inconsistencies are found, fix them in PLAN.md before finalizing:

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 DESIGN ► DEPENDENCY VALIDATION
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Errors found:

- Task 003 depends on 001, but task 001 does not list 003 in Enables
- Circular dependency detected: 004 → 006 → 004
- Task 007 references task 099 in Depends on, but task 099 does not exist

```

**If errors are found:** Fix all inconsistencies in PLAN.md, then re-run validation until clean.

**If no errors:** Proceed to completion.

**Record the Claude session ID:** Get the current session ID by reading the most recently modified `.jsonl` file in `~/.claude/projects/` for the current project directory. The filename (without `.jsonl` extension) is the session UUID.

```bash
ls -t ~/.claude/projects/$(echo "$PWD" | sed 's|/|-|g')/*.jsonl 2>/dev/null | head -1 | xargs basename | sed 's/\.jsonl$//'
```

Store this as `SESSION_ID` for use in metadata.json and PLAN.md footer.

**Add footer to PLAN.md:**

```markdown
---
*Last updated: [date] after planning*
*Claude session: [SESSION_ID]*
```

**Update metadata.json:**

Read the existing metadata.json first, then update it. Append a new entry to the `sessions` array.

```json
{
  "slug": "<slug>",
  "created_at": "<existing>",
  "phase": "plan",
  "brainstorm_complete": true,
  "design_complete": true,
  "plan_complete": true,
  "sessions": [
    {"phase": "brainstorm", "session_id": "<existing>", "timestamp": "<existing>"},
    {"phase": "design", "session_id": "<existing>", "timestamp": "<existing>"},
    {"phase": "plan", "session_id": "<SESSION_ID>", "timestamp": "<ISO timestamp>"}
  ]
}
```

## Phase 13: Done

Display completion:

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 DESIGN ► PLAN COMPLETE
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

**[Project Name]**

| Artifact | Location                         |
|----------|----------------------------------|
| Plan     | `$DESIGN_DIR/<slug>/PLAN.md`     |

**Workstreams:** [count]
**Milestones:** [count]
**Tasks:** [count] ([X] Code, [Y] Human)
**Critical Path:** [count] tasks

───────────────────────────────────────────────────────────────

## Next Up

Start implementing! Work through tasks in dependency order,
beginning with those on the critical path that have no blockers.

───────────────────────────────────────────────────────────────
```

</process>

<output>

- `$DESIGN_DIR/<slug>/PLAN.md` — hierarchical implementation plan
- Updated `metadata.json` with plan_complete flag

</output>

<success_criteria>

- [ ] DESIGN.md exists and was loaded
- [ ] All context loaded (PROJECT.md, CONTEXT.md)
- [ ] Milestone structuring pattern selected by user
- [ ] Slicing strategy analyzed and presented to user
- [ ] User confirmed slicing approach (or chose to skip)
- [ ] Schema migrations are separate tasks (unless user overrode)
- [ ] Production data status verified for all affected tables before planning backfill tasks
- [ ] Backfill tasks only included for tables with verified production data
- [ ] Assumptions about production data status stated explicitly when uncertain
- [ ] Workstreams identified and validated
- [ ] Milestones defined with exit criteria
- [ ] Tasks broken down following chosen slicing strategy
- [ ] Task types assigned (Build/Spike/Docs/Manual)
- [ ] Execution modes assigned (Code/Human)
- [ ] Dependencies mapped (depends-on/enables)
- [ ] Critical path identified
- [ ] All REQ-IDs covered by tasks
- [ ] User approved the plan
- [ ] PLAN.md written with full content
- [ ] API response type tasks include explicit field lists with exclusions documented
- [ ] Dependency validation passed:
  - [ ] Bidirectional consistency: every Depends on has matching Enables (and vice versa)
  - [ ] All referenced task IDs exist in PLAN.md
  - [ ] No circular dependencies
  - [ ] Critical path is a valid path through the dependency graph
- [ ] Claude session ID recorded in metadata.json and PLAN.md footer
- [ ] metadata.json updated

</success_criteria>
</output>
</output>
