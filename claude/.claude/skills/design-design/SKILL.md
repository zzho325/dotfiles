---
name: design:design
description: Create technical design from PROJECT.md with architecture, requirement mapping, and implementation approach
allowed-tools:
  - Read
  - Bash
  - Write
  - Glob
  - Task
  - AskUserQuestion
---

<objective>

Create technical design from brainstorming output. Takes PROJECT.md and produces DESIGN.md with architecture decisions, requirement-to-component mapping, and implementation approach.

**Requires:**
- `$DESIGN_DIR/<slug>/PROJECT.md` — from `/design:brainstorm`

**Creates:**
- `$DESIGN_DIR/<slug>/DESIGN.md` — technical design document
- `$DESIGN_DIR/<slug>/DECISIONS.md` — structured decision log
- `$DESIGN_DIR/<slug>/TECHNICAL-RESEARCH.md` — implementation research (optional)

**After this command:** Run `/design:plan` to break into tasks.

</objective>

<execution_context>

@.claude/design/references/questioning.md
@.claude/design/templates/design.md
@.claude/design/templates/decisions.md

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
   - question: "Which project do you want to design?"
   - options: [one option per project found]

3. **Verify PROJECT.md exists:**
   ```bash
   cat $DESIGN_DIR/<slug>/PROJECT.md
   ```

   If PROJECT.md doesn't exist:
   ```
   PROJECT.md not found for this project. Run `/design:brainstorm` first.
   ```
   Exit command.

4. **Load existing context:**
   - Read PROJECT.md
   - Read CONTEXT.md (if exists)
   - Read RESEARCH.md (if exists)

5. **Display requirements summary:**

   ```
   ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    DESIGN ► DESIGN
   ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

   **Project:** [name from PROJECT.md]

   **Requirements:**
   - Must Have: [count]
   - Should Have: [count]
   - Won't Have: [count]

   **Context available:**
   - CONTEXT.md: [yes/no]
   - RESEARCH.md: [yes/no]
   ```

## Phase 2: Pattern Detection (Mandatory)

**Before designing anything, detect existing patterns in the target packages.**

Based on the requirements in PROJECT.md, identify which packages/files will be modified or extended. Then spawn an exploration agent to detect existing patterns:

```
Task(
  prompt="Analyze existing code patterns in the packages that will be affected by this project.

  Project requirements:
  [Summary from PROJECT.md - especially which packages/files are mentioned]

  For each target package, investigate and report:

  1. **Function style**: Are functions standalone package-level functions or methods on structs?
     - Look for existing functions that do similar work to what the project needs
     - Note the receiver type if methods are used, or note 'standalone' if package-level
     - Example: 'ActivateCardToken() is a standalone function in pkg/cards/card_token_store.go'

  2. **Naming conventions**: How are similar functions/types named?
     - Prefix patterns (e.g., Create*, Get*, Update*, Transition*)
     - Parameter ordering conventions
     - Return value patterns (single error, value+error, etc.)

  3. **File organization**: Where do similar functions live?
     - Are they grouped by entity, by operation type, or by layer?

  4. **Error handling patterns**: How are errors wrapped and returned?

  5. **Testing patterns**: How are similar functions tested?
     - Test file naming, helper functions, table-driven vs individual tests

  Return findings in this format:

  ## Detected Patterns

  ### [Package/File Path]

  **Function Style:** [standalone functions | methods on StructName | mixed]
  **Evidence:** [list 2-3 existing functions with their signatures]

  **Naming Convention:** [describe pattern]
  **Evidence:** [list examples]

  **File Organization:** [describe where things live]

  **Error Handling:** [describe pattern]

  **Testing:** [describe pattern]

  ### Summary: Patterns to Follow

  - New functions in [package] MUST be [standalone/methods on X] — follow existing pattern in [file]
  - Naming should follow [pattern] — e.g., [example]
  - Tests should follow [pattern] — see [file] for reference
  ",
  subagent_type="explore",
  description="Detect codebase patterns"
)
```

Save the detected patterns to `$DESIGN_DIR/<slug>/PATTERNS.md` and display a summary:

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 DESIGN ► PATTERNS DETECTED
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

