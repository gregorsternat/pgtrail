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
        health: health(sequence),
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

fn health(sequence: u64) -> crate::model::Health {
    use crate::model::*;
    let epoch = DateTime::from_timestamp(1_790_000_000, 0).unwrap_or_default();
    let n = i64::try_from(sequence.min(1_000_000)).unwrap_or_default();
    let table = |oid, name: &str, size, live, dead| TableStats {
        oid,
        schema: "public".into(),
        name: name.into(),
        total_bytes: size,
        table_bytes: size * 3 / 4,
        index_bytes: size / 4,
        seq_scan: 200 + n,
        idx_scan: Some(120_000 + n * 5),
        live_tuples: live,
        dead_tuples: dead,
        modified_since_analyze: dead / 2,
        inserts: 10_000 + n * 10,
        updates: 20_000 + n * 20,
        deletes: 1000 + n,
        last_vacuum: None,
        last_autovacuum: Some(epoch),
        last_analyze: None,
        last_autoanalyze: Some(epoch),
        frozen_xid_age: 50_000_000,
    };
    Health {
        database: Observation::Available(DatabaseStats {
            stats_reset: Some(epoch),
            size_bytes: 8_000_000_000,
            num_backends: 42,
            cluster_backends: 84,
            max_connections: 100,
            reserved_connections: 3,
            xact_commit: 250_000 + n * 150,
            xact_rollback: 1000 + n * 3,
            blks_read: 100_000 + n * 10,
            blks_hit: 8_000_000 + n * 1000,
            tup_returned: 9_000_000 + n * 2000,
            tup_fetched: 4_000_000 + n * 1000,
            tup_inserted: 150_000 + n * 50,
            tup_updated: 100_000 + n * 30,
            tup_deleted: 10_000 + n,
            conflicts: 0,
            deadlocks: 2,
            temp_files: 40 + n,
            temp_bytes: 100_000_000 + n * 4_000_000,
            blk_read_time_ms: 15_000.0 + n as f64 * 20.0,
            blk_write_time_ms: 8_000.0 + n as f64 * 10.0,
            frozen_xid_age: 50_000_000,
            autovacuum: true,
            track_counts: true,
            track_io_timing: true,
        }),
        tables: Observation::Available(RelationStats {
            tables: vec![
                table(17001, "orders", 5_000_000_000, 4_000_000, 25_000),
                table(17002, "inventory", 900_000_000, 150_000, 80_000),
            ],
            indexes: vec![
                IndexStats {
                    oid: 17101,
                    table_oid: 17001,
                    schema: "public".into(),
                    table: "orders".into(),
                    name: "orders_pkey".into(),
                    size_bytes: 600_000_000,
                    scans: 100_000 + n * 50,
                    tuples_read: 800_000 + n * 100,
                    tuples_fetched: 600_000 + n * 80,
                    unique: true,
                    primary: true,
                    valid: true,
                },
                IndexStats {
                    oid: 17102,
                    table_oid: 17002,
                    schema: "public".into(),
                    table: "inventory".into(),
                    name: "inventory_lookup".into(),
                    size_bytes: 80_000_000,
                    scans: 50_000 + n * 30,
                    tuples_read: 150_000 + n * 50,
                    tuples_fetched: 120_000 + n * 40,
                    unique: false,
                    primary: false,
                    valid: true,
                },
            ],
            truncated: false,
        }),
        replication: Observation::Available(ReplicationStats {
            in_recovery: false,
            replay_delay_seconds: None,
            receive_replay_lag_bytes: None,
            senders: vec![Replica {
                pid: 4201,
                application: "analytics-replica".into(),
                client: Some("192.0.2.20".into()),
                state: Some("streaming".into()),
                sync_state: Some("async".into()),
                sent_replay_lag_bytes: Some(8_000_000),
                replay_lag_ms: Some(1200.0),
            }],
            slots: vec![
                ReplicationSlot {
                    name: "analytics_replica".into(),
                    slot_type: "physical".into(),
                    database: None,
                    active: true,
                    retained_bytes: Some(16_000_000),
                    wal_status: Some("reserved".into()),
                    safe_wal_size: None,
                },
                ReplicationSlot {
                    name: "old_cdc_consumer".into(),
                    slot_type: "logical".into(),
                    database: Some("demo_shop".into()),
                    active: false,
                    retained_bytes: Some(3_000_000_000),
                    wal_status: Some("extended".into()),
                    safe_wal_size: None,
                },
            ],
        }),
        wal: Observation::Available(WalStats {
            stats_reset: Some(epoch),
            records: 2_000_000 + n * 200,
            full_page_images: 10_000 + n,
            bytes: 4_000_000_000.0 + n as f64 * 2_000_000.0,
            buffers_full: 10,
        }),
        io: Observation::Available(vec![IoStats {
            backend_type: "client backend".into(),
            object: "relation".into(),
            context: "normal".into(),
            reads: Some(100_000 + n * 10),
            writes: Some(30_000 + n * 5),
            read_time_ms: Some(50_000.0 + n as f64 * 20.0),
            write_time_ms: Some(40_000.0 + n as f64 * 10.0),
            hits: Some(8_000_000 + n * 1000),
            evictions: Some(4000 + n),
            fsyncs: None,
            stats_reset: Some(epoch),
        }]),
        vacuum: Observation::Available(vec![VacuumProgress {
            pid: 4301,
            table_oid: 17002,
            phase: "scanning heap".into(),
            heap_blocks_total: 100_000,
            heap_blocks_scanned: 30_000 + (n % 60_000),
            heap_blocks_vacuumed: 0,
        }]),
    }
}
