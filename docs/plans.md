# Execution plans

<!-- owner: maintainers; reviewed: 2026-09-28 -->

A small, local change can use its PR description as the plan. Before work spanning
multiple boundaries, a migration, or uncertain behavior, add a Markdown plan under
`docs/exec-plans/active/` and link it below. The plan must let a fresh contributor
continue using repository context alone. Update it when evidence changes a decision.
Move it to `completed/` when its acceptance checks are satisfied, update its link,
and record unverified checks separately. Carry unresolved work into the
[debt tracker](exec-plans/tech-debt-tracker.md) or [roadmap](roadmap.md).

## Active

No active plans. Create the active directory when starting the next complex change.

## Completed

- [Repository harness](exec-plans/completed/harness-engineering.md)

## Plan format

Use the ownership/review comment documented in [the knowledge index](index.md).
Keep the following sections concise and specific to the change:

- **Outcome and acceptance:** observable result and checks that demonstrate it.
- **Scope and constraints:** affected modules, compatibility, data/privacy limits.
- **Steps:** ordered work with completion status and dependencies.
- **Decisions:** chosen approach, reason, and evidence; include rejected approaches
  only when they explain a tradeoff.
- **Validation:** exact commands, results, relevant environment, and missing checks.
- **Follow-up:** remaining work with links and a clear completion condition.

A plan records engineering decisions and evidence, not an agent transcript. Do not
store private query text, connection strings, or machine-specific secrets in it.