[For each target package:]
- **[package]**: [standalone functions | methods on X] — see [file]
- **Naming**: [pattern with examples]
- **Tests**: [pattern]
```

**CRITICAL RULE:** All subsequent design phases MUST respect detected patterns. If existing functions in a package are standalone, new functions MUST be standalone. If they are methods on a struct, new ones MUST be methods on the same struct. Never propose a different style than what already exists unless the user explicitly requests it.

## Phase 3: Technical Research (Optional)

Use AskUserQuestion:
- header: "Technical Research"
- question: "Any specific technical questions to research before designing?"
- options:
  - "Yes, I have questions" — Research implementation specifics first
  - "No, let's design" — I know enough, proceed to architecture

**If "Yes, I have questions":**

Display:
```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 DESIGN ► TECHNICAL RESEARCH
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

Ask inline: "What technical questions do you need answered? (Enter each question, or 'done' when finished)"

Collect questions until user says "done".

For each question, spawn research agent:

```
Task(
  prompt="Research this technical question in the context of the codebase:

  Question: [user's question]

  Codebase context:
  [Summary from CONTEXT.md if exists]

  Project requirements:
  [Summary from PROJECT.md]

  Investigate:
  1. How this is typically done
  2. How this codebase handles similar things
  3. Recommended approach for this project
  4. Potential pitfalls

  Return findings in this format:

  ## [Question]

  ### Current Codebase Approach
  [How the codebase handles this or similar things]

  ### Recommended Approach
  [What to do for this project]

  ### Considerations
  - [Thing to keep in mind]

  ### Example
  [Code snippet or pattern if helpful]
  ",
  subagent_type="explore",
  description="Research: [abbreviated question]"
)
```

Combine all research findings into `$DESIGN_DIR/<slug>/TECHNICAL-RESEARCH.md`:

```markdown
# Technical Research

[Date]

## Questions Researched

1. [Question 1]
2. [Question 2]

---

[Research findings for each question]
```

Display key findings before proceeding.

**If "No, let's design":** Continue to Phase 4.

## Phase 4: Architecture Discussion

Display:
```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 DESIGN ► ARCHITECTURE
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

**Analyze requirements and context:**

Based on PROJECT.md requirements, CONTEXT.md (if exists), and **detected patterns from Phase 2**, identify:
- Key architectural decisions needed
- **Patterns already detected in target packages that MUST be followed** (from PATTERNS.md)
- New components/modules needed

**Present high-level approach:**

Ask inline: "Based on the requirements and the existing patterns I detected, here's what I'm thinking for the architecture: [describe approach, explicitly noting which existing patterns will be followed and in which files]. Does this align with your thinking, or do you have a different direction in mind?"

**Key decisions via AskUserQuestion:**

For each major technical decision, use AskUserQuestion:

Example decisions (adapt based on project):
- Data storage approach (if new data needed)
- API style (if new APIs needed)
- Authentication approach (if auth needed)
- Caching strategy (if performance sensitive)

Format:
- header: "[Decision Area]"
- question: "[What needs to be decided]"
- options:
  - "[Option 1]" — [tradeoffs]
  - "[Option 2]" — [tradeoffs]
  - "[Option 3]" — [tradeoffs]

Track each decision with rationale for DESIGN.md and DECISIONS.md.

**Maintain DECISIONS.md:**

After each AskUserQuestion decision, append a row to `$DESIGN_DIR/<slug>/DECISIONS.md` using the template from `templates/decisions.md`. Assign sequential IDs (D-1, D-2, ...). Record all options that were presented, the chosen option, and the rationale. Set status to "Active" and date to today.

If a decision is later revised during the design process (e.g., user changes their mind), mark the previous decision as "Superseded by D-[new ID]" in the Status column and add the new decision with a fresh ID.

### Handling Decision Reversals

During iterative design, the user may reverse or change a previous decision. When this happens, follow this protocol:

1. **Identify affected sections.** Before making any edits, scan the existing DESIGN.md (if it exists from a prior iteration) and list every section that references the old decision. This includes:
   - The Technical Decisions table entry
   - Data model definitions that depend on the decision
   - API/interface specifications derived from the decision
   - Component descriptions that reference the decision
   - Requirement mapping entries affected by the decision
   - Implementation approach (order, risks, integration points) impacted by the change

2. **Present the impact to the user.** Before editing, show:
   ```
   ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    DECISION CHANGED
   ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

   **Previous:** [old decision]
   **New:** [new decision]

   **Sections to update:**
   - [Section 1]: [what changes]
   - [Section 2]: [what changes]
   - ...
   ```

3. **Update all affected sections in one pass.** Do not update only the decision point — update every downstream reference so the document stays internally consistent. Write all changes to DESIGN.md together.

4. **Update DECISIONS.md (if it exists).** If a `$DESIGN_DIR/<slug>/DECISIONS.md` file exists, mark the old decision as superseded:
   - Set the old entry's status to `Superseded by D-[new ID]`
   - Add the new decision as a new entry with its own rationale
   - Preserve the old entry for history — do not delete it

5. **Summarize the changes.** After updating, display:
   ```
   ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    DECISION REVERSAL COMPLETE
   ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

   **Decision:** [brief description]
   **Changed from:** [old choice]
   **Changed to:** [new choice]

   **Sections updated ([count]):**
   - [Section]: [summary of change]
   - [Section]: [summary of change]
   ```

## Phase 5: Component Design

**Break into components:**

Based on architecture decisions, identify components/modules:

For each component, define:
- **Name**: What to call it
- **Purpose**: Single responsibility
- **Interfaces**: What it exposes — **must follow detected patterns from Phase 2** (e.g., if existing functions in the package are standalone, new ones must be standalone too; if methods on a struct, use the same struct)
- **Dependencies**: What it needs from other components
- **Key files**: Where code will live (new or modified)
- **Pattern reference**: "follow existing pattern in [file]" — cite the specific file from PATTERNS.md

**Validate with user:**

Present component breakdown:

```
## Proposed Components

