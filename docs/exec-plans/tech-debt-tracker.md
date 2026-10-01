# Technical debt

<!-- owner: maintainers; reviewed: 2026-09-28 -->

These are observed compromises, not shipped features or active assignments. Take
one focused item when touching its boundary; use an [execution plan](../plans.md)
when the change spans several modules. Maintainers own prioritization.

| ID | Evidence and impact | Completion condition |
| --- | --- | --- |
| D1 | [app](../../src/app.rs) and [navigation](../../src/app/navigation.rs) call [UI](../../src/ui.rs) wrapping/detail helpers; UI also reads app state. State transitions depend on presentation geometry. | Move shared presentation calculations behind a pure module or explicit messages; remove the app-to-UI edge; preserve long-text navigation and PTY checks. |
| D2 | [store](../../src/store.rs) owns incident/capture records imported by pure reports, state, and UI. The guard permits named records but rejects the Store service. | Move shared records into a driver-independent model module; remove pure-to-store edges without changing snapshot/export compatibility. |
| D3 | [compare](../../src/compare.rs), [metrics](../../src/metrics.rs), and [diagnostics](../../src/diagnostics.rs) form a dependency cycle through identity guards and analysis. | Extract shared identity/baseline calculations and establish one-way analysis dependencies; retain reset, incomplete-source, and zero-denominator regressions. |
| D4 | [Architecture tests](../../tests/architecture.rs) inspect source syntax, not resolved calls or expanded macro bodies. | Extend checks when a real bypass occurs; keep negative regression fixtures and behavioral review. Do not claim static purity or an I/O sandbox. |

Resolved items should retain a short resolution and the relevant validation or
commit link. Product expansion belongs in the [roadmap](../roadmap.md).
