//! Metadata and aggregate statistics only. Every section has its own savepoint:
//! a permission change or unsupported field cannot discard the activity capture.
use sqlx::{Connection, PgConnection, Postgres, Row, Transaction};

use super::CollectorError;
use crate::model::{
    DatabaseStats, Health, IndexStats, IoStats, Observation, RelationStats, Replica,
    ReplicationSlot, ReplicationStats, TableStats, VacuumProgress, WalStats,
};

const RELATION_LIMIT: usize = 1_000;

pub(super) async fn collect(
    connection: &mut PgConnection,
    all_stats: bool,
) -> Result<Health, CollectorError> {
    let mut savepoint = connection.begin().await?;
    let result = database(&mut savepoint).await;
    let database = finish(savepoint, result, "Database statistics").await?;

    let mut savepoint = connection.begin().await?;
    let result = relations(&mut savepoint).await;
    let tables = finish(savepoint, result, "Table and index statistics").await?;

    let replication = if all_stats {
        let mut savepoint = connection.begin().await?;
        let result = replication(&mut savepoint).await;
        finish(savepoint, result, "Replication").await?
    } else {
        Observation::Unavailable(
            "Replication visibility is restricted; pg_read_all_stats is required.".into(),
        )
    };

    let mut savepoint = connection.begin().await?;
    let result = wal(&mut savepoint).await;
    let wal = finish(savepoint, result, "WAL statistics").await?;

    let mut savepoint = connection.begin().await?;
    let result = io(&mut savepoint).await;
    let io = finish(savepoint, result, "I/O statistics").await?;

    let vacuum = if all_stats {
        let mut savepoint = connection.begin().await?;
        let result = vacuum(&mut savepoint).await;
        finish(savepoint, result, "Vacuum progress").await?
    } else {
        Observation::Unavailable(
            "Vacuum visibility is restricted; pg_read_all_stats is required.".into(),
        )
    };
    Ok(Health {
        database,
        tables,
        replication,
        wal,
        io,
        vacuum,
    })
}

async fn finish<T>(
    savepoint: Transaction<'_, Postgres>,
    result: Result<T, CollectorError>,
    section: &str,
) -> Result<Observation<T>, CollectorError> {
    match result {
        Ok(value) => {
            savepoint.commit().await?;
            Ok(Observation::Available(value))
        }
        Err(error) => {
            savepoint.rollback().await?;
            Ok(Observation::Unavailable(format!(
                "{section} unavailable: {error}"
            )))
        }
    }
}

async fn database(connection: &mut PgConnection) -> Result<DatabaseStats, CollectorError> {
    let row = sqlx::query(
        "SELECT s.stats_reset, pg_catalog.pg_database_size(d.oid) AS size_bytes,
                s.numbackends::bigint AS num_backends,
                (SELECT count(*) FROM pg_catalog.pg_stat_activity
                    WHERE backend_type = 'client backend') AS cluster_backends,
                current_setting('max_connections')::bigint AS max_connections,
                (current_setting('superuser_reserved_connections')::bigint
                    + current_setting('reserved_connections')::bigint) AS reserved_connections,
                s.xact_commit, s.xact_rollback, s.blks_read, s.blks_hit,
                s.tup_returned, s.tup_fetched, s.tup_inserted, s.tup_updated, s.tup_deleted,
                s.conflicts, s.deadlocks, s.temp_files, s.temp_bytes,
                s.blk_read_time AS blk_read_time_ms, s.blk_write_time AS blk_write_time_ms,
                pg_catalog.age(d.datfrozenxid)::bigint AS frozen_xid_age,
                current_setting('autovacuum')::boolean AS autovacuum,
                current_setting('track_counts')::boolean AS track_counts,
                current_setting('track_io_timing')::boolean AS track_io_timing
         FROM pg_catalog.pg_stat_database s
         JOIN pg_catalog.pg_database d ON d.oid = s.datid
         WHERE d.datname = current_database()",
    )
    .fetch_one(connection)
    .await?;
    Ok(DatabaseStats {
        stats_reset: row.try_get("stats_reset")?,
        size_bytes: row.try_get("size_bytes")?,
        num_backends: row.try_get("num_backends")?,
        cluster_backends: row.try_get("cluster_backends")?,
        max_connections: row.try_get("max_connections")?,
        reserved_connections: row.try_get("reserved_connections")?,
        xact_commit: row.try_get("xact_commit")?,
        xact_rollback: row.try_get("xact_rollback")?,
        blks_read: row.try_get("blks_read")?,
        blks_hit: row.try_get("blks_hit")?,
        tup_returned: row.try_get("tup_returned")?,
        tup_fetched: row.try_get("tup_fetched")?,
        tup_inserted: row.try_get("tup_inserted")?,
        tup_updated: row.try_get("tup_updated")?,
        tup_deleted: row.try_get("tup_deleted")?,
        conflicts: row.try_get("conflicts")?,
        deadlocks: row.try_get("deadlocks")?,
        temp_files: row.try_get("temp_files")?,
        temp_bytes: row.try_get("temp_bytes")?,
        blk_read_time_ms: row.try_get("blk_read_time_ms")?,
        blk_write_time_ms: row.try_get("blk_write_time_ms")?,
        frozen_xid_age: row.try_get("frozen_xid_age")?,
        autovacuum: row.try_get("autovacuum")?,
        track_counts: row.try_get("track_counts")?,
        track_io_timing: row.try_get("track_io_timing")?,
    })
}

