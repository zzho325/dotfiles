# [Project Name] Implementation Plan

## Summary

| Workstream | Milestones | Tasks |
|------------|------------|-------|
| Workstream 1: [Name] | [count] | [count] |
| Workstream 2: [Name] | [count] | [count] |
| **Total** | [count] | [count] |

## Critical Path

The critical path determines the minimum time to completion:

```
task-001 → task-003 → task-005 → task-008 → task-010
```

Tasks on the critical path should be prioritized - delays here delay everything.

## Dependency Graph

```
Milestone 1.1 ──► Milestone 1.2 ──► Milestone 1.3
              │
Milestone 2.1 ─────────┴──► Milestone 2.2
```

---

## Workstream 1: [Workstream Name]

### Milestones Overview

| Milestone | Goal | Tasks | Depends on |
|-----------|------|-------|------------|
| Milestone 1.1: [Name] | [One-line goal] | [count] | - |
| Milestone 1.2: [Name] | [One-line goal] | [count] | Milestone 1.1 |

---

### Milestone 1.1: [Milestone Name]

**Goal:** [What's shippable after this milestone]

**Depends on:** [Nothing / Milestone X / Workstream Y complete]

**Exit Criteria:**
- [ ] [Criterion 1 - what can be demoed]
- [ ] [Criterion 2]

#### Tasks

| # | Task | Type | Mode | Depends on | Critical Path |
|---|------|------|------|------------|---------------|
| 001 | [Name] | Build | Code | - | Yes |
| 002 | [Name] | Build | Code | - | No |
| 003 | [Name] | Build | Code | 001 | Yes |

*Mode: Code = Claude/agent can implement, Human = requires human action*

#### Task Details

##### Task 001: [Task Name]

**Type:** Build
**Mode:** Code
**Depends on:** None
**Enables:** Task 003

**What:** [Clear description - what capability does this add?]
[After this task, we can: ...]

**Context:** [Background from PROJECT.md - why this matters]

**Requirements Covered:**
- REQ-01: [description]

**Files to Modify:**
- `path/to/file.go` — [what changes]

**Acceptance Criteria:**
- [ ] [Testable criterion - what can be demonstrated]
- [ ] [Tests pass - specific tests for this capability]

**Approach:**
[How to implement - patterns to follow from CONTEXT.md]
[Key decisions from DESIGN.md]

---

##### Task 002: [Task Name]

**Type:** Manual
**Mode:** Human
**Depends on:** None
**Enables:** Task 004

**What:** [What the human needs to do]

**Instructions:**
1. [Step 1]
2. [Step 2]
3. [Step 3]

**Verification:**
- [ ] [How to verify this was done correctly]

---

### Milestone 1.2: [Next Milestone]

...

---

## Workstream 2: [Workstream Name]

...

---

## Spikes

| ID | Question | Depends on | Blocks |
|----|----------|------------|--------|
| Spike 1 | [What to investigate] | - | Task 005 |

---

## Traceability

| REQ-ID | Requirement | Covered By |
|--------|-------------|------------|
| REQ-01 | [desc] | Task 001, Task 003 |
| REQ-02 | [desc] | Task 002 |

## Open Questions

- [ ] [Question needing resolution before execution]

---
*Last updated: [date]*
