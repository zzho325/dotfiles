# Questioning Philosophy

This document guides how to conduct deep questioning during the brainstorming phase.

## How to Question

- **Start open** — "What do you want to build?" — let them dump their mental model
- **Follow energy** — dig into what excites them, what problem sparked this
- **Challenge vagueness** — "fast" means what? "users" means who? "simple" how?
- **Make abstract concrete** — "Walk me through using this"
- **Use options as interpretations** — options should reflect what they said, not predefined categories

## Propose-and-Correct Over Ask-and-Wait

**Default stance: propose complete answers, let the user correct.**

When you can infer an answer from context, don't ask — propose it with your rationale:

- BAD: "Should the scope be design only, or design + implement?"
- GOOD: "I'm assuming design + implement since you described concrete behavior. Let me know if you want design only."

- BAD: "At what point should the token be created?"
- GOOD: "Based on your description, tokens should be created at the Check Eligibility step — that's when you first mentioned token state. Does that match your intent?"

- BAD: "What priority are these requirements?"
- GOOD: Present requirements with proposed priorities inline: "Here's how I'd categorize these — correct anything that's wrong."

This approach:
- Respects the user's time (fewer round trips)
- Shows you're actually processing what they said
- Makes it easy to correct (changing a proposed answer is faster than answering from scratch)
- Surfaces your reasoning so the user can catch misunderstandings

## Batch Related Questions

**Combine related questions into a single interaction instead of asking one at a time.**

When several questions are independent, present them together in one turn. Use inline text to list the questions with numbered items, then let the user respond to all at once. This reduces back-and-forth without sacrificing depth.

- BAD: Ask about scope → wait → ask about priority → wait → ask about timeline
- GOOD: Present all three in one message:
  ```
  A few things to clarify before we go deeper:
  1. **Timeline** — is there a deadline, or is this open-ended?
  2. **Dependencies** — does this block or depend on other work?
  3. **Users** — is this internal-only or customer-facing?
  ```

For structured choices, use a single AskUserQuestion where the `question` field contains numbered sub-questions and the `options` combine the key choices:

- header: "Project Setup"
- question: "1. Should this support both REST and gRPC?\n2. Do we need backwards compatibility?"
- options combine the most likely answer sets (e.g., "REST only, no backwards compat", "Both, with backwards compat")

Only ask questions one at a time when:
- The answer to question 1 determines what question 2 should be
- You're following a deep thread and need to understand one thing before asking the next
- The question is freeform and requires a thoughtful written response

## Anti-patterns to Avoid

- Walking through a checklist regardless of what they said
- Generic questions ("What are your success criteria?")
- Rushing to structure before understanding
- Accepting vague answers without probing
- Asking about things they already explained
- Asking one question at a time when several are independent
- Asking questions whose answers are already implied by context

## Goals (Weave Naturally)

By the end of brainstorming, you should understand these areas. Don't interrogate — weave questions naturally as threads emerge:

1. **Problem** — What problem are we solving? Who experiences this pain?
2. **Goal** — What will be true when this is done? What does success look like?
3. **Scope** — What's in? What's out? Must/Should/Could/Won't
4. **Technical** — Any performance, security, or compliance requirements?
5. **Constraints** — Timeline, dependencies on other work
6. **Assumptions** — What are we assuming to be true?
7. **Risks** — What could go wrong? What unknowns need investigation?

## Decision Gates

Use `AskUserQuestion` when you need structured input:
- Binary choices (Research first? Yes/No)
- Clarifications with concrete options
- Adjustments after a propose-and-correct round (e.g., user wants to re-prioritize specific items)

Use inline propose-and-correct for:
- Scoping decisions (propose Must/Should/Won't categorization, let user correct)
- Questions where the user's description already implies the answer

Use inline freeform questions for:
- Opening the conversation
- Following up on something they said
- Asking for examples or walkthroughs