async fn relations(connection: &mut PgConnection) -> Result<RelationStats, CollectorError> {
    // Select candidates from catalog estimates before touching relation files.
    // MATERIALIZED keeps the limit ahead of the exact-size function calls even
    // if PostgreSQL would otherwise inline/reorder these expressions. Estimates
    // can be stale: this is a bounded sample, not a guaranteed largest-N result.
    let rows = sqlx::query(
        "WITH index_pages AS MATERIALIZED (
            SELECT i.indrelid, sum(greatest(c.relpages, 0))::bigint AS pages
            FROM pg_catalog.pg_index i
            JOIN pg_catalog.pg_class c ON c.oid = i.indexrelid
            GROUP BY i.indrelid
         ), candidates AS MATERIALIZED (
            SELECT s.relid, c.relfrozenxid,
                (greatest(c.relpages, 0)::bigint + coalesce(ip.pages, 0)
                    + greatest(coalesce(t.relpages, 0), 0)::bigint
                    + coalesce(tip.pages, 0)) AS estimated_pages
            FROM pg_catalog.pg_stat_user_tables s
            JOIN pg_catalog.pg_class c ON c.oid = s.relid
            LEFT JOIN pg_catalog.pg_class t ON t.oid = c.reltoastrelid
            LEFT JOIN index_pages ip ON ip.indrelid = c.oid
            LEFT JOIN index_pages tip ON tip.indrelid = t.oid
            ORDER BY estimated_pages DESC, s.relid LIMIT $1
         ), sizes AS MATERIALIZED (
            SELECT relid, pg_catalog.pg_table_size(relid) AS table_bytes,
                pg_catalog.pg_indexes_size(relid) AS index_bytes FROM candidates
         )
         SELECT s.relid::bigint AS oid, s.schemaname::text AS schema,
                s.relname::text AS name, z.table_bytes + z.index_bytes AS total_bytes,
                z.table_bytes, z.index_bytes,
                s.seq_scan, s.idx_scan, s.n_live_tup AS live_tuples, s.n_dead_tup AS dead_tuples,
                s.n_mod_since_analyze AS modified_since_analyze,
                s.n_tup_ins AS inserts, s.n_tup_upd AS updates, s.n_tup_del AS deletes,
                s.last_vacuum, s.last_autovacuum, s.last_analyze, s.last_autoanalyze,
                pg_catalog.age(c.relfrozenxid)::bigint AS frozen_xid_age
         FROM pg_catalog.pg_stat_user_tables s
         JOIN candidates c ON c.relid = s.relid
         JOIN sizes z ON z.relid = s.relid
         ORDER BY c.estimated_pages DESC, s.relid",
    )
    .bind((RELATION_LIMIT + 1) as i64)
    .fetch_all(&mut *connection)
    .await?;
    let mut truncated = rows.len() > RELATION_LIMIT;
    let tables = rows
        .into_iter()
        .take(RELATION_LIMIT)
        .map(|row| {
            Ok(TableStats {
                oid: row.try_get("oid")?,
                schema: row.try_get("schema")?,
                name: row.try_get("name")?,
                total_bytes: row.try_get("total_bytes")?,
                table_bytes: row.try_get("table_bytes")?,
                index_bytes: row.try_get("index_bytes")?,
                seq_scan: row.try_get("seq_scan")?,
                idx_scan: row.try_get("idx_scan")?,
                live_tuples: row.try_get("live_tuples")?,
                dead_tuples: row.try_get("dead_tuples")?,
                modified_since_analyze: row.try_get("modified_since_analyze")?,
                inserts: row.try_get("inserts")?,
                updates: row.try_get("updates")?,
                deletes: row.try_get("deletes")?,
                last_vacuum: row.try_get("last_vacuum")?,
                last_autovacuum: row.try_get("last_autovacuum")?,
                last_analyze: row.try_get("last_analyze")?,
                last_autoanalyze: row.try_get("last_autoanalyze")?,
                frozen_xid_age: row.try_get("frozen_xid_age")?,
            })
        })
        .collect::<Result<Vec<_>, CollectorError>>()?;
    let rows = sqlx::query(
        "WITH candidates AS MATERIALIZED (
            SELECT s.indexrelid, greatest(c.relpages, 0) AS estimated_pages
            FROM pg_catalog.pg_stat_user_indexes s
            JOIN pg_catalog.pg_class c ON c.oid = s.indexrelid
            ORDER BY estimated_pages DESC, s.indexrelid LIMIT $1
         ), sizes AS MATERIALIZED (
            SELECT indexrelid, pg_catalog.pg_relation_size(indexrelid) AS size_bytes
            FROM candidates
         )
         SELECT s.indexrelid::bigint AS oid, s.relid::bigint AS table_oid,
                s.schemaname::text AS schema, s.relname::text AS table,
                s.indexrelname::text AS name, z.size_bytes,
                s.idx_scan AS scans, s.idx_tup_read AS tuples_read, s.idx_tup_fetch AS tuples_fetched,
                i.indisunique AS unique, i.indisprimary AS primary, i.indisvalid AS valid
         FROM pg_catalog.pg_stat_user_indexes s
         JOIN pg_catalog.pg_index i ON i.indexrelid = s.indexrelid
         JOIN candidates c ON c.indexrelid = s.indexrelid
         JOIN sizes z ON z.indexrelid = s.indexrelid
         ORDER BY c.estimated_pages DESC, s.indexrelid",
    )
    .bind((RELATION_LIMIT + 1) as i64)
    .fetch_all(connection)
    .await?;
    truncated |= rows.len() > RELATION_LIMIT;
    let indexes = rows
        .into_iter()
        .take(RELATION_LIMIT)
        .map(|row| {
            Ok(IndexStats {
                oid: row.try_get("oid")?,
                table_oid: row.try_get("table_oid")?,
                schema: row.try_get("schema")?,
                table: row.try_get("table")?,
                name: row.try_get("name")?,
                size_bytes: row.try_get("size_bytes")?,
                scans: row.try_get("scans")?,
                tuples_read: row.try_get("tuples_read")?,
                tuples_fetched: row.try_get("tuples_fetched")?,
                unique: row.try_get("unique")?,
                primary: row.try_get("primary")?,
                valid: row.try_get("valid")?,
            })
        })
        .collect::<Result<Vec<_>, CollectorError>>()?;
    Ok(RelationStats {
        tables,
        indexes,
        truncated,
    })
}

