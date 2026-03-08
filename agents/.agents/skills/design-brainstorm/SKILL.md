---
name: design:brainstorm
description: Initialize a new project through deep questioning, research, and brainstorming
allowed-tools:
  - Read
  - Bash
  - Write
  - Glob
  - Task
  - AskUserQuestion
---

<objective>

Initialize a new project through unified flow: codebase context → domain research (optional) → deep questioning → PROJECT.md.

This is the most leveraged moment in any project. Deep questioning here means better plans, better execution, better outcomes.

**Creates:**
- `$DESIGN_DIR/<slug>/PROJECT.md` — project definition with requirements
- `$DESIGN_DIR/<slug>/CONTEXT.md` — codebase analysis (if existing code)
- `$DESIGN_DIR/<slug>/RESEARCH.md` — domain research (optional)
- `$DESIGN_DIR/<slug>/metadata.json` — project state

**After this command:** Run `/design:design` to create technical design.

</objective>

<execution_context>

@.claude/design/references/questioning.md
@.claude/design/templates/project.md

</execution_context>

<process>

## Phase 1: Setup

**MANDATORY FIRST STEP — Execute these checks before ANY user interaction:**

1. **Resolve the design directory (shared across worktrees):**
   ```bash
   DESIGN_DIR="$(git rev-parse --git-common-dir)/.design"
   echo "Design directory: $DESIGN_DIR"
   ```

   Use `$DESIGN_DIR` for all project paths throughout this command.

2. **Check for existing projects:**
   ```bash
   ls -la $DESIGN_DIR/ 2>/dev/null || echo "No $DESIGN_DIR directory"
   ```

   If `$DESIGN_DIR/` exists with projects, use AskUserQuestion:
   - header: "Existing Projects"
   - question: "Found existing design projects. What would you like to do?"
   - options:
     - "Start new project" — Initialize a new project
     - "Resume [project-name]" — Continue an existing project (one option per project found)

3. **If new project, get project description:**

   Ask inline (freeform): "Describe what you want to build in a sentence or two."

   Auto-generate a kebab-case slug from the description:
   - Extract the key noun phrases (drop filler words like "a", "the", "for", "to", "and", "with", "that", "this", "should", "will", "can", "we", "need", "want", "update", "add", "implement", "create", "build", "make")
   - Keep 3-5 significant words maximum
   - Lowercase, hyphen-separated, under 50 characters
   - Examples:
     - "Update card token state machine for initialization" → `card-token-state-machine`
     - "Add retry logic to webhook delivery" → `webhook-delivery-retry`
     - "Build a CLI tool for managing deployments" → `deployment-cli`

   Store the description as `PROJECT_DESCRIPTION` for use in Phase 4.

