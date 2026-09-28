# Repository knowledge

<!-- owner: maintainers; reviewed: 2026-09-28 -->

Start with [AGENTS.md](../AGENTS.md), then open only the guide needed for the task.
Repository files are the durable record of behavior, decisions, and validation.
The owner of these guides is the pgtrail maintainer; contributors update the owning
guide in the same change as its code.

| Need | Source of truth | Review trigger |
| --- | --- | --- |
| Implemented behavior, setup, privacy, commands | [README](../README.md) | User-visible behavior or version changes |
| Modules, data flow, boundaries, technology decisions | [Architecture](architecture.md) | Dependencies, persistence, collection, or runtime changes |
| Delivered scope and future acceptance criteria | [Roadmap](roadmap.md) | Feature scope changes |
| Development, checks, fixture isolation, review | [Contributing](../CONTRIBUTING.md) | Workflow or CI changes |
| Execution plans and decisions | [Plans](plans.md) | Complex work starts, changes direction, or finishes |
| Evidence and validation gaps by domain | [Quality](quality.md) | Relevant checks or coverage change |
| Known architectural compromises | [Technical debt](exec-plans/tech-debt-tracker.md) | A compromise is found, introduced, or resolved |

## Maintenance

Run `just docs-check` after editing knowledge files. It checks local inline links,
Markdown heading fragments, discoverability, ownership, review dates, and the size
of AGENTS.md. Keep local links relative and use inline Markdown links, including
links to source files. External URLs are references and are not fetched by CI.

Review current guides at least every 90 days and whenever their owning behavior
changes. Verify linked code and commands before updating a review date. A date is
an attestation of review, not proof that prose is correct. Completed plans preserve
their historical review date. CI checks these rules on changes and weekly; a
scheduled failure requests maintenance through the normal GitHub Actions results.

Use [quality gaps](quality.md) and the debt tracker to choose a small cleanup,
reproduce the problem, fix it, and record the evidence. Promote repeated review
feedback into a focused test or lint with a repair hint. Keep unverified work
explicit, and keep operational captures and credentials outside the repository.