async fn replication(connection: &mut PgConnection) -> Result<ReplicationStats, CollectorError> {
    let row = sqlx::query(
        "SELECT pg_catalog.pg_is_in_recovery() AS in_recovery,
                CASE WHEN pg_catalog.pg_is_in_recovery()
                    THEN extract(epoch FROM clock_timestamp() - pg_catalog.pg_last_xact_replay_timestamp())::float8
                END AS replay_delay_seconds,
                CASE WHEN pg_catalog.pg_is_in_recovery()
                    THEN pg_catalog.pg_wal_lsn_diff(pg_catalog.pg_last_wal_receive_lsn(),
                        pg_catalog.pg_last_wal_replay_lsn())::bigint
                END AS receive_replay_lag_bytes",
    )
    .fetch_one(&mut *connection)
    .await?;
    let rows = sqlx::query(
        "SELECT pid, application_name AS application, client_addr::text AS client,
                state, sync_state,
                pg_catalog.pg_wal_lsn_diff(sent_lsn, replay_lsn)::bigint AS sent_replay_lag_bytes,
                (extract(epoch FROM replay_lag) * 1000)::float8 AS replay_lag_ms
         FROM pg_catalog.pg_stat_replication ORDER BY pid",
    )
    .fetch_all(&mut *connection)
    .await?;
    let senders = rows
        .into_iter()
        .map(|row| {
            Ok(Replica {
                pid: row.try_get("pid")?,
                application: row.try_get("application")?,
                client: row.try_get("client")?,
                state: row.try_get("state")?,
                sync_state: row.try_get("sync_state")?,
                sent_replay_lag_bytes: row.try_get("sent_replay_lag_bytes")?,
                replay_lag_ms: row.try_get("replay_lag_ms")?,
            })
        })
        .collect::<Result<Vec<_>, CollectorError>>()?;
    let rows = sqlx::query(
        "SELECT slot_name::text AS name, slot_type, database::text, active,
                pg_catalog.pg_wal_lsn_diff(
                    CASE WHEN pg_catalog.pg_is_in_recovery() THEN pg_catalog.pg_last_wal_replay_lsn()
                        ELSE pg_catalog.pg_current_wal_lsn() END,
                    restart_lsn)::bigint AS retained_bytes,
                wal_status, safe_wal_size
         FROM pg_catalog.pg_replication_slots ORDER BY slot_name",
    )
    .fetch_all(connection)
    .await?;
    let slots = rows
        .into_iter()
        .map(|row| {
            Ok(ReplicationSlot {
                name: row.try_get("name")?,
                slot_type: row.try_get("slot_type")?,
                database: row.try_get("database")?,
                active: row.try_get("active")?,
                retained_bytes: row.try_get("retained_bytes")?,
                wal_status: row.try_get("wal_status")?,
                safe_wal_size: row.try_get("safe_wal_size")?,
            })
        })
        .collect::<Result<Vec<_>, CollectorError>>()?;
    Ok(ReplicationStats {
        in_recovery: row.try_get("in_recovery")?,
        // Time since the last replayed transaction can grow on an idle primary.
        // It is exposed separately from byte backlog, never treated as lag alone.
        replay_delay_seconds: row.try_get("replay_delay_seconds")?,
        receive_replay_lag_bytes: row.try_get("receive_replay_lag_bytes")?,
        senders,
        slots,
    })
}