4. **Create project directory (don't ask for confirmation):**

   First, check for slug collisions:
   ```bash
   ls -d $DESIGN_DIR/<slug> 2>/dev/null
   ```
   If the directory already exists, append a numeric suffix (`-2`, `-3`, etc.) until the slug is unique.

   Display the generated slug but do NOT block on user confirmation:
   ```
   Project slug: <slug>
   ```

   ```bash
   mkdir -p $DESIGN_DIR/<slug>
   ```

   The user can rename the directory later if they want a different slug.

5. **Detect existing code (brownfield detection):**
   ```bash
   CODE_FILES=$(find . -name "*.ts" -o -name "*.tsx" -o -name "*.js" -o -name "*.py" -o -name "*.go" -o -name "*.rs" -o -name "*.java" 2>/dev/null | grep -v node_modules | grep -v .git | grep -v vendor | head -20)
   HAS_PACKAGE=$([ -f package.json ] || [ -f requirements.txt ] || [ -f go.mod ] || [ -f Cargo.toml ] && echo "yes")
   ```

## Phase 2: Codebase Context

**If existing code detected (`CODE_FILES` is non-empty OR `HAS_PACKAGE` is "yes"):**

Display:
```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 DESIGN ► ANALYZING CODEBASE
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

Spawn exploration agent to understand existing codebase:

```
Task(
  prompt="Analyze this codebase to understand its architecture and patterns.

  Focus on:
  1. Overall architecture (monolith, microservices, modules)
  2. Key directories and their purposes
  3. Coding patterns and conventions used
  4. Tech stack (languages, frameworks, databases)
  5. How new features are typically added

  Write findings to: $DESIGN_DIR/<slug>/CONTEXT.md

  Format:
  # Codebase Context

  ## Architecture
  [Overview of how the codebase is structured]

  ## Key Directories
  - `path/` — [purpose]

  ## Tech Stack
  - [Language/framework] — [how it's used]

  ## Patterns & Conventions
  - [Pattern] — [where it's used]

  ## Adding New Features
  [How new features typically get added based on existing patterns]
  ",
  subagent_type="explore",
  description="Analyze codebase"
)
```

After agent completes, read CONTEXT.md and display key findings:
```
## Codebase Context

**Architecture:** [from CONTEXT.md]
**Stack:** [from CONTEXT.md]
**Conventions:** [from CONTEXT.md]

Saved to: `$DESIGN_DIR/<slug>/CONTEXT.md`
```

**If no existing code (greenfield):** Skip to Phase 3.

## Phase 3: Domain Research (Optional)

Use AskUserQuestion:
- header: "Research"
- question: "Research the domain before we dive into requirements?"
- options:
  - "Yes, research first" — Discover standard approaches, common features, pitfalls
  - "Skip research" — I know this domain, go straight to questioning

**If "Yes, research first":**

Display:
```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 DESIGN ► RESEARCHING DOMAIN
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

Ask inline: "What type of thing are you building? (e.g., 'CLI tool', 'REST API', 'authentication system')"

Spawn research agent:

```
Task(
  prompt="Research [domain] to understand standard approaches and best practices.

  Investigate:
  1. Standard approaches for building [domain]
  2. Common features users expect (table stakes vs differentiators)
  3. Best practices and patterns
  4. Common pitfalls and mistakes to avoid

  Write findings to: $DESIGN_DIR/<slug>/RESEARCH.md

  Format:
  # Domain Research: [Domain]

  ## Standard Approaches
  [How [domain] is typically built]

  ## Common Features
  ### Table Stakes (users expect these)
  - [Feature]

  ### Differentiators (competitive advantages)
  - [Feature]

  ## Best Practices
  - [Practice] — [why]

  ## Common Pitfalls
  - [Pitfall] — [how to avoid]
  ",
  subagent_type="explore",
  description="Research domain"
)
```

After agent completes, display key findings:
```
## Domain Research

**Standard approach:** [from RESEARCH.md]
**Table stakes:** [from RESEARCH.md]
**Watch out for:** [from RESEARCH.md]

Saved to: `$DESIGN_DIR/<slug>/RESEARCH.md`
```

**If "Skip research":** Continue to Phase 4.

## Phase 4: Deep Questioning

Display:
```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 DESIGN ► BRAINSTORMING
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

**Default assumptions (propose, don't ask):**

Start with these defaults and state them upfront. Only change if the user corrects:
- **Scope:** Design + implement (not design-only). Don't ask "design only or design + implement?" — assume both.
- **Deliverable:** Working code merged to the codebase, not just a document.

**Open the conversation:**

If `PROJECT_DESCRIPTION` is set (new project path): use it as the starting context — the user already described what they want to build. Acknowledge their description and immediately begin probing with follow-up questions. Do NOT re-ask "What do you want to build?"

If `PROJECT_DESCRIPTION` is not set (resume path): read the existing `$DESIGN_DIR/<slug>/PROJECT.md` to recover context. If PROJECT.md doesn't exist yet (brainstorm was interrupted before writing it), ask inline (freeform): "What do you want to build?"

In either case, use AskUserQuestion with options that probe the description — interpretations, clarifications, concrete examples.

Keep following threads. Each answer opens new threads to explore. Ask about:
- What excited them
- What problem sparked this
- What they mean by vague terms
- What it would actually look like
- What's already decided

**Batch related questions:** When multiple independent questions arise, present them together in one message using numbered items instead of asking one at a time. For freeform questions, list them inline. For structured choices, use a single AskUserQuestion where the `question` field contains numbered sub-questions and `options` combine the most likely answer sets. Only ask sequentially when the answer to one question determines the next.

Consult `references/questioning.md` for techniques:
- Challenge vagueness
- Make abstract concrete
- Surface assumptions
- Find edges
- Reveal motivation
- **Propose-and-correct** — when the user's description implies an answer, propose it with rationale instead of asking an open question

**Reference research if available:**

If CONTEXT.md exists, use it to propose how this fits into existing code (don't ask — propose with rationale and let the user correct).
If RESEARCH.md exists, use it to propose which standard features apply and which don't (present a categorized list rather than asking about each one).

**Scope requirements (propose-and-correct):**

As requirements emerge, propose the full categorization with your reasoning — don't ask the user to categorize from scratch. Present your proposed Must/Should/Won't breakdown and let the user correct:

Display inline (NOT AskUserQuestion):
```
Here's how I'd scope the requirements based on what you've described:

**Must Have** (blocking for v1)
- [Requirement 1] — [why you think it's must-have]
- [Requirement 2] — [why you think it's must-have]

**Should Have** (important but not blocking)
- [Requirement 3] — [why you think it's should-have]

**Won't Have** (explicitly out of scope)
- [Requirement 4] — [why you think it should be excluded]

Does this match your priorities? Tell me what to move.
```

Then use AskUserQuestion only if the user wants to adjust specific items:

- header: "Adjust Priorities"
- question: "Which items should change priority?"
- multiSelect: true
- options: [list items the user flagged for change]

This avoids the anti-pattern of asking "which are Must Have?" when you already have enough context to propose the answer.

**Decision gate:**

When you could write a clear PROJECT.md, use AskUserQuestion:

- header: "Ready?"
- question: "I think I understand what you're building. Ready to create PROJECT.md?"
- options:
  - "Create PROJECT.md" — Let's move forward
  - "Keep exploring" — I want to share more

If "Keep exploring" — ask what they want to add, or probe remaining gaps.

Loop until "Create PROJECT.md" selected.

## Phase 5: Write PROJECT.md

Synthesize all context into `$DESIGN_DIR/<slug>/PROJECT.md` using the template from `templates/project.md`.

**Include:**
- Everything gathered from questioning
- Context from CONTEXT.md (if exists)
- Insights from RESEARCH.md (if exists)
- Scoped requirements with REQ-IDs

**Requirements format:**

```markdown
## Requirements

### Must Have
- [ ] **REQ-01**: [User can X] — [brief rationale]
- [ ] **REQ-02**: [User can Y] — [brief rationale]

### Should Have
- [ ] **REQ-03**: [User can Z] — [brief rationale]

### Won't Have
- [Feature] — [why excluded]
```

**Last updated footer:**

```markdown
---
*Last updated: [date] after brainstorming*
*Claude session: [session-id]*
```

**Record the Claude session ID:** Get the current session ID by reading the most recently modified `.jsonl` file in `~/.claude/projects/` for the current project directory. The filename (without `.jsonl` extension) is the session UUID.

```bash
ls -t ~/.claude/projects/$(echo "$PWD" | sed 's|/|-|g')/*.jsonl 2>/dev/null | head -1 | xargs basename | sed 's/\.jsonl$//'
```

Store this as `SESSION_ID` for use in metadata.json and document footers.

**Create metadata.json:**

```json
{
  "slug": "<slug>",
  "created_at": "<ISO timestamp>",
  "phase": "brainstorm",
  "brainstorm_complete": true,
  "sessions": [
    {
      "phase": "brainstorm",
      "session_id": "<SESSION_ID>",
      "timestamp": "<ISO timestamp>"
    }
  ]
}
```

## Phase 6: Done

Display completion with next steps:

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 DESIGN ► PROJECT INITIALIZED
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

**[Project Name]**

| Artifact       | Location                              |
|----------------|---------------------------------------|
| Project        | `$DESIGN_DIR/<slug>/PROJECT.md`       |
| Context        | `$DESIGN_DIR/<slug>/CONTEXT.md`       |
| Research       | `$DESIGN_DIR/<slug>/RESEARCH.md`      |

**Requirements:** [X] must have | [Y] should have | [Z] won't have

───────────────────────────────────────────────────────────────

## Next Up

/design:design — create technical design for implementation

───────────────────────────────────────────────────────────────
```

</process>

<output>

- `$DESIGN_DIR/<slug>/PROJECT.md` — project definition with requirements
- `$DESIGN_DIR/<slug>/CONTEXT.md` — codebase analysis (if brownfield)
- `$DESIGN_DIR/<slug>/RESEARCH.md` — domain research (if selected)
- `$DESIGN_DIR/<slug>/metadata.json` — project state

</output>

<success_criteria>

- [ ] $DESIGN_DIR/<slug>/ directory created
- [ ] Brownfield detection completed
- [ ] Codebase context gathered (if existing code)
- [ ] Domain research completed (if selected)
- [ ] Deep questioning completed (threads followed, not rushed)
- [ ] Requirements scoped into Must/Should/Won't
- [ ] PROJECT.md captures full context
- [ ] metadata.json created with session ID
- [ ] Claude session ID recorded in metadata.json and document footer
- [ ] User knows next step is `/design:design`

</success_criteria>
</output>
</output>
