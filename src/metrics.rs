//! Conservative interval rates. Each unavailable metric retains its own reason.
use serde::Serialize;

use crate::{
    compare::{StatementIdentity, compare_statements, sources_compatible},
    model::{DatabaseStats, Observation, Snapshot, WalStats},
};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct IntervalMetrics {
    pub(crate) elapsed_seconds: Option<f64>,
    pub(crate) database: Observation<DatabaseRates>,
    pub(crate) wal: Observation<WalRates>,
    pub(crate) statements: Observation<Vec<StatementRate>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct DatabaseRates {
    pub(crate) baseline: String,
    pub(crate) commits_per_second: Observation<f64>,
    pub(crate) rollbacks_per_second: Observation<f64>,
    pub(crate) transactions_per_second: Observation<f64>,
    pub(crate) rollback_percent: Observation<f64>,
    pub(crate) reads_per_second: Observation<f64>,
    pub(crate) hits_per_second: Observation<f64>,
    pub(crate) cache_hit_percent: Observation<f64>,
    pub(crate) temp_bytes_per_second: Observation<f64>,
    pub(crate) deadlocks_per_second: Observation<f64>,
    pub(crate) inserted_per_second: Observation<f64>,
    pub(crate) updated_per_second: Observation<f64>,
    pub(crate) deleted_per_second: Observation<f64>,
    pub(crate) read_ms_per_second: Observation<f64>,
    pub(crate) write_ms_per_second: Observation<f64>,
    pub(crate) deadlocks: Observation<i64>,
    pub(crate) temp_bytes: Observation<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct WalRates {
    pub(crate) bytes_per_second: Observation<f64>,
    pub(crate) records_per_second: Observation<f64>,
    pub(crate) buffers_full_per_second: Observation<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct StatementRate {
    pub(crate) identity: StatementIdentity,
    pub(crate) calls_per_second: Observation<f64>,
    pub(crate) exec_ms_per_second: Observation<f64>,
    pub(crate) mean_exec_ms: Observation<f64>,
    pub(crate) shared_reads_per_second: Observation<f64>,
    pub(crate) temp_blocks_per_second: Observation<f64>,
}

impl IntervalMetrics {
    pub(crate) fn unavailable(reason: &str) -> Self {
        Self {
            elapsed_seconds: None,
            database: Observation::Unavailable(reason.into()),
            wal: Observation::Unavailable(reason.into()),
            statements: Observation::Unavailable(reason.into()),
        }
    }
}

/// Elapsed time is completion-to-completion, only for nonoverlapping captures.
pub(crate) fn interval_seconds(before: &Snapshot, after: &Snapshot) -> Result<f64, String> {
    if !sources_compatible(&before.source, &after.source) {
        return Err("Source identity differs or PostgreSQL restarted; no shared baseline".into());
    }
    if !(1..=crate::model::SNAPSHOT_VERSION).contains(&before.schema_version)
        || !(1..=crate::model::SNAPSHOT_VERSION).contains(&after.schema_version)
    {
        return Err("Snapshot format version is unsupported".into());
    }
    if before.completed_at < before.started_at
        || after.completed_at < after.started_at
        || after.started_at < before.completed_at
    {
        return Err("Capture intervals overlap or have invalid timestamps".into());
    }
    let elapsed = (after.completed_at - before.completed_at).num_milliseconds() as f64 / 1000.0;
    if elapsed <= 0.0 {
        return Err("Captures require positive elapsed time in chronological order".into());
    }
    Ok(elapsed)
}

pub(crate) fn between(before: &Snapshot, after: &Snapshot) -> IntervalMetrics {
    let elapsed = match interval_seconds(before, after) {
        Ok(seconds) => seconds,
        Err(reason) => return IntervalMetrics::unavailable(&reason),
    };
    let database = match (&before.health.database, &after.health.database) {
        (Observation::Available(left), Observation::Available(right)) => {
            match database_baseline_problem(left, right) {
                Some(reason) => Observation::Unavailable(reason),
                None => Observation::Available(database_rates(left, right, elapsed)),
            }
        }
        (left, right) => missing("Database statistics", left, right),
    };
    let wal = match (&before.health.wal, &after.health.wal) {
        (Observation::Available(left), Observation::Available(right)) => {
            match reset_problem(left.stats_reset, right.stats_reset, "WAL") {
                Some(reason) => Observation::Unavailable(reason),
                None => Observation::Available(wal_rates(left, right, elapsed)),
            }
        }
        (left, right) => missing("WAL statistics", left, right),
    };
    let statements = match (&before.statements, &after.statements) {
        (Observation::Available(left), Observation::Available(right)) => {
            let comparison = compare_statements(left, right, Some((elapsed * 1000.0) as i64));
            if comparison.entries.is_empty()
                && let Observation::Unavailable(reason) = comparison.total
            {
                // With no entries there is nowhere else to retain the reset,
                // eviction, or coverage error. Empty observations do not prove
                // a valid interval baseline.
                Observation::Unavailable(reason)
            } else {
                Observation::Available(
                    comparison
                        .entries
                        .into_iter()
                        .map(|entry| {
                            let identity = entry.identity;
                            match entry.delta {
                                Observation::Available(delta) => StatementRate {
                                    identity,
                                    calls_per_second: Observation::Available(
                                        delta.calls as f64 / elapsed,
                                    ),
                                    exec_ms_per_second: finite(
                                        delta.total_exec_ms / elapsed,
                                        "Statement execution rate",
                                    ),
                                    mean_exec_ms: delta.mean_exec_ms.map_or_else(
                                        || {
                                            Observation::Unavailable(
                                                "No completed calls in this interval".into(),
                                            )
                                        },
                                        Observation::Available,
                                    ),
                                    shared_reads_per_second: Observation::Available(
                                        delta.shared_blks_read as f64 / elapsed,
                                    ),
                                    temp_blocks_per_second: Observation::Available(
                                        delta.temp_blks_written as f64 / elapsed,
                                    ),
                                },
                                Observation::Unavailable(reason) => StatementRate {
                                    identity,
                                    calls_per_second: Observation::Unavailable(reason.clone()),
                                    exec_ms_per_second: Observation::Unavailable(reason.clone()),
                                    mean_exec_ms: Observation::Unavailable(reason.clone()),
                                    shared_reads_per_second: Observation::Unavailable(
                                        reason.clone(),
                                    ),
                                    temp_blocks_per_second: Observation::Unavailable(reason),
                                },
                            }
                        })
                        .collect(),
                )
            }
        }
        (left, right) => missing("Statement statistics", left, right),
    };
    IntervalMetrics {
        elapsed_seconds: Some(elapsed),
        database,
        wal,
        statements,
    }
}

pub(crate) fn database_baseline_problem(
    before: &DatabaseStats,
    after: &DatabaseStats,
) -> Option<String> {
    if !before.track_counts || !after.track_counts {
        return Some("track_counts was disabled in at least one capture; database counter coverage is unavailable".into());
    }
    // PostgreSQL reports SQL NULL for an initial, never-reset database entry.
    // A reset writes a timestamp, including a single relation counter reset.
    // See REL_18_STABLE pgstatfuncs.c:pg_stat_get_db_stat_reset_time and
    // pgstat.c:pgstat_reset_counters / pgstat_reset. Source/start checks happen
    // before this function, and each counter must still be non-regressing.
    if before.stats_reset.is_none() && after.stats_reset.is_none() {
        return None;
    }
    reset_problem(before.stats_reset, after.stats_reset, "Database")
}

fn reset_problem(
    before: Option<chrono::DateTime<chrono::Utc>>,
    after: Option<chrono::DateTime<chrono::Utc>>,
    section: &str,
) -> Option<String> {
    match (before, after) {
        (Some(left), Some(right)) if left == right => None,
        (Some(_), Some(_)) => Some(format!("{section} statistics reset between captures")),
        _ => Some(format!(
            "{section} reset metadata unavailable; cannot establish counter continuity"
        )),
    }
}

pub(crate) fn counter_delta(before: i64, after: i64, metric: &str) -> Observation<i64> {
    if before < 0 || after < before {
        Observation::Unavailable(format!(
            "{metric} counter regressed or is invalid; reset or inconsistent baseline is possible"
        ))
    } else {
        Observation::Available(after - before)
    }
}

fn rate(before: i64, after: i64, elapsed: f64, metric: &str) -> Observation<f64> {
    match counter_delta(before, after, metric) {
        Observation::Available(delta) => Observation::Available(delta as f64 / elapsed),
        Observation::Unavailable(reason) => Observation::Unavailable(reason),
    }
}

fn float_rate(before: f64, after: f64, elapsed: f64, metric: &str) -> Observation<f64> {
    if !before.is_finite() || !after.is_finite() || before < 0.0 || after < before {
        Observation::Unavailable(format!("{metric} counter regressed or is invalid"))
    } else {
        finite((after - before) / elapsed, metric)
    }
}

fn finite(value: f64, metric: &str) -> Observation<f64> {
    if value.is_finite() {
        Observation::Available(value)
    } else {
        Observation::Unavailable(format!("{metric} exceeds the supported numeric range"))
    }
}

fn sum(left: &Observation<f64>, right: &Observation<f64>) -> Observation<f64> {
    match (left, right) {
        (Observation::Available(left), Observation::Available(right)) => {
            Observation::Available(left + right)
        }
        (Observation::Unavailable(reason), _) | (_, Observation::Unavailable(reason)) => {
            Observation::Unavailable(reason.clone())
        }
    }
}

fn percent(part: &Observation<f64>, total: &Observation<f64>) -> Observation<f64> {
    match (part, total) {
        (Observation::Available(part), Observation::Available(total)) if *total > 0.0 => {
            Observation::Available(100.0 * part / total)
        }
        (Observation::Unavailable(reason), _) | (_, Observation::Unavailable(reason)) => {
            Observation::Unavailable(reason.clone())
        }
        _ => Observation::Unavailable(
            "No corresponding events in this interval; ratio is undefined".into(),
        ),
    }
}

fn database_rates(before: &DatabaseStats, after: &DatabaseStats, elapsed: f64) -> DatabaseRates {
    let commits_per_second = rate(before.xact_commit, after.xact_commit, elapsed, "Commit");
    let rollbacks_per_second = rate(
        before.xact_rollback,
        after.xact_rollback,
        elapsed,
        "Rollback",
    );
    let transactions_per_second = sum(&commits_per_second, &rollbacks_per_second);
    let rollback_percent = percent(&rollbacks_per_second, &transactions_per_second);
    let reads_per_second = rate(before.blks_read, after.blks_read, elapsed, "Block reads");
    let hits_per_second = rate(before.blks_hit, after.blks_hit, elapsed, "Block hits");
    let cache_hit_percent = percent(&hits_per_second, &sum(&hits_per_second, &reads_per_second));
    let (read_ms_per_second, write_ms_per_second) = if before.track_io_timing
        && after.track_io_timing
    {
        (
            float_rate(
                before.blk_read_time_ms,
                after.blk_read_time_ms,
                elapsed,
                "Read time",
            ),
            float_rate(
                before.blk_write_time_ms,
                after.blk_write_time_ms,
                elapsed,
                "Write time",
            ),
        )
    } else {
        let reason = "track_io_timing was disabled in at least one capture; zero timing does not mean zero I/O cost".to_owned();
        (
            Observation::Unavailable(reason.clone()),
            Observation::Unavailable(reason),
        )
    };
    DatabaseRates {
        baseline: match before.stats_reset {
            Some(time) => format!("Matching observed database reset timestamp: {}", time.to_rfc3339()),
            None => "Both database observations report SQL NULL stats_reset: initial unreset epoch, constrained by the same server instance and non-regressing counters. This is not missing collection metadata.".into(),
        },
        commits_per_second,
        rollbacks_per_second,
        transactions_per_second,
        rollback_percent,
        reads_per_second,
        hits_per_second,
        cache_hit_percent,
        temp_bytes_per_second: rate(
            before.temp_bytes,
            after.temp_bytes,
            elapsed,
            "Temporary bytes",
        ),
        deadlocks_per_second: rate(before.deadlocks, after.deadlocks, elapsed, "Deadlocks"),
        inserted_per_second: rate(
            before.tup_inserted,
            after.tup_inserted,
            elapsed,
            "Inserted tuples",
        ),
        updated_per_second: rate(
            before.tup_updated,
            after.tup_updated,
            elapsed,
            "Updated tuples",
        ),
        deleted_per_second: rate(
            before.tup_deleted,
            after.tup_deleted,
            elapsed,
            "Deleted tuples",
        ),
        read_ms_per_second,
        write_ms_per_second,
        deadlocks: counter_delta(before.deadlocks, after.deadlocks, "Deadlocks"),
        temp_bytes: counter_delta(before.temp_bytes, after.temp_bytes, "Temporary bytes"),
    }
}

fn wal_rates(before: &WalStats, after: &WalStats, elapsed: f64) -> WalRates {
    WalRates {
        bytes_per_second: float_rate(before.bytes, after.bytes, elapsed, "WAL bytes"),
        records_per_second: rate(before.records, after.records, elapsed, "WAL records"),
        buffers_full_per_second: rate(
            before.buffers_full,
            after.buffers_full,
            elapsed,
            "WAL buffers full",
        ),
    }
}

fn missing<T, U>(section: &str, before: &Observation<T>, after: &Observation<T>) -> Observation<U> {
    let mut reasons = Vec::new();
    if let Observation::Unavailable(reason) = before {
        reasons.push(format!("before: {reason}"));
    }
    if let Observation::Unavailable(reason) = after {
        reasons.push(format!("after: {reason}"));
    }
    Observation::Unavailable(format!("{section} unavailable ({})", reasons.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::tests::snapshot;
    use chrono::Duration;

    fn later(before: &Snapshot) -> Snapshot {
        let mut after = before.clone();
        after.started_at += Duration::seconds(10);
        after.completed_at += Duration::seconds(10);
        after
    }

    #[test]
    fn real_zero_rates_do_not_fabricate_ratios_or_execution_mean() {
        let before = snapshot();
        let result = between(&before, &later(&before));
        let db = result.database.available().unwrap();
        assert_eq!(result.elapsed_seconds, Some(10.0));
        assert_eq!(db.transactions_per_second.available(), Some(&0.0));
        assert!(db.rollback_percent.available().is_none());
        assert!(db.cache_hit_percent.available().is_none());
        let statement = &result.statements.available().unwrap()[0];
        assert_eq!(statement.calls_per_second.available(), Some(&0.0));
        assert!(statement.mean_exec_ms.available().is_none());
    }

    #[test]
    fn interval_rates_use_elapsed_time_and_completed_counter_differences() {
        let before = snapshot();
        let mut after = later(&before);
        if let Observation::Available(db) = &mut after.health.database {
            db.xact_commit += 90;
            db.xact_rollback += 10;
            db.blks_hit += 80;
            db.blks_read += 20;
            db.temp_bytes += 1_000;
        }
        if let Observation::Available(stats) = &mut after.statements {
            stats.entries[0].calls += 5;
            stats.entries[0].total_exec_ms += 1000.0;
        }
        let result = between(&before, &after);
        let db = result.database.available().unwrap();
        assert_eq!(db.transactions_per_second.available(), Some(&10.0));
        assert_eq!(db.rollback_percent.available(), Some(&10.0));
        assert_eq!(db.cache_hit_percent.available(), Some(&80.0));
        assert_eq!(db.temp_bytes_per_second.available(), Some(&100.0));
        let stmt = &result.statements.available().unwrap()[0];
        assert_eq!(stmt.mean_exec_ms.available(), Some(&200.0));
        assert_eq!(stmt.exec_ms_per_second.available(), Some(&100.0));
    }

    #[test]
    fn regressed_counter_invalidates_only_dependent_metrics() {
        let before = snapshot();
        let mut after = later(&before);
        if let Observation::Available(db) = &mut after.health.database {
            db.xact_rollback = 0;
            db.blks_read += 5;
        }
        let result = between(&before, &after);
        let db = result.database.available().unwrap();
        assert!(db.rollback_percent.available().is_none());
        assert!(db.transactions_per_second.available().is_none());
        assert_eq!(db.commits_per_second.available(), Some(&0.0));
        assert_eq!(db.reads_per_second.available(), Some(&0.5));
        assert!(result.wal.available().is_some());
        assert!(result.statements.available().is_some());
    }

    #[test]
    fn unknown_reset_or_disabled_counts_blocks_database_rates_only() {
        let before = snapshot();
        for case in 0..3 {
            let mut after = later(&before);
            if let Observation::Available(db) = &mut after.health.database {
                match case {
                    0 => db.stats_reset = None,
                    1 => db.stats_reset = Some(after.started_at),
                    _ => db.track_counts = false,
                }
            }
            let result = between(&before, &after);
            assert!(result.database.available().is_none(), "case {case}");
            assert!(result.wal.available().is_some());
            assert!(
                result.statements.available().unwrap()[0]
                    .calls_per_second
                    .available()
                    .is_some()
            );
        }
    }

    #[test]
    fn unknown_io_timing_and_regressed_wal_bytes_do_not_become_zero() {
        let before = snapshot();
        let mut after = later(&before);
        if let Observation::Available(db) = &mut after.health.database {
            db.track_io_timing = false;
        }
        if let Observation::Available(wal) = &mut after.health.wal {
            wal.bytes = 0.0;
        }
        let result = between(&before, &after);
        assert!(
            result
                .database
                .available()
                .unwrap()
                .read_ms_per_second
                .available()
                .is_none()
        );
        let wal = result.wal.available().unwrap();
        assert!(wal.bytes_per_second.available().is_none());
        assert_eq!(wal.records_per_second.available(), Some(&0.0));
    }

    #[test]
    fn unsafe_time_and_identity_baselines_never_produce_rates() {
        let before = snapshot();
        for case in 0..5 {
            let mut after = later(&before);
            match case {
                0 => after.source.server_started_at += Duration::seconds(1),
                1 => after.source.system_identifier = Some("other".into()),
                2 => after.started_at = before.started_at,
                3 => after.completed_at = before.completed_at,
                _ => after.schema_version = 999,
            }
            let result = between(&before, &after);
            assert_eq!(result.elapsed_seconds, None);
            assert!(result.database.available().is_none());
            assert!(result.wal.available().is_none());
            assert!(result.statements.available().is_none());
        }
    }

    #[test]
    fn legacy_captures_preserve_statement_rates_and_mark_new_sections_unavailable() {
        let mut before = snapshot();
        before.schema_version = 1;
        before.health = Default::default();
        let mut after = later(&before);
        after.schema_version = 2;
        after.health = crate::demo::snapshot(0, false).health;
        let result = between(&before, &after);
        assert!(result.database.available().is_none());
        assert!(result.wal.available().is_none());
        assert_eq!(
            result.statements.available().unwrap()[0]
                .calls_per_second
                .available(),
            Some(&0.0)
        );
    }

    #[test]
    fn statement_reset_and_recreated_entry_keep_explanations_per_entry() {
        let before = snapshot();
        let mut after = later(&before);
        if let Observation::Available(stats) = &mut after.statements {
            stats.entries[0].stats_since = Some(after.started_at);
        }
        let result = between(&before, &after);
        let statement = &result.statements.available().unwrap()[0];
        assert!(
            matches!(&statement.calls_per_second, Observation::Unavailable(reason) if reason.contains("recreated"))
        );
        assert!(result.database.available().is_some());
    }

    #[test]
    fn empty_statement_rankings_preserve_unknown_interval_continuity() {
        let mut before = snapshot();
        if let Observation::Available(stats) = &mut before.statements {
            stats.entries.clear();
        }
        let mut after = later(&before);
        assert_eq!(
            between(&before, &after).statements.available(),
            Some(&vec![])
        );
        if let Observation::Available(stats) = &mut after.statements {
            stats.reset_at = Some(after.started_at);
        }
        let result = between(&before, &after);
        assert!(
            matches!(result.statements, Observation::Unavailable(reason) if reason.contains("reset between captures"))
        );
        assert!(result.database.available().is_some());
    }
}

#[cfg(test)]
mod initial_epoch_tests {
    use super::*;
    use crate::compare::tests::snapshot;
    use chrono::Duration;

    #[test]
    fn two_observed_null_database_resets_are_an_unreset_epoch_not_missing_data() {
        let mut before = snapshot();
        if let Observation::Available(db) = &mut before.health.database {
            db.stats_reset = None;
        }
        let mut after = before.clone();
        after.started_at += Duration::seconds(10);
        after.completed_at += Duration::seconds(10);
        if let Observation::Available(db) = &mut after.health.database {
            db.xact_commit += 30;
        }
        let result = between(&before, &after);
        let database = result.database.available().unwrap();
        assert_eq!(database.transactions_per_second.available(), Some(&3.0));
        assert!(database.baseline.contains("initial unreset epoch"));
        assert!(
            crate::diagnostics::analyze(&after, Some(&before))
                .coverage
                .iter()
                .any(|line| line.contains("initial unreset epoch"))
        );
        if let Observation::Available(db) = &mut after.health.database {
            db.stats_reset = Some(after.started_at);
        }
        assert!(between(&before, &after).database.available().is_none());
    }

    #[test]
    fn unreset_epoch_does_not_relax_source_regression_or_missing_section_guards() {
        let mut before = snapshot();
        if let Observation::Available(db) = &mut before.health.database {
            db.stats_reset = None;
        }
        let mut after = before.clone();
        after.started_at += Duration::seconds(10);
        after.completed_at += Duration::seconds(10);
        if let Observation::Available(db) = &mut after.health.database {
            db.xact_commit = 0;
        }
        assert!(
            between(&before, &after)
                .database
                .available()
                .unwrap()
                .transactions_per_second
                .available()
                .is_none()
        );
        after.source.server_started_at += Duration::seconds(1);
        assert!(between(&before, &after).database.available().is_none());
        after.source = before.source.clone();
        before.health.database =
            Observation::Unavailable("old capture has no database section".into());
        assert!(between(&before, &after).database.available().is_none());
    }

    #[test]
    fn explicit_database_reset_null_is_distinct_from_missing_json_metadata() {
        let capture = snapshot();
        let mut value = serde_json::to_value(capture.health.database.available().unwrap()).unwrap();
        value["stats_reset"] = serde_json::Value::Null;
        let explicit_null: DatabaseStats = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(explicit_null.stats_reset, None);
        value.as_object_mut().unwrap().remove("stats_reset");
        assert!(serde_json::from_value::<DatabaseStats>(value).is_err());
    }

    #[test]
    fn float_rate_overflow_is_unavailable_instead_of_serialized_null() {
        let mut before = snapshot();
        if let Observation::Available(wal) = &mut before.health.wal {
            wal.bytes = 0.0;
        }
        let mut after = before.clone();
        after.started_at = before.completed_at;
        after.completed_at += Duration::milliseconds(1);
        if let Observation::Available(wal) = &mut after.health.wal {
            wal.bytes = f64::MAX;
        }
        let result = between(&before, &after);
        assert!(
            matches!(&result.wal.available().unwrap().bytes_per_second, Observation::Unavailable(reason) if reason.contains("numeric range"))
        );
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("\"data\":null")
        );
    }
}