async fn wal(connection: &mut PgConnection) -> Result<WalStats, CollectorError> {
    let row = sqlx::query(
        "SELECT stats_reset, wal_records AS records, wal_fpi AS full_page_images,
                wal_bytes::float8 AS bytes, wal_buffers_full AS buffers_full
         FROM pg_catalog.pg_stat_wal",
    )
    .fetch_one(connection)
    .await?;
    Ok(WalStats {
        stats_reset: row.try_get("stats_reset")?,
        records: row.try_get("records")?,
        full_page_images: row.try_get("full_page_images")?,
        bytes: row.try_get("bytes")?,
        buffers_full: row.try_get("buffers_full")?,
    })
}

async fn io(connection: &mut PgConnection) -> Result<Vec<IoStats>, CollectorError> {
    // These columns exist on PG16–18. PG18 changed byte accounting and moved
    // WAL timing into this view; read/write operation counts remain comparable.
    let rows = sqlx::query(
        "SELECT backend_type, object, context, reads, writes,
                CASE WHEN (object = 'wal' AND current_setting('track_wal_io_timing')::boolean)
                        OR (object <> 'wal' AND current_setting('track_io_timing')::boolean)
                    THEN read_time END AS read_time_ms,
                CASE WHEN (object = 'wal' AND current_setting('track_wal_io_timing')::boolean)
                        OR (object <> 'wal' AND current_setting('track_io_timing')::boolean)
                    THEN write_time END AS write_time_ms,
                hits, evictions, fsyncs, stats_reset
         FROM pg_catalog.pg_stat_io ORDER BY backend_type, object, context",
    )
    .fetch_all(connection)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(IoStats {
                backend_type: row.try_get("backend_type")?,
                object: row.try_get("object")?,
                context: row.try_get("context")?,
                reads: row.try_get("reads")?,
                writes: row.try_get("writes")?,
                read_time_ms: row.try_get("read_time_ms")?,
                write_time_ms: row.try_get("write_time_ms")?,
                hits: row.try_get("hits")?,
                evictions: row.try_get("evictions")?,
                fsyncs: row.try_get("fsyncs")?,
                stats_reset: row.try_get("stats_reset")?,
            })
        })
        .collect()
}

async fn vacuum(connection: &mut PgConnection) -> Result<Vec<VacuumProgress>, CollectorError> {
    let rows = sqlx::query(
        "SELECT pid, relid::bigint AS table_oid, phase,
                heap_blks_total AS heap_blocks_total, heap_blks_scanned AS heap_blocks_scanned,
                heap_blks_vacuumed AS heap_blocks_vacuumed
         FROM pg_catalog.pg_stat_progress_vacuum
         WHERE datid = (SELECT oid FROM pg_catalog.pg_database WHERE datname = current_database())
         ORDER BY pid",
    )
    .fetch_all(connection)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(VacuumProgress {
                pid: row.try_get("pid")?,
                table_oid: row.try_get("table_oid")?,
                phase: row.try_get("phase")?,
                heap_blocks_total: row.try_get("heap_blocks_total")?,
                heap_blocks_scanned: row.try_get("heap_blocks_scanned")?,
                heap_blocks_vacuumed: row.try_get("heap_blocks_vacuumed")?,
            })
        })
        .collect()
}
