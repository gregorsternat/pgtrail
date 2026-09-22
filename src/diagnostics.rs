//! Evidence-led, read-only investigation hints; findings are not a health verdict.
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::{
    metrics::{self, IntervalMetrics},
    model::{Observation, SYNTHETIC_WARNING, Session, Snapshot},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Severity {
    Critical,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Finding {
    pub(crate) id: String,
    pub(crate) severity: Severity,
    pub(crate) title: String,
    pub(crate) evidence: Vec<String>,
    pub(crate) interpretation: String,
    pub(crate) next_steps: Vec<String>,
    pub(crate) related_pids: Vec<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Analysis {
    pub(crate) findings: Vec<Finding>,
    pub(crate) rates: IntervalMetrics,
    pub(crate) coverage: Vec<String>,
}

pub(crate) fn analyze(snapshot: &Snapshot, previous: Option<&Snapshot>) -> Analysis {
    let rates = previous.map_or_else(
        || IntervalMetrics::unavailable("A previous capture is required for interval metrics"),
        |before| metrics::between(before, snapshot),
    );
    let mut analysis = Analysis {
        findings: Vec::new(),
        rates,
        coverage: Vec::new(),
    };
    availability(&mut analysis.coverage, "Activity", &snapshot.activity);
    availability(&mut analysis.coverage, "Statements", &snapshot.statements);
    availability(
        &mut analysis.coverage,
        "Database",
        &snapshot.health.database,
    );
    availability(
        &mut analysis.coverage,
        "Tables and indexes",
        &snapshot.health.tables,
    );
    availability(
        &mut analysis.coverage,
        "Replication",
        &snapshot.health.replication,
    );
    availability(&mut analysis.coverage, "WAL", &snapshot.health.wal);
    availability(&mut analysis.coverage, "I/O", &snapshot.health.io);
    availability(
        &mut analysis.coverage,
        "Vacuum progress",
        &snapshot.health.vacuum,
    );
    if snapshot.source.system_identifier.is_none()
        || previous.is_some_and(|before| before.source.system_identifier.is_none())
    {
        analysis.coverage.push("Source system identifier unavailable; endpoint, database OID, and server start time provide a weaker identity check".into());
    }
    if let Observation::Available(stats) = &snapshot.statements
        && stats.truncated
    {
        analysis.coverage.push("Statement ranking was truncated; workload coverage and interval baselines are incomplete".into());
    }
    if let Observation::Available(stats) = &snapshot.health.tables
        && stats.truncated
    {
        analysis.coverage.push("Relation ranking was truncated; absent tables or indexes are not proven absent from PostgreSQL".into());
    }
    analysis.coverage.extend(
        snapshot
            .warnings
            .iter()
            .filter(|warning| *warning != SYNTHETIC_WARNING)
            .cloned(),
    );
    if let Some(before) = previous {
        analysis.coverage.extend(
            before
                .warnings
                .iter()
                .filter(|warning| *warning != SYNTHETIC_WARNING)
                .map(|warning| format!("Baseline capture: {warning}")),
        );
    }
    if let Observation::Available(sessions) = &snapshot.activity {
        blocking_findings(sessions, &mut analysis.findings);
        session_findings(sessions, &mut analysis.findings);
        let hidden = sessions
            .iter()
            .filter(|session| session.state.is_none())
            .count();
        if hidden > 0 {
            analysis.coverage.push(format!("{hidden} sessions have unavailable state; transaction and query findings cannot cover all activity"));
        }
    }
    database_findings(snapshot, &mut analysis.findings, &mut analysis.coverage);
    relation_findings(snapshot, &mut analysis.findings);
    replication_findings(snapshot, &mut analysis.findings);
    interval_findings(snapshot, previous, &analysis.rates, &mut analysis.findings);
    interval_coverage(&mut analysis.coverage, &analysis.rates);
    if let Some(database) = analysis.rates.database.available() {
        analysis.coverage.push(database.baseline.clone());
    }
    analysis
        .findings
        .sort_by(|a, b| (a.severity, &a.id).cmp(&(b.severity, &b.id)));
    // Put missing/restricted evidence and interval limitations ahead of the
    // successful-section list, including when the first available section is large.
    analysis
        .coverage
        .sort_by_key(|line| line.ends_with(": collected"));
    analysis.coverage.insert(
        0,
        format!(
            "Provenance: {}",
            if snapshot.is_synthetic() {
                "SYNTHETIC demo; no PostgreSQL connection"
            } else {
                "PostgreSQL observation"
            }
        ),
    );
    analysis.coverage.insert(1, collection_summary(snapshot));
    analysis
        .coverage
        .insert(2, interval_summary(&analysis.rates));
    analysis.coverage.push("No finding is not proof of health. Findings use observed thresholds, not a validated capacity model or causal diagnosis. Captures are non-atomic and statistics may lag activity.".into());
    analysis
}

pub(crate) fn collection_summary(snapshot: &Snapshot) -> String {
    let collected = snapshot.collected_sections();
    format!(
        "Collection: {collected}/8 sections; {}",
        if collected < 8 {
            format!("{} unavailable", 8 - collected)
        } else if snapshot.is_complete() {
            "complete collection (not a health verdict)".into()
        } else {
            "LIMITED; inspect restrictions and warnings".into()
        }
    )
}

pub(crate) fn interval_summary(rates: &IntervalMetrics) -> String {
    let database = match &rates.database {
        Observation::Unavailable(_) => "unavailable",
        Observation::Available(database) => {
            if database_metric_gaps(database).is_empty() {
                "ready"
            } else {
                "partial"
            }
        }
    };
    let wal = match &rates.wal {
        Observation::Unavailable(_) => "unavailable",
        Observation::Available(wal) => {
            if [
                &wal.bytes_per_second,
                &wal.records_per_second,
                &wal.buffers_full_per_second,
            ]
            .iter()
            .all(|metric| metric.available().is_some())
            {
                "ready"
            } else {
                "partial"
            }
        }
    };
    let statements = match &rates.statements {
        Observation::Unavailable(_) => "unavailable",
        Observation::Available(entries) if entries.is_empty() => "no rows",
        Observation::Available(entries) => {
            if entries
                .iter()
                .all(|entry| entry.calls_per_second.available().is_some())
            {
                "ready"
            } else {
                "partial"
            }
        }
    };
    format!("Intervals: DB {database} · WAL {wal} · SQL {statements}")
}

fn database_metric_gaps(database: &crate::metrics::DatabaseRates) -> Vec<(&'static str, &str)> {
    [
        ("commits/s", &database.commits_per_second),
        ("rollbacks/s", &database.rollbacks_per_second),
        ("transactions/s", &database.transactions_per_second),
        ("rollback ratio", &database.rollback_percent),
        ("reads/s", &database.reads_per_second),
        ("hits/s", &database.hits_per_second),
        ("cache hit ratio", &database.cache_hit_percent),
        ("temporary bytes/s", &database.temp_bytes_per_second),
        ("deadlocks/s", &database.deadlocks_per_second),
        ("inserts/s", &database.inserted_per_second),
        ("updates/s", &database.updated_per_second),
        ("deletes/s", &database.deleted_per_second),
        ("read time/s", &database.read_ms_per_second),
        ("write time/s", &database.write_ms_per_second),
    ]
    .into_iter()
    .filter_map(|(name, metric)| match metric {
        Observation::Available(_) => None,
        Observation::Unavailable(reason) => Some((name, reason.as_str())),
    })
    .collect()
}

fn interval_coverage(coverage: &mut Vec<String>, rates: &IntervalMetrics) {
    match &rates.database {
        Observation::Unavailable(reason) => {
            coverage.push(format!("Database interval: unavailable ({reason})"))
        }
        Observation::Available(database) => {
            for (name, reason) in database_metric_gaps(database) {
                coverage.push(format!("Database interval {name}: unavailable ({reason})"));
            }
        }
    }
    match &rates.wal {
        Observation::Unavailable(reason) => {
            coverage.push(format!("WAL interval: unavailable ({reason})"))
        }
        Observation::Available(wal) => {
            for (name, metric) in [
                ("bytes/s", &wal.bytes_per_second),
                ("records/s", &wal.records_per_second),
                ("buffers full/s", &wal.buffers_full_per_second),
            ] {
                if let Observation::Unavailable(reason) = metric {
                    coverage.push(format!("WAL interval {name}: unavailable ({reason})"));
                }
            }
        }
    }
    match &rates.statements {
        Observation::Unavailable(reason) => {
            coverage.push(format!("Statement interval: unavailable ({reason})"))
        }
        Observation::Available(entries) => {
            for entry in entries {
                if let Observation::Unavailable(reason) = &entry.calls_per_second {
                    coverage.push(format!("Statement interval query {}, user {}, database {}, top level {}: unavailable ({reason})", entry.identity.queryid, entry.identity.userid, entry.identity.dbid, entry.identity.toplevel));
                }
            }
        }
    }
}

fn availability<T>(coverage: &mut Vec<String>, name: &str, observation: &Observation<T>) {
    match observation {
        Observation::Available(_) => coverage.push(format!("{name}: collected")),
        Observation::Unavailable(reason) => {
            coverage.push(format!("{name}: unavailable ({reason})"))
        }
    }
}

fn finding(
    id: String,
    severity: Severity,
    title: String,
    evidence: Vec<String>,
    interpretation: &str,
    next_steps: &[&str],
) -> Finding {
    Finding {
        id,
        severity,
        title,
        evidence,
        interpretation: interpretation.into(),
        next_steps: next_steps.iter().map(|step| (*step).into()).collect(),
        related_pids: Vec::new(),
    }
}

fn upstream(start: i32, sessions: &BTreeMap<i32, &Session>) -> BTreeSet<i32> {
    let mut found = BTreeSet::new();
    let mut pending = sessions
        .get(&start)
        .map(|s| s.blockers.clone())
        .unwrap_or_default();
    while let Some(pid) = pending.pop() {
        if found.insert(pid)
            && let Some(session) = sessions.get(&pid)
        {
            pending.extend(session.blockers.iter().copied());
        }
    }
    found
}

fn blocking_findings(sessions: &[Session], output: &mut Vec<Finding>) {
    let indexed: BTreeMap<_, _> = sessions.iter().map(|s| (s.pid, s)).collect();
    if indexed.len() != sessions.len() {
        output.push(finding("blocking-identity".into(), Severity::Warning, "Blocking sample contains duplicate PIDs".into(), vec![format!("{} rows, {} unique PIDs", sessions.len(), indexed.len())], "Observation: this sample cannot establish an unambiguous blocking graph.", &["Collect another capture and inspect collector warnings before attributing blockers."]));
        return;
    }
    let ancestors: BTreeMap<_, _> = indexed
        .keys()
        .map(|pid| (*pid, upstream(*pid, &indexed)))
        .collect();
    let mut cycles = BTreeSet::new();
    let mut unresolved: BTreeMap<i32, BTreeSet<i32>> = BTreeMap::new();
    for (&waiter, parents) in &ancestors {
        for parent in parents {
            if !indexed.contains_key(parent) {
                unresolved.entry(*parent).or_default().insert(waiter);
            }
        }
        if parents.contains(&waiter) {
            let members: Vec<_> = parents
                .iter()
                .filter(|pid| {
                    ancestors
                        .get(pid)
                        .is_some_and(|other| other.contains(&waiter))
                })
                .copied()
                .collect();
            cycles.insert(members);
        }
    }
    for pids in cycles {
        let mut item = finding(
            format!(
                "blocking-cycle:{}",
                pids.iter()
                    .map(i32::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Severity::Critical,
            "Observed blocking graph contains a cycle".into(),
            vec![format!("Mutually reachable blocker PIDs: {pids:?}")],
            "Observation: the collected blocking relationships form a cycle. Inference: this may be a transient deadlock; a non-atomic sample cannot prove PostgreSQL has detected a deadlock.",
            &[
                "Capture again and correlate with the database deadlock counter and server logs.",
                "Inspect transaction ownership and lock acquisition order before taking any operational action.",
            ],
        );
        item.related_pids = pids;
        output.push(item);
    }
    for (&pid, session) in &indexed {
        if !session.blockers.is_empty() {
            continue;
        }
        let waiters: Vec<_> = ancestors
            .iter()
            .filter(|(waiter, parents)| **waiter != pid && parents.contains(&pid))
            .map(|(waiter, _)| *waiter)
            .collect();
        if waiters.is_empty() {
            continue;
        }
        let direct = sessions
            .iter()
            .filter(|session| session.blockers.contains(&pid))
            .count();
        let mut item = finding(
            format!("blocking-root:{pid}"),
            if waiters.len() >= 5 {
                Severity::Critical
            } else {
                Severity::Warning
            },
            format!("PID {pid} is an observed root blocker"),
            vec![
                format!(
                    "{direct} direct and {} total downstream waiters: {waiters:?}",
                    waiters.len()
                ),
                format!(
                    "State: {}; transaction age: {} ms; application: {}",
                    session.state.as_deref().unwrap_or("unavailable"),
                    session
                        .transaction_age_ms
                        .map_or_else(|| "unavailable".into(), |v| v.to_string()),
                    session.application
                ),
            ],
            "Observation: blocked sessions reach this backend and it has no observed blocker. This identifies a root in the captured graph, not the business cause of the incident. Five or more downstream waiters raises priority.",
            &[
                "Inspect the root transaction and its application owner, then the waiting operations.",
                "Take a second capture to distinguish persistent contention from a short-lived queue. pgtrail never cancels sessions.",
            ],
        );
        item.related_pids.push(pid);
        item.related_pids.extend(waiters);
        output.push(item);
    }
    for (pid, waiters) in unresolved {
        let mut item = finding(
            format!("blocking-unresolved:{pid}"),
            Severity::Warning,
            format!("Blocker PID {pid} is outside the visible sample"),
            vec![format!("Downstream waiter PIDs: {waiters:?}")],
            if pid == 0 {
                "Observation: PostgreSQL reported blocker PID 0, which can represent a prepared transaction. No backend identity can be assigned."
            } else {
                "Observation: blocker identity is missing. It may have exited between collection queries or be outside the visible scope; it cannot be named a confirmed root blocker."
            },
            &[
                "Repeat the capture and check activity visibility.",
                "For PID 0, inspect prepared transactions with an authorized database operator.",
            ],
        );
        item.related_pids = waiters.into_iter().collect();
        output.push(item);
    }
}

fn session_findings(sessions: &[Session], output: &mut Vec<Finding>) {
    for session in sessions {
        let idle = matches!(
            session.state.as_deref(),
            Some("idle in transaction" | "idle in transaction (aborted)")
        );
        let age = session.transaction_age_ms.unwrap_or_default();
        if (idle && age >= 60_000) || age >= 300_000 {
            let mut item = finding(
                format!("transaction:{}", session.pid),
                if age >= 1_800_000 {
                    Severity::Critical
                } else {
                    Severity::Warning
                },
                format!(
                    "PID {} has {} transaction",
                    session.pid,
                    if idle {
                        "an idle open"
                    } else {
                        "a long-running"
                    }
                ),
                vec![format!(
                    "State: {}; transaction age {:.1} seconds; application: {}",
                    session.state.as_deref().unwrap_or("unavailable"),
                    age as f64 / 1000.0,
                    session.application
                )],
                "Observation: transaction age crosses the 60-second idle or 5-minute open-transaction investigation threshold (30 minutes raises priority). Inference: an open transaction may retain locks or an old snapshot, but age alone does not prove it prevents vacuum.",
                &[
                    "Inspect transaction ownership, intended duration, and observed blocking relationships.",
                    "Check application transaction boundaries and relevant timeout settings; correlate vacuum impact with database evidence.",
                ],
            );
            item.related_pids = vec![session.pid];
            output.push(item);
        }
        if session.state.as_deref() == Some("active")
            && let Some(age) = session.query_age_ms.filter(|age| *age >= 60_000)
        {
            let mut item = finding(
                format!("active-query:{}", session.pid),
                if age >= 300_000 {
                    Severity::Warning
                } else {
                    Severity::Info
                },
                format!(
                    "PID {} has an active query older than one minute",
                    session.pid
                ),
                vec![format!(
                    "Current query age {:.1} seconds; wait: {} / {}",
                    age as f64 / 1000.0,
                    session
                        .wait_event_type
                        .as_deref()
                        .unwrap_or("none / unavailable"),
                    session
                        .wait_event
                        .as_deref()
                        .unwrap_or("none / unavailable")
                )],
                "Observation: the currently active query has elapsed at least 60 seconds (5 minutes raises priority). Elapsed time includes waiting and does not establish CPU consumption or poor query design.",
                &[
                    "Inspect its wait event and blockers, and verify whether this duration is expected for the workload.",
                    "Use the statement interval view to separate current elapsed time from completed execution statistics.",
                ],
            );
            item.related_pids = vec![session.pid];
            output.push(item);
        }
    }
}

fn database_findings(snapshot: &Snapshot, output: &mut Vec<Finding>, coverage: &mut Vec<String>) {
    let Some(db) = snapshot.health.database.available() else {
        return;
    };
    if !db.track_counts {
        coverage.push("track_counts is disabled; tuple estimates and cumulative usage counters cannot support normal maintenance analysis".into());
    }
    if !db.track_io_timing {
        coverage.push(
            "track_io_timing is disabled; recorded zero timing cannot establish cheap I/O".into(),
        );
    }
    let usable = db.max_connections.saturating_sub(db.reserved_connections);
    if usable > 0 && db.cluster_backends >= 0 {
        let percent = db.cluster_backends as f64 / usable as f64 * 100.0;
        if percent >= 80.0 {
            output.push(finding("connection-pressure".into(), if percent >= 95.0 { Severity::Critical } else { Severity::Warning }, "Connection usage is near the ordinary connection budget".into(), vec![format!("{} cluster client backends / {usable} nominal non-reserved slots ({percent:.1}%); {} connections in this database", db.cluster_backends, db.num_backends)], "Observation: cluster client backends are at least 80% of max_connections minus reserved slots (95% raises priority). This is an approximate pressure signal: role privileges, reserved-slot use, and connection churn affect actual admission.", &["Inspect connection counts by application and pool configuration, including idle connections.", "Correlate with connection admission errors before changing pooling or connection limits."]));
        }
    }
    xid_finding(
        "database".into(),
        &snapshot.source.database,
        db.frozen_xid_age,
        output,
    );
    if !db.autovacuum {
        output.push(finding("autovacuum-disabled".into(), Severity::Warning, "Autovacuum is disabled at server level".into(), vec!["Observed autovacuum = off".into()], "Observation: normal automatic vacuum/analyze scheduling is disabled globally. PostgreSQL can still start anti-wraparound vacuum, and table settings or scheduled manual maintenance must be considered.", &["Verify the maintenance policy and inspect dead tuple estimates, analyze recency, and frozen transaction ID ages.", "Ask the database operator to review the setting if disabling autovacuum was not intentional."]));
    }
}

fn xid_finding(id: String, name: &str, age: i64, output: &mut Vec<Finding>) {
    if age < 1_000_000_000 {
        return;
    }
    output.push(finding(format!("xid-age:{id}"), if age >= 1_500_000_000 { Severity::Critical } else { Severity::Warning }, format!("High frozen transaction ID age: {name}"), vec![format!("Observed age: {age} transactions")], "Observation: frozen transaction ID age exceeds the conservative one-billion investigation threshold (1.5 billion raises priority). This is transaction age, not wall-clock time or a prediction of when wraparound protection will occur.", &["Inspect vacuum progress, freeze settings, and transactions or replication slots retaining old horizons.", "Escalate to the database operator to review anti-wraparound maintenance and current server warnings."]));
}

fn relation_findings(snapshot: &Snapshot, output: &mut Vec<Finding>) {
    let Some(relations) = snapshot.health.tables.available() else {
        return;
    };
    let counts = snapshot
        .health
        .database
        .available()
        .is_some_and(|db| db.track_counts);
    for table in &relations.tables {
        let name = format!("{}.{}", table.schema, table.name);
        xid_finding(
            format!("table:{}", table.oid),
            &name,
            table.frozen_xid_age,
            output,
        );
        if !counts {
            continue;
        }
        let total = table.live_tuples as f64 + table.dead_tuples as f64;
        if table.dead_tuples >= 10_000
            && table.live_tuples >= 0
            && total > 0.0
            && table.dead_tuples as f64 / total >= 0.2
        {
            output.push(finding(format!("dead-tuples:{}", table.oid), Severity::Warning, format!("Elevated estimated dead tuple share: {name}"), vec![format!("{} estimated dead / {} estimated live tuples ({:.1}% estimated dead share)", table.dead_tuples, table.live_tuples, table.dead_tuples as f64 / total * 100.0), format!("Last autovacuum: {}; last manual vacuum: {}", optional_time(table.last_autovacuum), optional_time(table.last_vacuum))], "Observation: statistics estimate at least 10,000 dead tuples and a 20% dead share. These are estimates, not an exact bloat or reclaimable-disk measurement; normal write bursts may cross this threshold.", &["Inspect active vacuum progress and per-table autovacuum settings, then compare estimates over time.", "Check long transactions and replication horizons before concluding vacuum is ineffective."]));
        }
        if table.modified_since_analyze >= 10_000
            && table.live_tuples >= 0
            && table.modified_since_analyze as f64 >= table.live_tuples.max(1) as f64 * 0.2
        {
            output.push(finding(format!("analyze-drift:{}", table.oid), Severity::Info, format!("Many tuples modified since analyze: {name}"), vec![format!("{} estimated modifications since analyze; {} estimated live tuples; last autoanalyze: {}; last analyze: {}", table.modified_since_analyze, table.live_tuples, optional_time(table.last_autoanalyze), optional_time(table.last_analyze))], "Observation: estimated modifications exceed 10,000 and 20% of estimated live tuples. Inference: planner statistics may be less representative; this threshold alone does not prove a bad plan.", &["Review analyze scheduling and table-specific thresholds.", "Correlate changing execution plans or latency with data distribution before scheduling maintenance."]));
        }
    }
    for index in &relations.indexes {
        if !index.valid {
            output.push(finding(format!("invalid-index:{}", index.oid), Severity::Warning, format!("Index is not valid: {}.{}", index.schema, index.name), vec![format!("Table: {}.{}; index OID {}; primary: {}; unique: {}", index.schema, index.table, index.oid, index.primary, index.unique)], "Observation: PostgreSQL reports indisvalid = false. An index build may still be in progress or a concurrent build may have failed; this does not justify dropping an index without reviewing its role.", &["Inspect index build progress and recent DDL errors with the database operator.", "Review constraints and the intended index definition before planning any repair."]));
        }
    }
}

fn replication_findings(snapshot: &Snapshot, output: &mut Vec<Finding>) {
    let Some(replication) = snapshot.health.replication.available() else {
        return;
    };
    for slot in &replication.slots {
        if slot.wal_status.as_deref() == Some("lost") {
            output.push(finding(format!("slot-lost:{}", slot.name), Severity::Critical, format!("Replication slot has lost required WAL: {}", slot.name), vec!["Observed wal_status = lost".into()], "Observation: the slot no longer retains all required WAL. The downstream consumer's recovery procedure must be reviewed.", &["Verify the consumer status and its resynchronization procedure with the operator.", "Inspect WAL retention policy and disk capacity; do not drop the slot without confirming ownership."]));
        } else if !slot.active
            && slot
                .retained_bytes
                .is_some_and(|bytes| bytes >= 1_073_741_824)
        {
            output.push(finding(format!("slot-retention:{}", slot.name), Severity::Warning, format!("Inactive slot retains at least 1 GiB of WAL: {}", slot.name), vec![format!("Observed retained bytes: {}; WAL status: {}", slot.retained_bytes.unwrap_or_default(), slot.wal_status.as_deref().unwrap_or("unavailable"))], "Observation: an inactive slot retains at least 1 GiB of WAL. Inference: a disconnected consumer may increase WAL disk pressure; retained bytes do not show free disk space or prove an outage.", &["Identify the consumer and compare retained WAL in later captures.", "Check server disk capacity and the consumer recovery plan before changing slot retention."]));
        }
        if slot
            .safe_wal_size
            .is_some_and(|bytes| (0..268_435_456).contains(&bytes))
        {
            output.push(finding(format!("slot-headroom:{}", slot.name), Severity::Warning, format!("Replication slot has limited safe WAL headroom: {}", slot.name), vec![format!("safe_wal_size: {} bytes", slot.safe_wal_size.unwrap_or_default())], "Observation: safe_wal_size is below 256 MiB. Future WAL generation may endanger the slot; this is not an estimate of time remaining.", &["Review the consumer lag, WAL generation rate, and configured slot retention limit."]));
        }
    }
    for replica in &replication.senders {
        if let Some(bytes) = replica
            .sent_replay_lag_bytes
            .filter(|bytes| *bytes >= 268_435_456)
        {
            let mut item = finding(
                format!("replica-backlog:{}", replica.pid),
                Severity::Warning,
                format!(
                    "Replica {} has at least 256 MiB sent-to-replay backlog",
                    replica.application
                ),
                vec![format!(
                    "Observed sent minus replay LSN: {bytes} bytes; state: {}",
                    replica.state.as_deref().unwrap_or("unavailable")
                )],
                "Observation: this sender reports a byte gap between sent and replayed WAL. Inference: transport, receiver write, flush, or replay may be contributing; one sample cannot identify the cause or show end-to-end primary lag.",
                &[
                    "Compare the byte backlog across captures and inspect receiver/replay state.",
                    "Correlate with workload, network, and storage metrics before attributing a bottleneck.",
                ],
            );
            item.related_pids = vec![replica.pid];
            output.push(item);
        }
    }
    if replication.in_recovery
        && replication
            .receive_replay_lag_bytes
            .is_some_and(|bytes| bytes >= 268_435_456)
    {
        output.push(finding("standby-replay-backlog".into(), Severity::Warning, "Standby has at least 256 MiB received-to-replayed WAL backlog".into(), vec![format!("Observed receive minus replay LSN: {} bytes", replication.receive_replay_lag_bytes.unwrap_or_default())], "Observation: received WAL is ahead of replay by at least 256 MiB. This byte backlog is distinct from time since the last replayed transaction, which may grow on an idle primary.", &["Inspect replay state and compare subsequent captures, including recovery conflicts.", "Correlate replica queries and storage activity with replay progress."]));
    }
}

fn interval_findings(
    snapshot: &Snapshot,
    previous: Option<&Snapshot>,
    rates: &IntervalMetrics,
    output: &mut Vec<Finding>,
) {
    if let Some(db) = rates.database.available() {
        if db.deadlocks.available().is_some_and(|count| *count > 0) {
            output.push(finding("interval-deadlocks".into(), Severity::Warning, "PostgreSQL counted deadlocks during the interval".into(), vec![format!("{} deadlocks over {:.3} seconds", db.deadlocks.available().copied().unwrap_or_default(), rates.elapsed_seconds.unwrap_or_default())], "Observation: the database deadlock counter increased on a valid shared baseline. The involved transactions may already have ended and cannot be identified from this counter alone.", &["Correlate the interval with server deadlock logs and application retry errors.", "Review transaction lock acquisition order for the affected operations."]));
        }
        if db
            .rollback_percent
            .available()
            .is_some_and(|percent| *percent >= 10.0)
            && db
                .transactions_per_second
                .available()
                .is_some_and(|rate| *rate * rates.elapsed_seconds.unwrap_or_default() >= 20.0)
        {
            output.push(finding("interval-rollbacks".into(), Severity::Warning, "Rollbacks exceed 10% of completed transactions".into(), vec![format!("Rollback share {:.1}%; transaction rate {:.2}/s", db.rollback_percent.available().copied().unwrap_or_default(), db.transactions_per_second.available().copied().unwrap_or_default())], "Observation: at least 20 transactions completed and at least 10% rolled back. Inference: errors, intentional rollback workflows, or client behavior may explain the share; the counter does not establish application failure.", &["Correlate with application error rates and intentional rollback patterns.", "Inspect deadlock and timeout logs before attributing rollback causes."]));
        }
        if db
            .temp_bytes
            .available()
            .is_some_and(|bytes| *bytes >= 67_108_864)
        {
            output.push(finding("interval-temp-files".into(), Severity::Info, "At least 64 MiB of temporary files written in the interval".into(), vec![format!("{} temporary bytes; {:.1} bytes/s", db.temp_bytes.available().copied().unwrap_or_default(), db.temp_bytes_per_second.available().copied().unwrap_or_default())], "Observation: database temporary file bytes increased by at least 64 MiB. Inference: sorts, hashes, or other operations may have spilled; this counter alone does not prove work_mem is too low or that the spill is harmful.", &["Inspect statement interval temp blocks and correlate with affected query plans.", "Consider workload concurrency and per-operation memory use before changing memory settings."]));
        }
    }
    let Some(entries) = rates.statements.available() else {
        return;
    };
    let total: f64 = entries
        .iter()
        .filter_map(|entry| entry.exec_ms_per_second.available())
        .sum();
    for entry in entries {
        let identity = &entry.identity;
        let id = format!(
            "{}:{}:{}:{}",
            identity.userid, identity.dbid, identity.queryid, identity.toplevel
        );
        let exec_rate = entry
            .exec_ms_per_second
            .available()
            .copied()
            .unwrap_or_default();
        if total > 0.0
            && exec_rate / total >= 0.5
            && exec_rate * rates.elapsed_seconds.unwrap_or_default() >= 1000.0
        {
            output.push(finding(format!("statement-share:{id}"), Severity::Info, format!("Query ID {} dominates comparable execution time", identity.queryid), vec![format!("{:.1}% of summed valid statement interval execution time; {exec_rate:.1} execution ms/s; user OID {}; top level: {}", exec_rate / total * 100.0, identity.userid, identity.toplevel)], "Observation: this entry accounts for at least half of execution time among valid matched entries, with at least one second of interval execution. This is observed execution-time concentration, not CPU share; nested statements may overlap and unavailable entries are excluded.", &["Inspect interval calls, mean duration, reads, and temporary blocks for this statement.", "Correlate with workload expectations before tuning a frequently executed statement."]));
        }
        let baseline = previous
            .and_then(|before| before.statements.available())
            .and_then(|stats| {
                stats.entries.iter().find(|old| {
                    old.userid == identity.userid
                        && old.dbid == identity.dbid
                        && old.queryid == identity.queryid
                        && old.toplevel == identity.toplevel
                })
            });
        if let (Some(old), Some(mean), Some(calls)) = (
            baseline,
            entry.mean_exec_ms.available(),
            entry.calls_per_second.available(),
        ) && old.calls >= 20
            && old.mean_exec_ms.is_finite()
            && old.mean_exec_ms > 0.0
            && calls * rates.elapsed_seconds.unwrap_or_default() >= 5.0
            && *mean >= 100.0
            && *mean >= old.mean_exec_ms * 3.0
        {
            output.push(finding(format!("statement-latency:{id}"), Severity::Warning, format!("Query ID {} interval mean exceeds its historical mean", identity.queryid), vec![format!("Interval mean {mean:.1} ms vs prior cumulative mean {:.1} ms; baseline calls {}; interval calls {:.0}; captured at {}", old.mean_exec_ms, old.calls, calls * rates.elapsed_seconds.unwrap_or_default(), snapshot.completed_at.to_rfc3339())], "Observation: a valid interval with at least five calls has mean execution at least 100 ms and three times the prior cumulative mean (at least 20 historical calls). Inference: workload inputs, contention, plan changes, or infrastructure may explain it; this is a heuristic comparison, not a percentile regression test.", &["Compare statement interval reads, temporary blocks, and concurrent waits.", "Correlate with workload input changes and authorized plan evidence before naming a cause."]));
        }
    }
}

fn optional_time(time: Option<chrono::DateTime<chrono::Utc>>) -> String {
    time.map_or_else(
        || "unavailable / never recorded".into(),
        |time| time.to_rfc3339(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::tests::snapshot;
    use chrono::Duration;

    fn blank() -> Snapshot {
        let mut snapshot = snapshot();
        snapshot.health = Default::default();
        snapshot
    }

    fn ids(analysis: &Analysis) -> Vec<&str> {
        analysis
            .findings
            .iter()
            .map(|finding| finding.id.as_str())
            .collect()
    }

    #[test]
    fn synthetic_provenance_does_not_hide_real_collection_or_interval_gaps() {
        let mut snapshot = crate::demo::snapshot(0, false);
        assert!(snapshot.is_synthetic());
        assert!(snapshot.is_complete());
        let analysis = analyze(&snapshot, None);
        assert!(analysis.coverage[0].contains("SYNTHETIC"));
        assert!(analysis.coverage[1].contains("8/8 sections; complete collection"));
        assert!(analysis.coverage[2].contains("DB unavailable"));
        assert!(
            analysis
                .coverage
                .iter()
                .any(|line| line.contains("A previous capture is required"))
        );
        snapshot
            .warnings
            .push("Session visibility is restricted".into());
        assert!(!snapshot.is_complete());
        assert!(collection_summary(&snapshot).contains("LIMITED"));
        snapshot.warnings.pop();
        snapshot.health.io = Observation::Unavailable("restricted role".into());
        assert!(!snapshot.is_complete());
        let analysis = analyze(&snapshot, None);
        let unavailable = analysis
            .coverage
            .iter()
            .position(|line| line.contains("I/O: unavailable (restricted role)"))
            .unwrap();
        let success = analysis
            .coverage
            .iter()
            .position(|line| line.ends_with(": collected"))
            .unwrap();
        assert!(unavailable < success);
        assert!(analysis.coverage[1].contains("7/8 sections; 1 unavailable"));
    }

    #[test]
    fn interval_readiness_is_partial_when_individual_counters_are_invalid() {
        let before = snapshot();
        let mut after = before.clone();
        after.started_at += Duration::seconds(10);
        after.completed_at += Duration::seconds(10);
        if let Observation::Available(db) = &mut after.health.database {
            db.deadlocks = -1;
        }
        let analysis = analyze(&after, Some(&before));
        assert!(interval_summary(&analysis.rates).contains("DB partial"));
        assert!(
            analysis
                .coverage
                .iter()
                .any(|line| line.contains("Database interval deadlocks/s: unavailable"))
        );
        assert!(
            analysis
                .rates
                .database
                .available()
                .unwrap()
                .deadlocks
                .available()
                .is_none()
        );
    }

    #[test]
    fn missing_observations_are_coverage_gaps_not_a_health_verdict() {
        let mut snapshot = blank();
        snapshot.activity = Observation::Unavailable("restricted role".into());
        snapshot.statements = Observation::Unavailable("extension absent".into());
        let analysis = analyze(&snapshot, None);
        assert!(analysis.findings.is_empty());
        assert!(
            analysis
                .coverage
                .iter()
                .any(|line| line.contains("restricted role"))
        );
        assert!(
            analysis
                .coverage
                .iter()
                .any(|line| line.contains("not proof of health"))
        );
        assert!(analysis.rates.elapsed_seconds.is_none());
    }

    #[test]
    fn baseline_identity_and_collection_warnings_remain_in_analysis_coverage() {
        let mut before = snapshot();
        let mut after = before.clone();
        after.started_at += Duration::seconds(10);
        after.completed_at += Duration::seconds(10);
        before.source.system_identifier = None;
        before
            .warnings
            .push("Partial baseline activity visibility".into());
        let analysis = analyze(&after, Some(&before));
        assert!(analysis.rates.database.available().is_some());
        assert!(
            analysis
                .coverage
                .iter()
                .any(|line| line.contains("weaker identity check"))
        );
        assert!(
            analysis
                .coverage
                .iter()
                .any(|line| line == "Baseline capture: Partial baseline activity visibility")
        );
    }

    #[test]
    fn blocking_graph_follows_transitive_roots_without_blaming_middle_waiters() {
        let mut snapshot = blank();
        if let Observation::Available(sessions) = &mut snapshot.activity {
            let mut middle = sessions[0].clone();
            middle.pid = 456;
            middle.blockers = vec![123];
            let mut leaf = middle.clone();
            leaf.pid = 789;
            leaf.blockers = vec![456, 456];
            sessions.extend([middle, leaf]);
        }
        let analysis = analyze(&snapshot, None);
        let findings: Vec<_> = analysis
            .findings
            .iter()
            .filter(|finding| finding.id.starts_with("blocking-root"))
            .collect();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].id, "blocking-root:123");
        assert_eq!(findings[0].related_pids, vec![123, 456, 789]);
        assert!(findings[0].evidence[0].contains("1 direct and 2 total"));
    }

    #[test]
    fn cycles_are_finite_and_unresolved_or_prepared_blockers_are_not_invented_roots() {
        let mut snapshot = blank();
        if let Observation::Available(sessions) = &mut snapshot.activity {
            sessions[0].blockers = vec![456];
            let mut second = sessions[0].clone();
            second.pid = 456;
            second.blockers = vec![123, 999, 0];
            sessions.push(second);
        }
        let analysis = analyze(&snapshot, None);
        assert_eq!(
            analysis
                .findings
                .iter()
                .filter(|finding| finding.id.starts_with("blocking-cycle"))
                .count(),
            1
        );
        assert!(
            !ids(&analysis)
                .iter()
                .any(|id| id.starts_with("blocking-root"))
        );
        assert!(ids(&analysis).contains(&"blocking-unresolved:999"));
        assert!(
            analysis
                .findings
                .iter()
                .any(|finding| finding.id == "blocking-unresolved:0"
                    && finding.interpretation.contains("prepared transaction"))
        );
    }

    #[test]
    fn disappearing_blocker_does_not_remain_as_a_current_root() {
        let mut before = blank();
        if let Observation::Available(sessions) = &mut before.activity {
            let mut waiter = sessions[0].clone();
            waiter.pid = 456;
            waiter.blockers = vec![123];
            sessions.push(waiter);
        }
        let mut after = before.clone();
        after.started_at += Duration::seconds(10);
        after.completed_at += Duration::seconds(10);
        if let Observation::Available(sessions) = &mut after.activity {
            sessions.remove(0);
        }
        let analysis = analyze(&after, Some(&before));
        assert!(!ids(&analysis).contains(&"blocking-root:123"));
        assert!(ids(&analysis).contains(&"blocking-unresolved:123"));
    }

    #[test]
    fn long_idle_transactions_and_active_query_age_remain_distinct() {
        let mut snapshot = blank();
        if let Observation::Available(sessions) = &mut snapshot.activity {
            sessions[0].state = Some("idle in transaction".into());
            sessions[0].transaction_age_ms = Some(70_000);
            // A malformed/imported idle record must not be called an active slow query.
            sessions[0].query_age_ms = Some(600_000);
        }
        let analysis = analyze(&snapshot, None);
        assert!(ids(&analysis).contains(&"transaction:123"));
        assert!(!ids(&analysis).contains(&"active-query:123"));
    }

    #[test]
    fn maintenance_findings_use_estimates_and_respect_disabled_statistics() {
        let mut snapshot = snapshot();
        if let Observation::Available(tables) = &mut snapshot.health.tables {
            tables.tables[0].dead_tuples = 100_000;
            tables.tables[0].live_tuples = 100_000;
            tables.tables[0].modified_since_analyze = 100_000;
            tables.indexes[0].valid = false;
        }
        let analysis = analyze(&snapshot, None);
        let dead = analysis
            .findings
            .iter()
            .find(|finding| finding.id == "dead-tuples:17001")
            .unwrap();
        assert!(dead.interpretation.contains("not an exact bloat"));
        assert!(ids(&analysis).contains(&"invalid-index:17101"));
        assert!(ids(&analysis).contains(&"analyze-drift:17001"));
        if let Observation::Available(db) = &mut snapshot.health.database {
            db.track_counts = false;
        }
        let analysis = analyze(&snapshot, None);
        assert!(!ids(&analysis).contains(&"dead-tuples:17001"));
        assert!(ids(&analysis).contains(&"invalid-index:17101"));
    }

    #[test]
    fn lag_time_alone_on_an_idle_standby_is_not_a_replication_incident() {
        let mut snapshot = blank();
        snapshot.health.replication = crate::demo::snapshot(0, false).health.replication;
        if let Observation::Available(replication) = &mut snapshot.health.replication {
            replication.in_recovery = true;
            replication.replay_delay_seconds = Some(86400.0);
            replication.receive_replay_lag_bytes = Some(0);
            replication.senders.clear();
            replication.slots.clear();
        }
        let analysis = analyze(&snapshot, None);
        assert!(analysis.findings.is_empty());
        if let Observation::Available(replication) = &mut snapshot.health.replication {
            replication.receive_replay_lag_bytes = Some(536_870_912);
        }
        assert!(ids(&analyze(&snapshot, None)).contains(&"standby-replay-backlog"));
    }

    #[test]
    fn deadlock_and_temp_findings_require_positive_valid_deltas() {
        let before = snapshot();
        let mut after = before.clone();
        after.started_at += Duration::seconds(10);
        after.completed_at += Duration::seconds(10);
        if let Observation::Available(db) = &mut after.health.database {
            db.deadlocks += 2;
            db.temp_bytes += 100_000_000;
            db.xact_commit += 70;
            db.xact_rollback += 30;
        }
        let analysis = analyze(&after, Some(&before));
        for id in [
            "interval-deadlocks",
            "interval-temp-files",
            "interval-rollbacks",
        ] {
            assert!(ids(&analysis).contains(&id), "{id}");
        }
        if let Observation::Available(db) = &mut after.health.database {
            db.stats_reset = None;
        }
        let analysis = analyze(&after, Some(&before));
        assert!(!ids(&analysis).iter().any(|id| id.starts_with("interval-")));
    }

    #[test]
    fn latency_heuristic_requires_enough_completed_calls_and_valid_baselines() {
        let mut before = blank();
        if let Observation::Available(stats) = &mut before.statements {
            stats.entries[0].calls = 100;
            stats.entries[0].total_exec_ms = 2_000.0;
            stats.entries[0].mean_exec_ms = 20.0;
        }
        let mut after = before.clone();
        after.started_at += Duration::seconds(10);
        after.completed_at += Duration::seconds(10);
        if let Observation::Available(stats) = &mut after.statements {
            stats.entries[0].calls += 10;
            stats.entries[0].total_exec_ms += 2_000.0;
        }
        let analysis = analyze(&after, Some(&before));
        assert!(
            ids(&analysis)
                .iter()
                .any(|id| id.starts_with("statement-latency"))
        );
        if let Observation::Available(stats) = &mut after.statements {
            stats.truncated = true;
        }
        assert!(
            !ids(&analyze(&after, Some(&before)))
                .iter()
                .any(|id| id.starts_with("statement-latency"))
        );
    }
}
