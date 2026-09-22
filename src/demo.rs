//! An explicitly synthetic incident for trying the entire workflow.
use chrono::{DateTime, Duration, Utc};

use crate::model::{
    Observation, SNAPSHOT_VERSION, Session, Snapshot, Source, Statement, StatementStats,
};

pub(crate) fn snapshot(sequence: u64, include_query_text: bool) -> Snapshot {
    let origin = DateTime::from_timestamp(1_790_000_000, 0).unwrap_or_default();
    let now = Utc::now();
    let blocked = sequence % 4 < 2;
    let session =
        |pid, user: &str, application: &str, state: &str, age, blockers: Vec<i32>, sql: &str| {
            Session {
                pid,
                backend_start: Some(origin + Duration::seconds(i64::from(pid))),
                user: Some(user.into()),
                database: Some("demo_shop".into()),
                application: application.into(),
                client: Some("192.0.2.10".into()),
                state: Some(state.into()),
                query_age_ms: (state == "active").then_some(age),
                transaction_age_ms: (state != "idle").then_some(age + 5_000),
                wait_event_type: if !blockers.is_empty() {
                    Some("Lock".into())
                } else if state == "active" {
                    Some("IO".into())
                } else {
                    Some("Client".into())
                },
                wait_event: if !blockers.is_empty() {
                    Some("transactionid".into())
                } else if state == "active" {
                    Some("DataFileRead".into())
                } else {
                    Some("ClientRead".into())
                },
                blockers,
                query: include_query_text.then(|| sql.into()),
            }
        };
    let activity = vec![
        session(
            4101,
            "checkout",
            "checkout-api",
            if blocked {
                "idle in transaction"
            } else {
                "idle"
            },
            85_000,
            vec![],
            "UPDATE inventory SET stock = stock - 1 WHERE id = $1",
        ),
        session(
            4102,
            "checkout",
            "checkout-worker",
            "active",
            if blocked { 32_000 } else { 200 },
            if blocked { vec![4101] } else { vec![] },
            "UPDATE inventory SET stock = stock - 1 WHERE id = $1",
        ),
        session(
            4103,
            "analytics",
            "daily-report",
            "active",
            123_000,
            vec![],
            "SELECT region, sum(total) FROM orders GROUP BY region",
        ),
        session(
            4104,
            "api",
            "catalog-api",
            "idle",
            0,
            vec![],
            "SELECT id, title FROM products WHERE category = $1",
        ),
    ];
    let entries = [
        (
            701,
            280_i64,
            820_000.0,
            "SELECT region, sum(total) FROM orders GROUP BY region",
        ),
        (
            702,
            18_200,
            251_000.0,
            "UPDATE inventory SET stock = stock - 1 WHERE id = $1",
        ),
        (
            703,
            142_000,
            108_000.0,
            "SELECT id, title FROM products WHERE category = $1",
        ),
    ]
    .into_iter()
    .map(|(queryid, calls, total, query)| {
        let growth = i64::try_from(sequence.min(1_000_000)).unwrap_or_default();
        let calls = calls + growth * 10;
        let total_exec_ms = total + growth as f64 * 80.0;
        Statement {
            userid: 100,
            dbid: 16384,
            queryid,
            toplevel: true,
            stats_since: Some(origin),
            calls,
            total_exec_ms,
            mean_exec_ms: total_exec_ms / calls as f64,
            rows: calls * 3,
            shared_blks_hit: calls * 15,
            shared_blks_read: calls,
            temp_blks_written: 0,
            query: include_query_text.then(|| query.into()),
        }
    })
    .collect();
    Snapshot {
        schema_version: SNAPSHOT_VERSION,
        started_at: now,
        completed_at: now,
        source: Source {
            endpoint: "synthetic-demo:5432".into(),
            database: "demo_shop".into(),
            database_oid: 16384,
            server_version: "18 (synthetic demo)".into(),
            server_started_at: origin,
            system_identifier: Some("pgtrail-demo-v1".into()),
        },
        activity: Observation::Available(activity),
        statements: Observation::Available(StatementStats {
            reset_at: Some(origin),
            dealloc: Some(0),
            truncated: false,
            entries,
        }),
        warnings: vec!["Synthetic demo data; no PostgreSQL connection is open.".into()],
    }
}
