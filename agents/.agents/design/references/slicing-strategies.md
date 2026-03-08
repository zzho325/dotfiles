# Task Decomposition & Slicing Strategies

This document defines strategies for breaking down work into well-scoped tasks during the planning phase.

## Core Principle

The way you slice tasks determines how safely and incrementally you can ship. The wrong decomposition leads to large, risky PRs that bundle unrelated concerns. The right decomposition produces small, independently deployable changes.

## Default Rules

These rules apply unless the user explicitly overrides them:

1. **Schema migrations are always separate tasks.** Database migrations must be independently deployable before any code that uses the new schema. Never bundle a migration with the code that depends on it.
2. **One API endpoint per task.** When multiple endpoints are affected, default to one task per endpoint (e.g., separate tasks for activate, deactivate, renew, replace) unless the change is trivially mechanical across all of them.
3. **Prefer smaller tasks over fewer tasks.** A plan with 15 focused tasks is better than 5 sprawling ones. Each task should be reviewable in a single PR.

## Slicing Strategies

### Schema-First

**When to use:** The project involves database changes alongside code changes.

**How it works:**
- All schema migrations are extracted into their own tasks at the front of the dependency chain
- Code tasks depend on their migration tasks but never contain migrations themselves
- Migrations are deployed and verified before dependent code ships

**Example:**
```
Task 1: Add deactivation_reason column (migration only)
Task 2: Add card_status_deactivated column (migration only)
Task 3: Implement deactivate endpoint (code, depends on Task 1 & 2)
Task 4: Implement activate endpoint (code, depends on Task 1 & 2)
```

**Why:** Migrations are high-risk, irreversible changes. Shipping them separately means rollback is simpler and problems surface earlier.

**Backfill caveat:** Only plan backfill tasks for tables that have existing production data. If the table was created in a recent project that hasn't shipped yet (no records in production), no backfill is needed — just write the migration for the new schema. Always verify production data status before assuming a backfill is required.

### Verb/Endpoint Slicing

**When to use:** The project adds or modifies multiple API endpoints or operations.

**How it works:**
- Each API verb or endpoint gets its own task
- Each task includes the full vertical: handler, service logic, store methods, tests
- Tasks can often be worked in parallel once shared dependencies (models, migrations) are done

**Example:**
```
Task 1: Implement POST /cards/{id}/deactivate
Task 2: Implement POST /cards/{id}/activate
Task 3: Implement POST /cards/{id}/renew
Task 4: Implement POST /cards/{id}/replace
```

**Why:** Each endpoint is independently shippable and testable. Reviewing one endpoint at a time is far easier than reviewing all of them together.

### Field/Concern Slicing

**When to use:** A single field or concern needs to be traced across multiple surfaces (API request/response, database, internal logic).

**How it works:**
- One task per field or concern, implemented end-to-end across all layers
- Each task adds the field everywhere it needs to appear: model, store, handler, API response, tests

**Example:**
```
Task 1: Add deactivation_reason to all API surfaces (model, store, handler, response)
Task 2: Add renewal_count to all API surfaces (model, store, handler, response)
```

**Why:** Keeps related changes together. When a field is simple and mechanical, it's cleaner to add it everywhere in one pass rather than touching the same files repeatedly across endpoint tasks.

### Layer Slicing

**When to use:** The project involves building a new feature from scratch where each layer can be built and verified independently.

**How it works:**
- Tasks are organized by architectural layer: model → store → service/handler → API surface
- Each layer task builds on the previous one
- Useful when layers are substantial enough to warrant separate review

**Example:**
```
Task 1: Add card lifecycle models and enums
Task 2: Add card lifecycle store methods
Task 3: Add card lifecycle handler logic
Task 4: Add card lifecycle API endpoints
```

**Why:** Each layer can be reviewed by the person who knows it best. Useful for complex features where each layer has significant logic.

## Choosing a Strategy

Most projects use a **combination** of strategies. The decision tree:

1. **Are there database changes?** → Extract migrations first (Schema-First). This is non-negotiable by default.
2. **Are multiple API endpoints affected?**
   - If changes per endpoint are substantial → Verb/Endpoint Slicing
   - If changes per endpoint are trivially mechanical → Field/Concern Slicing
3. **Is this a new feature built from scratch?** → Consider Layer Slicing for the initial build, then Verb/Endpoint Slicing for the API surface.
4. **Is this a cross-cutting concern (e.g., add a field everywhere)?** → Field/Concern Slicing.

## Heuristics

These heuristics apply across all slicing strategies:

### Co-locate utility functions with their first consumer

**Never create "write all the helpers" tasks.** When a plan involves shared utility functions (state machine transitions, validation helpers, conversion functions, etc.), each function should be written in the task that first needs it — not bundled into a separate "core utilities" or "shared helpers" task.

**Why:**
- A task that writes 10 helper functions with no caller is impossible to meaningfully test or review
- The implementer lacks context for the function's API because the consumer doesn't exist yet
- It creates a massive, monolithic task that blocks everything downstream

**Instead:**
- Each consumer task writes the utility function(s) it needs
- Later tasks reuse functions written by earlier tasks (document the dependency)
- The first consumer shapes the function's API based on real usage

**Example (bad):**
```
Task 3: Implement all 10 state transition functions (blocks Tasks 4-8)
Task 4: Refactor activate handler (depends on Task 3)
Task 5: Refactor deactivate handler (depends on Task 3)
```

**Example (good):**
```
Task 3: Refactor activate handler + write activate/suspend transitions
Task 4: Refactor deactivate handler + write deactivate transition (reuses activate from Task 3)
Task 5: Refactor renew handler + write renew transition
```

## Anti-Patterns

| Anti-Pattern | Why It's Bad | Better Approach |
|---|---|---|
| Bundling migration + code in one task | Can't deploy migration independently; rollback is complex | Schema-First: separate migration task |
| One task for "implement all endpoints" | Massive PR, hard to review, all-or-nothing deploy | Verb/Endpoint Slicing: one task per endpoint |
| Slicing by file instead of capability | Tasks like "edit models.go" don't deliver shippable value | Slice by capability: "users can deactivate cards" |
| Over-slicing mechanical changes | 10 tasks for adding the same field to 10 response structs | Field/Concern Slicing: one task for the field across surfaces |
| "Write all helpers" task | Functions written without consumer context; massive blocking task; untestable in isolation | Co-locate each helper with its first consumer task |
| Planning backfill for empty tables | Backfill task for a table with no production data wastes effort and adds confusion | Verify production data exists before planning backfill; skip for unshipped features |
