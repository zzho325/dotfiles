# [Project Name] Design

## Overview

[High-level technical approach - 2-3 sentences]

## Detected Patterns

Patterns detected in the target packages that this design follows.

| Package | Pattern | Evidence | Reference File |
|---------|---------|----------|----------------|
| [pkg/example] | [standalone functions] | [ExistingFunc() is a package-level function] | [pkg/example/store.go] |

> All new functions/methods in this design follow the patterns above. See `docs/design/<slug>/PATTERNS.md` for full details.

## Architecture

[Diagram or description of how components fit together]

```
[ASCII diagram or mermaid code block]
```

### Components

| Component | Purpose | Key Files |
|-----------|---------|-----------|
| [Name] | [What it does] | [Files affected] |

## Technical Decisions

| Decision | Options Considered | Choice | Rationale |
|----------|-------------------|--------|-----------|
| [Decision] | [Options] | [Choice] | [Why] |

## Data Model

[Key entities and relationships]

```
[Entity diagram or description]
```

## APIs / Interfaces

[Key interfaces between components]

### [Interface Name]

- **Purpose:** [What it does]
- **Input:** [What it takes]
- **Output:** [What it returns]

### API Response Fields

*Include this subsection when the design introduces new customer-facing API response types.*

For each new API response type, document the explicit field list after cross-referencing the parent resource:

#### [New Type] Response

**Parent resource:** [Parent] (`openapi/[parent].yaml`)

| Field | Type | Required | Source | Rationale |
|-------|------|----------|--------|-----------|
| id | id.[Type] | yes | model.[Type]ID | Primary identifier |
| [parent]_id | id.[Parent] | yes | model.[Parent]ID | Parent reference |
| created_at | time.Time | yes | model.CreatedAt | Standard convention |

**Fields NOT exposed:**
- `platform_id` — internal, not customer-facing
- `updated_at` — parent doesn't expose; Column convention is `created_at` only

**Conventions applied:**
- Inherited parent resource IDs: [list, e.g., card_account_id, card_program_id]
- Temporal fields: `created_at` only (matching parent pattern)
- Internal fields excluded: [list]

## Requirement Mapping

| REQ-ID | Requirement | Component(s) | Files | Complexity |
|--------|-------------|--------------|-------|------------|
| REQ-01 | [Description] | [Component] | [Files] | S/M/L |

## Implementation Approach

### Suggested Order

1. [First thing to build] — [why first]
2. [Second thing] — [dependencies]
3. [Third thing] — [dependencies]

### Integration Points

- [Component A] ↔ [Component B]: [How they connect]

### Testing Strategy

- [What to test and how]

### Risk Areas

- [What could go wrong] — [mitigation]

---
*Last updated: [date]*
