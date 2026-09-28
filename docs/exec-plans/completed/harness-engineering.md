# Repository harness

<!-- owner: maintainers; reviewed: 2026-09-28 -->

## Outcome and acceptance

Make repository knowledge discoverable and enforce key boundaries using the local
check command and CI. Adapt the practices in OpenAI's
[harness engineering article](https://openai.com/index/harness-engineering/) to a
single Rust TUI package. Acceptance: indexed documentation, versioned plans and
quality/debt evidence, regression-tested documentation and architecture checks,
weekly maintenance validation, and successful Rust and synthetic PTY checks.

## Scope and constraints

Development harness, documentation, and CI. Preserve read-only PostgreSQL behavior,
private local evidence, one package, and database-free default tests. Keep existing
PostgreSQL and terminal checks. No runtime feature or deployment is required.

## Steps

- [x] Inspect current code, documentation, CI, and the referenced article.
- [x] Add architecture guardrails and negative regression fixtures.
- [x] Index knowledge and record quality gaps and architectural debt.
- [x] Wire documentation checks into local and CI workflows, including weekly review.
- [x] Run validation, review the diff, and archive this plan with exact results.

## Decisions

- Retain the existing architecture and contribution guides as their source of truth;
  additional files cover planning, evidence, and maintenance only.
- Parse Rust with Syn in development tests, so comments and strings do not trigger
  architecture violations. Reuse the locked Syn 2 dependency; add no runtime dependency.
- Preserve existing app/UI and comparison/metrics coupling as explicit debt. Guard
  present boundaries before attempting separate behavioral refactors.
- Reuse the synthetic PTY workflow for reproducible application feedback. Its temp
  stores and local fake server avoid operational databases and shared user state.
- Keep the current CI gates. The article's permissive merge policy is contextual;
  PostgreSQL safety and evidence compatibility still require regression checks.

## Validation

Validated locally on macOS with pinned Rust 1.98.1:

- `just check`: passed documentation checks, rustfmt, Clippy with warnings denied,
  144 Rust tests (130 library, 3 architecture, 11 CLI), build, and the synthetic PTY
  workflow. Four live PostgreSQL tests were deliberately ignored by the default run.
- Documentation checker: six regression tests cover broken paths/headings,
  repository escapes, orphan files, stale/invalid/future review dates, completed
  plan history, and an oversized instruction map.
- PTY: passed at 120×36 and 80×24, including all ten views, offline comparison,
  incident exports, resize/restoration, and Ctrl+C during stalled collection and
  blocked SQLite recording.
- Final guard refinement: `cargo clippy --locked --all-targets -- -D warnings` and
  `cargo test --locked --test architecture` passed after restricting app event
  imports to value types, with negative coverage for terminal reads.
- `actionlint .github/workflows/ci.yml` and `git diff --check`: passed.
- No live PostgreSQL fixture or remote CI run was performed for this harness-only
  change. No runtime code or database provisioning changed. The weekly job becomes
  active only when this workflow reaches the default branch.

## Follow-up

Track remaining structural work in [technical debt](../tech-debt-tracker.md).
