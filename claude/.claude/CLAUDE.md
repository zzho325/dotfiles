# General

- The user has trouble reading long text. Always be concise and prefer visuals
  over prose. Use whatever visual form fits the content — tables, ASCII trees,
  flow/state/sequence diagrams, bullet lists, code/diff blocks. Don't default
  to one form. If you're about to write more than 2-3 sentences of prose to
  explain something, stop and ask whether a diagram or table would convey it
  faster.

# Code Style Preferences

## Line length and breaking

- Keep lines under 100 characters
- When breaking long function calls, break at `(` and `)` — do not break mid-argument. "Mid-argument" includes the common mistake of keeping the first arg on the opening line:

```
// Good
foo(
    a, b, c, d,
)

// Bad — breaks mid-argument
foo(a,
    b, c, d,
)

// Also bad — first arg stays on opening line, rest cascade
foo(a,
    b,
    c,
    d,
)
```

- When a function call has many arguments, break into one parameter per line
- When parameter meaning is unclear, add `/*name*/` comments:

```
// Good
accrueSeasonedInterest(
    historySummary,
    newlySeasonedPR,
    seasonedSummary.EffectiveDate, /*startDate*/
    historySummary.EffectiveDate,  /*asOfDate*/
)
```

## Comments and documentation

- **Default: no comment.** Only add one when the WHY is non-obvious. If a
  test/function name already says it, don't repeat in a comment.
- **Keep comments concise.** One line when possible. No multi-sentence
  explanations for things the code already shows. If a comment is getting long,
  the code probably needs a better name or structure instead.
- **Never reference tickets or PRs in code** (e.g. `// Repro for ENG-29151`,
  `// Added in PR #123`). Belongs in commit message / PR description, rots in
  code as the codebase evolves.
- **No third-arg message on test assertions** (e.g.
  `assert.Equal(t, want, got, "must be stripped")`). The test name + diff
  output is the message. Drop the trailing string.
- Write comments as if the code was always this way — describe the final design, not the journey
- Never reference alternatives, prior bugs, or what "wouldn't work" in comments
- Bad: `// Mon switches to day-2 (unlike business_day which wouldn't until Tue)`
- Good: `// Mon: day-2 reached — NextBD(Fri + 2 = Sun) = Mon`
- Keep comments terse — state what, not implementation details the reader can see
- Bad: `// Insert a manual payout directly — no accruals backing it.`
- Good: `// Create a manual payout.`
- Field comments must start with the field name (Go convention):
  - Good: `// BankAccountID is set when all transfers share one source account.`
  - Bad: `// Set when all transfers share one source account.`
- Comment money amounts, big numbers, and math cases for readability:
  - `initialAmount := 100 * money.MicroPrecision // $100`
  - `Amount: 5000000, // $5.00`
  - `decimal.RequireFromString("0.05") // 5.00%`

## Struct field ordering

- In model structs, group fields by concern with section comments:
  1. Identity fields (ID, platform, type)
  2. Business fields (description, config, mode)
  3. Lifecycle/state fields (status, state machine)
  4. Aggregate/computed fields (counts, totals)
  5. Timestamps (created_at, updated_at, state timestamps)
- Struct first, then types/enums and methods below (not types before struct)

## Error handling

- Prefer typed API errors (`apierrors.Err*` in `pkg/errors/`) over raw errors — avoid 500s where possible
- When wrapping errors from DB/store calls, pick the most specific existing error type
- If no existing error type fits, check if the middleware already maps it (e.g., `gorm.ErrRecordNotFound` → 400)
- Only let raw errors through when they represent genuine infrastructure failures where 500 is appropriate

## Validation patterns

- Check the existing file's convention before choosing between `binding:` tags and manual `Validate()`
- Don't mix patterns in the same file unless there's a good reason (e.g., `id.*` types don't work with `binding:"required"`)
- Use `binding:"required,gt=0"` for primitives; use `Validate()` for custom ID types and business logic

## Function ordering

- Major/exported functions first, helpers below
- Helpers ordered by the order they are called
- Tests ordered to match the order of the functions they test

## Commit messages

- Format: `type(area): description` — where type is `fix`, `feat`, or `refactor`
- For DB migration PRs, use `migration: description` (no area, no parens)
  - Example: `migration: add BA-sort index for chunked processing (check + ACH)`
- Do NOT include Linear ticket numbers (e.g. ENG-12345)
- This overrides any project-level CLAUDE.md guidance on commit format

## PR descriptions

- Lead with **Why** then **What**
- **Why** is concise and engineering-plain. State the technical action or the
  concrete problem. No product/marketing framing, no business justification,
  no "customers need", no "to enable/unlock/improve X", no "Part of X
  initiative", no "needs/requires/so that/in order to".
  - Good: "Add readonly agent users on dashboard-db"
  - Good: "Chunk batch creation into 1000-row units for deploy resilience"
  - Good (multi-sentence is fine when the problem needs it):
    "`GET /investigation-cases/cases` was hitting MySQL Error 1038 — `SELECT *`
    blows the 640 KB `sort_buffer_size`. Fix sorts on PK only, then hydrates via
    a second query (deferred-join)."
  - Bad (producty): "Enable the agent platform to securely query dashboard data
    for better observability across teams"
  - Bad (wordy): "The replica needs readonly users so /db/query can query the
    dashboard database"
- Length follows the problem — a one-liner is great when it fits; a short
  paragraph is fine for perf numbers, EXPLAIN output, or incident context.
  Don't pad to feel thorough.
- Migration PRs: title is `migration: ...`; Why states the schema change plainly
- Format:
  ```
  ## Summary

  **Why:** Plain, engineering-focused — as short as the change allows

  **What:**
  - Bullet list of concrete changes

  **Stack:** N/M — dependency info (if stacked)

  ## Test plan
  - [x] What was tested
  ```

## Test assertions

- For verifying persisted objects, build an `expected` struct and use `assert.Equal(t, expected, actual)` for object-level comparison
- Copy dynamic fields (IDs, timestamps) from actual to expected, then separately assert their validity