1. **[Component A]**
   - Purpose: [what it does]
   - Files: [where it lives]
   - Pattern: follows [standalone functions | methods on X] per [file] (from detected patterns)

2. **[Component B]**
   - Purpose: [what it does]
   - Dependencies: [Component A]
   - Files: [where it lives]
   - Pattern: follows [standalone functions | methods on X] per [file] (from detected patterns)

Does this breakdown make sense?
```

Use AskUserQuestion if adjustments needed:
- header: "Component Design"
- question: "Any changes to the component breakdown?"
- options:
  - "Looks good" — Proceed with this design
  - "Needs adjustment" — Let me suggest changes

## Phase 5b: API Field Specification

**Skip this phase if the project does not involve new API response types.**

When the design includes new customer-facing API response types (e.g., a new resource like CardToken, or a new response struct), explicitly specify the fields before proceeding. Field selection errors are costly — each wrong field requires updating the OpenAPI spec, generated code, conversion function, and tests.

**Step 1: Identify parent/related resources.**

For each new API response type, identify:
- The **parent resource** it belongs to (e.g., CardToken → Card)
- **Sibling resources** at the same level (e.g., CardTransaction is a sibling of CardToken under Card)

**Step 2: Research parent resource's API fields.**

Use a Task agent to read the parent resource's existing API response type:

```
Task(
  prompt="Read the API response type for [parent resource].
  Look in:
  1. The OpenAPI spec: openapi/[parent].yaml
  2. The generated struct: pkg/[domain]/api/[parent].gen.go
  3. The constructor: pkg/[domain]/api/[parent].go

  List every field exposed in the API response, noting:
  - Field name and Go type
  - Whether it's required or optional (pointer vs value)
  - Any fields that are on the internal model but NOT on the API response (explicitly excluded)

  Also check sibling resources if they exist for the same patterns.",
  subagent_type="explore",
  description="Research [parent resource] API fields"
)
```

**Step 3: Build the field specification table.**

For the new API response type, create an explicit field list:

```
### [New Type] API Response Fields

**Parent resource:** [Parent] (see `openapi/[parent].yaml`)

