# Quality evidence

<!-- owner: maintainers; reviewed: 2026-09-28 -->

This is a coverage map, not a production-health verdict or percentage score.
**Covered** means repeatable checks exist; **partial** identifies a known gap.
Run results for this harness change belong in its [execution plan](plans.md).
Remote CI success must be verified on the actual commit.

| Domain | Assessment | Inspectable checks | Gap or next check |
| --- | --- | --- | --- |
| Observation compatibility | Covered | [model](../src/model.rs), [store](../src/store.rs), [CLI tests](../tests/cli.rs) | Preserve v1 payload and schema migration fixtures with each schema change |
| Counter/identity correctness | Covered | [compare](../src/compare.rs), [metrics](../src/metrics.rs), [diagnostics](../src/diagnostics.rs) unit tests | New metrics need independent reset, missing-data, and denominator cases |
| PostgreSQL safety and versions | Covered by fixture tests | [collector](../src/collector.rs), [privilege smoke check](../scripts/check-db.sh), [CI matrix](../.github/workflows/ci.yml) | Synthetic checks do not verify a live server; run the disposable 16–18 matrix for collector changes |
| Private local evidence and exports | Covered | [profiles](../src/profiles.rs), [store](../src/store.rs), [commands](../src/commands.rs), [CLI tests](../tests/cli.rs) | Logical deletion is not secure erasure; reports can contain operational metadata |
| Terminal lifecycle and navigation | Covered | Rust TestBackend assertions and [PTY workflow](../scripts/check-terminal.py) | Visual inspection remains necessary for layout changes; PTY decoding is deliberately limited |
| Architecture | Partial | [source guardrails](../tests/architecture.rs) | Existing cycles and shared records: [D1–D4](exec-plans/tech-debt-tracker.md) |
| Repository knowledge | Partial | [documentation checker](../scripts/check-docs.py), [negative fixtures](../scripts/test-check-docs.py) | Dates and links cannot establish semantic accuracy; review the owning code |
| Runtime observability and performance | Partial | CLI JSON/Markdown reports, UI freshness/coverage, bounded collection, PTY responsiveness assertions | No general tracing backend or performance baseline; add a reproducible measurement for a concrete performance task |

## Feedback loop

1. Reproduce the issue with the smallest relevant check from
   [Contributing](../CONTRIBUTING.md#validation).
2. Add a failing behavioral regression or a precise structural rule.
3. Fix the owning module and run its tests, then the required repository checks.
4. Update the owning guide and this table if coverage or a gap changed.
5. Review the diff for sensitive artifacts and unrelated scope before reporting
   exact passed and unverified checks.

The existing `--demo`, CLI tests, and PTY script are the default local feedback
surface. Use explicit temporary stores for ad hoc experiments and independent
Compose projects/ports for live fixtures, as documented in Contributing.