| Field | Type | Source | Include? | Rationale |
|-------|------|--------|----------|-----------|
| id | id.[Type] | model.[Type]ID | Yes | Primary identifier |
| [parent]_id | id.[Parent] | model.[Parent]ID | Yes | Parent reference |
| [field] | string | model.[Field] | Yes | Matches parent pattern |
| [field] | string | model.[Field] | **No** | Internal-only (not on parent API) |
| created_at | time.Time | model.CreatedAt | Yes | Standard convention |
| updated_at | time.Time | model.UpdatedAt | **No** | Parent doesn't expose; follow convention |

**Fields inherited from parent:** [list fields the parent exposes that the child should also expose, e.g., card_account_id, card_program_id]
**Fields deliberately excluded:** [list internal model fields not exposed, with reason]
```

**Step 4: Apply field selection conventions.**

When deciding which fields to include, follow these rules:

1. **Inherit parent resource IDs.** If the parent resource exposes `card_account_id` and `card_program_id`, the child resource should too — customers need these for filtering and association.
2. **Expose `created_at`, not `updated_at`.** Most Column resources expose only `created_at` in API responses. Only include `updated_at` if the parent resource does (e.g., transfer resources include both).
3. **Internal fields stay internal.** Fields like `platform_id`, network-specific metadata, and internal status tracking are not customer-facing. If a field exists on the internal model but not on the parent's API response, default to excluding it.
4. **When in doubt, check the parent.** The parent resource's API response is the source of truth for which patterns to follow.

**Step 5: Validate with user.**

Present the field specification table and get confirmation before proceeding. This prevents rework during implementation.

Use AskUserQuestion:
- header: "API Field Specification"
- question: "Here are the proposed fields for [New Type]. Any changes?"
- options:
  - "Looks good" — Proceed with these fields
  - "Needs adjustment" — Let me suggest changes

Record the final field list as a decision in DECISIONS.md (e.g., "D-N: [Type] API fields — expose [list], exclude [list]").

## Phase 6: Requirement Mapping

**Map each REQ-ID to components:**

For each requirement from PROJECT.md:
1. Identify which component(s) implement it
2. Identify specific files that will be modified/created
3. Estimate complexity (S/M/L):
   - **S**: Single file, straightforward change
   - **M**: Multiple files or moderate complexity
   - **L**: Cross-cutting, complex logic, or new patterns

**Build traceability matrix:**

```
| REQ-ID | Requirement | Component(s) | Files | Complexity |
|--------|-------------|--------------|-------|------------|
| REQ-01 | [desc] | [component] | [files] | M |
| REQ-02 | [desc] | [component] | [files] | S |
```

**Validate coverage:**

Ensure every Must Have requirement is mapped.
Flag any requirements that are unclear or need more investigation.

## Phase 7: Implementation Approach

**Determine build order:**

Based on component dependencies and requirement priorities:
1. What must be built first (foundations, dependencies)
2. What can be built in parallel
3. What comes last (depends on everything else)

**Identify integration points:**

Where do components connect? What interfaces need to be defined early?

**Note risk areas:**

What could go wrong? What needs extra attention?
- Complex logic
- Performance-sensitive areas
- External dependencies
- Areas with unclear requirements

**Testing strategy hints:**

What testing approach makes sense?
- Unit tests for [components]
- Integration tests for [flows]
- Manual testing for [areas]

## Phase 8: Write DESIGN.md

Synthesize all decisions into `$DESIGN_DIR/<slug>/DESIGN.md` using the template from `templates/design.md`.

**If updating an existing DESIGN.md** (iterative design), verify internal consistency before writing:
- Cross-check every section against current decisions — ensure no stale references remain from reversed decisions
- Verify data model matches current technical decisions
- Verify API/interface specs match current component design
- Verify requirement mapping reflects current architecture

**Include:**
- Overview of technical approach
- **Detected patterns section** — summarize patterns from PATTERNS.md that the design follows, with explicit "follow existing pattern in [file]" references
- Architecture diagram or description
- Component breakdown with purposes, files, **and pattern references**
- Technical decisions with rationale
- Data model (if applicable)
- API/interface definitions (if applicable)
- Requirement mapping table
- Implementation approach (order, integration points, risks)

**Finalize DECISIONS.md:**

Ensure `$DESIGN_DIR/<slug>/DECISIONS.md` contains all decisions made during the design process. If DECISIONS.md wasn't created yet (e.g., no AskUserQuestion decisions were made), create it now from `templates/decisions.md` with any implicit decisions captured from the design process. Verify every decision in the DESIGN.md "Technical Decisions" table has a corresponding entry in DECISIONS.md.

**Record the Claude session ID:** Get the current session ID by reading the most recently modified `.jsonl` file in `~/.claude/projects/` for the current project directory. The filename (without `.jsonl` extension) is the session UUID.

```bash
ls -t ~/.claude/projects/$(echo "$PWD" | sed 's|/|-|g')/*.jsonl 2>/dev/null | head -1 | xargs basename | sed 's/\.jsonl$//'
```

Store this as `SESSION_ID` for use in metadata.json and the DESIGN.md footer.

**Add footer to DESIGN.md:**

```markdown
---
*Last updated: [date] after design*
*Claude session: [SESSION_ID]*
```

**Update metadata.json:**

Read the existing metadata.json first, then update it. Append a new entry to the `sessions` array (create the array if it doesn't exist from an older brainstorm run).

```json
{
  "slug": "<slug>",
  "created_at": "<existing>",
  "phase": "design",
  "brainstorm_complete": true,
  "design_complete": true,
  "sessions": [
    {"phase": "brainstorm", "session_id": "<existing>", "timestamp": "<existing>"},
    {"phase": "design", "session_id": "<SESSION_ID>", "timestamp": "<ISO timestamp>"}
  ]
}
```

## Phase 9: Done

Display completion with next steps:

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 DESIGN ► DESIGN COMPLETE
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

**[Project Name]**

| Artifact            | Location                                    |
|---------------------|---------------------------------------------|
| Design              | `$DESIGN_DIR/<slug>/DESIGN.md`              |
| Decisions           | `$DESIGN_DIR/<slug>/DECISIONS.md`           |
| Detected Patterns   | `$DESIGN_DIR/<slug>/PATTERNS.md`            |
| Technical Research  | `$DESIGN_DIR/<slug>/TECHNICAL-RESEARCH.md`  |

**Components:** [count]
**Requirements mapped:** [count] ([X] Must Have, [Y] Should Have)
**Complexity breakdown:** [X]S / [Y]M / [Z]L

───────────────────────────────────────────────────────────────

## Next Up

/design:plan — break design into implementation tasks

───────────────────────────────────────────────────────────────
```

</process>

<output>

- `$DESIGN_DIR/<slug>/DESIGN.md` — technical design document
- `$DESIGN_DIR/<slug>/DECISIONS.md` — structured decision log
- `$DESIGN_DIR/<slug>/PATTERNS.md` — detected codebase patterns
- `$DESIGN_DIR/<slug>/TECHNICAL-RESEARCH.md` — implementation research (if selected)
- Updated `metadata.json` with design_complete flag

</output>

<success_criteria>

- [ ] PROJECT.md exists and was loaded
- [ ] Context files loaded (CONTEXT.md, RESEARCH.md if exist)
- [ ] **Existing codebase patterns detected and saved to PATTERNS.md**
- [ ] **Design follows detected patterns (e.g., standalone functions vs struct methods match existing code)**
- [ ] Technical research completed (if selected)
- [ ] Architecture decisions made with rationale
- [ ] Components defined with purposes, file locations, **and pattern references**
- [ ] API field specifications completed for new API response types (if applicable): parent fields researched, field list approved, decision recorded
- [ ] All Must Have requirements mapped to components
- [ ] Complexity estimates assigned (S/M/L)
- [ ] Implementation order determined
- [ ] Risk areas identified
- [ ] DESIGN.md written with full content **including detected patterns section**
- [ ] DECISIONS.md written with all decisions tracked (ID, options, choice, rationale, date, status)
- [ ] Superseded decisions marked with reference to replacement
- [ ] Claude session ID recorded in metadata.json and DESIGN.md footer
- [ ] metadata.json updated
- [ ] User knows next step is `/design:plan`

</success_criteria>
</output>
</output>
