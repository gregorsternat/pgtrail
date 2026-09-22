//! Bounded, read-only PostgreSQL observations. Driver errors never leave this module.
mod health;

use std::{str::FromStr, time::Duration};

use chrono::Utc;
use sqlx::{
    AssertSqlSafe, ConnectOptions, Connection, PgConnection, PgPool, Row,
    postgres::{PgConnectOptions, PgPoolOptions, PgRow},
};
use thiserror::Error;

use crate::model::{
    Observation, SNAPSHOT_VERSION, Session, Snapshot, Source, Statement, StatementStats,
};

const STATEMENT_LIMIT: usize = 1_000;

/// Deliberately contains neither a driver error nor a connection string, including in Debug.
#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum CollectorError {
    #[error("invalid PostgreSQL connection configuration; check the URL and connection options")]
    InvalidConnection,
    #[error("collection timeout must be greater than zero")]
    InvalidTimeout,
    #[error("superuser connections are refused; use a non-superuser monitoring role")]
    Superuser,
    #[error("PostgreSQL 16, 17, or 18 is required for these observations")]
    UnsupportedServer,
    #[error("the connection could not enforce read-only transactions")]
    ReadOnlyRequired,
    #[error("collection timed out; the previous observation may be stale")]
    Timeout,
    #[error("PostgreSQL authentication failed; check the monitoring credentials")]
    Authentication,
    #[error("permission denied; check the monitoring role's database and statistics grants")]
    Permission,
    #[error(
        "could not connect to PostgreSQL; check the endpoint, TLS configuration, and server availability"
    )]
    Connection,
    #[error("PostgreSQL returned an unexpected observation format")]
    InvalidData,
    #[error(
        "statement statistics changed reset or eviction epoch during collection; refresh to collect a consistent observation"
    )]
    StatisticsChanged,
    #[error(
        "the PostgreSQL observation query failed; check server compatibility and monitoring permissions"
    )]
    QueryFailed,
}

impl From<sqlx::Error> for CollectorError {
    fn from(error: sqlx::Error) -> Self {
        match error {
            sqlx::Error::PoolTimedOut => Self::Timeout,
            sqlx::Error::Io(_)
            | sqlx::Error::Tls(_)
            | sqlx::Error::PoolClosed
            | sqlx::Error::Protocol(_) => Self::Connection,
            sqlx::Error::Configuration(_) => Self::InvalidConnection,
            sqlx::Error::ColumnDecode { .. }
            | sqlx::Error::ColumnNotFound(_)
            | sqlx::Error::ColumnIndexOutOfBounds { .. }
            | sqlx::Error::Decode(_)
            | sqlx::Error::RowNotFound
            | sqlx::Error::TypeNotFound { .. } => Self::InvalidData,
            sqlx::Error::Database(error) => match error.code().as_deref() {
                Some("57014" | "55P03") => Self::Timeout,
                Some("42501") => Self::Permission,
                Some(code) if code.starts_with("28") => Self::Authentication,
                Some(code) if code.starts_with("08") || code == "3D000" => Self::Connection,
                _ => Self::QueryFailed,
            },
            _ => Self::QueryFailed,
        }
    }
}

pub(crate) struct Collector {
    pool: PgPool,
    endpoint: String,
    include_query_text: bool,
    timeout: Duration,
}

impl Collector {
    /// Parses configuration without opening a network connection. Call inside a Tokio runtime.
    pub(crate) fn new(
        dsn: &str,
        include_query_text: bool,
        timeout: Duration,
    ) -> Result<Self, CollectorError> {
        if timeout.is_zero() {
            return Err(CollectorError::InvalidTimeout);
        }
        let options = connection_options(dsn, timeout)?;
        let endpoint = safe_endpoint(&options);
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .min_connections(0)
            .acquire_timeout(timeout)
            .idle_timeout(Duration::from_secs(60))
            .connect_lazy_with(options);
        Ok(Self {
            pool,
            endpoint,
            include_query_text,
            timeout,
        })
    }

    pub(crate) async fn collect(&self) -> Result<Snapshot, CollectorError> {
        tokio::time::timeout(self.timeout, self.collect_inner())
            .await
            .map_err(|_| CollectorError::Timeout)?
    }

    async fn collect_inner(&self) -> Result<Snapshot, CollectorError> {
        let started_at = Utc::now();
        let mut connection = self.pool.acquire().await?;
        // Reassert session settings with bound values after startup parsing as well. Raw
        // libpq-style options can contain an end-of-options marker, so appending startup
        // defaults alone is not sufficient to enforce the collector's session settings.
        sqlx::query(
            "SELECT pg_catalog.set_config('default_transaction_read_only', 'on', false),
                    pg_catalog.set_config('statement_timeout', $1, false),
                    pg_catalog.set_config('lock_timeout', $1, false),
                    pg_catalog.set_config('idle_in_transaction_session_timeout', $1, false),
                    pg_catalog.set_config('application_name', 'pgtrail', false),
                    pg_catalog.set_config('search_path', 'pg_catalog', false)",
        )
        .bind(
            self.timeout
                .as_millis()
                .clamp(1, i32::MAX as u128)
                .to_string(),
        )
        .execute(&mut *connection)
        .await?;
        // An explicit READ ONLY transaction reinforces the startup default. No user-supplied
        // SQL is executed, and optional queries use savepoints so a denial preserves activity.
        let mut transaction = connection.begin_with("BEGIN READ ONLY").await?;
        let row = sqlx::query(
            "SELECT current_database() AS database, d.oid::bigint AS database_oid,
                    current_setting('server_version') AS server_version,
                    current_setting('server_version_num')::int AS version_num,
                    pg_catalog.pg_postmaster_start_time() AS server_started_at,
                    current_setting('transaction_read_only') = 'on' AS read_only,
                    (SELECT bool_or(rolsuper) FROM pg_catalog.pg_roles
                     WHERE rolname IN (current_user, session_user)) AS superuser,
                    pg_catalog.pg_has_role(current_user, 'pg_read_all_stats', 'USAGE') AS all_stats,
                    pg_catalog.has_function_privilege(current_user,
                        'pg_catalog.pg_control_system()', 'EXECUTE') AS control_system
             FROM pg_catalog.pg_database d WHERE d.datname = current_database()",
        )
        .fetch_one(&mut *transaction)
        .await?;
        if row.try_get::<bool, _>("superuser")? {
            return Err(CollectorError::Superuser);
        }
        if !supported_version(row.try_get("version_num")?) {
            return Err(CollectorError::UnsupportedServer);
        }
        if !row.try_get::<bool, _>("read_only")? {
            return Err(CollectorError::ReadOnlyRequired);
        }
        let all_stats = row.try_get::<bool, _>("all_stats")?;
        let mut warnings = Vec::new();
        if !all_stats {
            warnings.push("Session visibility is restricted. Grant pg_read_all_stats to the monitoring role to observe other users' activity and statement identities.".to_owned());
        }
        let system_identifier = if row.try_get::<bool, _>("control_system")? {
            optional_system_identifier(&mut transaction).await?
        } else {
            None
        };
        let source = Source {
            endpoint: self.endpoint.clone(),
            database: row.try_get("database")?,
            database_oid: row.try_get("database_oid")?,
            server_version: row.try_get("server_version")?,
            server_started_at: row.try_get("server_started_at")?,
            system_identifier,
        };
        let activity = collect_activity(&mut transaction, self.include_query_text).await?;
        if activity
            .available()
            .is_some_and(|sessions| sessions.iter().any(|session| session.state.is_none()))
        {
            warnings.push("Some sessions have unavailable state or timing fields; NULL values are not healthy or idle observations.".to_owned());
        }
        let statements =
            collect_statements(&mut transaction, self.include_query_text, all_stats).await?;
        if statements.available().is_some_and(|stats| stats.truncated) {
            warnings.push(format!(
                "Statement collection is limited to the {STATEMENT_LIMIT} largest entries by total execution time; lower-ranked entries are omitted."
            ));
        }
        if statements.available().is_some_and(|stats| {
            stats
                .entries
                .iter()
                .any(|entry| entry.stats_since.is_none())
        }) {
            warnings.push("Per-statement reset epochs are unavailable in this extension version; cumulative rankings remain available, but safe per-statement deltas cannot be calculated.".into());
        }
        let health = health::collect(&mut transaction, all_stats).await?;
        if health
            .tables
            .available()
            .is_some_and(|stats| stats.truncated)
        {
            warnings.push("Relation collection is limited to 1000 tables and 1000 indexes selected by catalog size estimates in the current database. Larger relations may be omitted when estimates are stale; exact sizes are collected only for the bounded candidates.".into());
        }
        if health
            .database
            .available()
            .is_some_and(|stats| !stats.track_counts)
        {
            warnings.push("track_counts is disabled; table and index counters cannot establish current workload or maintenance health.".into());
        }
        transaction.commit().await?;
        Ok(Snapshot {
            schema_version: SNAPSHOT_VERSION,
            health,
            started_at,
            completed_at: Utc::now(),
            source,
            activity,
            statements,
            warnings,
        })
    }
}

fn supported_version(version_num: i32) -> bool {
    (160_000..190_000).contains(&version_num)
}

fn connection_options(dsn: &str, timeout: Duration) -> Result<PgConnectOptions, CollectorError> {
    validate_connection_url(dsn)?;
    let milliseconds = timeout.as_millis().clamp(1, i32::MAX as u128).to_string();
    Ok(PgConnectOptions::from_str(dsn)
        .map_err(|_| CollectorError::InvalidConnection)?
        .application_name("pgtrail")
        // Appending these options overrides conflicting values in a supplied URL.
        .options([
            ("application_name", "pgtrail"),
            ("default_transaction_read_only", "on"),
            ("statement_timeout", milliseconds.as_str()),
            ("lock_timeout", milliseconds.as_str()),
            ("idle_in_transaction_session_timeout", milliseconds.as_str()),
            ("search_path", "pg_catalog"),
        ])
        .disable_statement_logging())
}

fn validate_connection_url(dsn: &str) -> Result<(), CollectorError> {
    let url = url::Url::parse(dsn).map_err(|_| CollectorError::InvalidConnection)?;
    if !matches!(url.scheme(), "postgres" | "postgresql") || url.fragment().is_some() {
        return Err(CollectorError::InvalidConnection);
    }
    // SQLx 0.9 logs unknown option names AND values while parsing. Reject those before
    // reaching its parser, so a typo such as passwrod=secret cannot enter any subscriber.
    for (key, _) in url.query_pairs() {
        let recognized = matches!(
            key.as_ref(),
            "sslmode"
                | "ssl-mode"
                | "sslrootcert"
                | "ssl-root-cert"
                | "ssl-ca"
                | "sslcert"
                | "ssl-cert"
                | "sslkey"
                | "ssl-key"
                | "statement-cache-capacity"
                | "host"
                | "hostaddr"
                | "port"
                | "dbname"
                | "user"
                | "password"
                | "application_name"
                | "options"
        ) || key
            .strip_prefix("options[")
            .and_then(|key| key.strip_suffix(']'))
            .is_some_and(|key| !key.is_empty());
        if !recognized {
            return Err(CollectorError::InvalidConnection);
        }
    }
    Ok(())
}

fn safe_endpoint(options: &PgConnectOptions) -> String {
    let host = options
        .get_socket()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| options.get_host().to_owned());
    // A parsed host cannot include the URL's credentials. Also avoid reflecting control
    // sequences or URL-like host/socket values back into exports and terminal surfaces.
    if host.is_empty()
        || host
            .chars()
            .any(|character| character.is_control() || "@?#=&".contains(character))
    {
        return format!("configured-host:{}", options.get_port());
    }
    if host.contains(':') && !host.starts_with('/') {
        format!("[{host}]:{}", options.get_port())
    } else {
        format!("{host}:{}", options.get_port())
    }
}

async fn optional_system_identifier(
    connection: &mut PgConnection,
) -> Result<Option<String>, CollectorError> {
    let mut savepoint = connection.begin().await?;
    match sqlx::query_scalar("SELECT system_identifier::text FROM pg_catalog.pg_control_system()")
        .fetch_one(&mut *savepoint)
        .await
    {
        Ok(identifier) => {
            savepoint.commit().await?;
            Ok(Some(identifier))
        }
        Err(_) => {
            // Privileges can change between the capability check and the call. This identity
            // is optional; the database OID, endpoint and server start remain available.
            savepoint.rollback().await?;
            Ok(None)
        }
    }
}

async fn collect_activity(
    connection: &mut PgConnection,
    include_query_text: bool,
) -> Result<Observation<Vec<Session>>, CollectorError> {
    let mut savepoint = connection.begin().await?;
    let result = sqlx::query(
        "SELECT pid, backend_start, usename::text AS username, datname::text AS database,
                application_name AS application, client_addr::text AS client, state,
                CASE WHEN state = 'active' AND query_start IS NOT NULL
                     THEN greatest(0, (extract(epoch FROM (clock_timestamp() - query_start)) * 1000)::bigint)
                END AS query_age_ms,
                CASE WHEN xact_start IS NOT NULL
                     THEN greatest(0, (extract(epoch FROM (clock_timestamp() - xact_start)) * 1000)::bigint)
                END AS transaction_age_ms,
                wait_event_type, wait_event, pg_catalog.pg_blocking_pids(pid) AS blockers,
                CASE WHEN $1 THEN query ELSE NULL END AS query
         FROM pg_catalog.pg_stat_activity
         WHERE datname = current_database() AND pid <> pg_backend_pid()
         ORDER BY pid",
    )
    .bind(include_query_text)
    .fetch_all(&mut *savepoint)
    .await;
    match result {
        Ok(rows) => {
            let sessions = rows
                .into_iter()
                .map(session_from_row)
                .collect::<Result<Vec<_>, _>>()?;
            savepoint.commit().await?;
            Ok(Observation::Available(sessions))
        }
        Err(error) => {
            savepoint.rollback().await?;
            Ok(Observation::Unavailable(format!(
                "Activity unavailable: {}",
                CollectorError::from(error)
            )))
        }
    }
}

fn session_from_row(row: PgRow) -> Result<Session, CollectorError> {
    Ok(Session {
        pid: row.try_get("pid")?,
        backend_start: row.try_get("backend_start")?,
        user: row.try_get("username")?,
        database: row.try_get("database")?,
        application: row.try_get("application")?,
        client: row.try_get("client")?,
        state: row.try_get("state")?,
        query_age_ms: row.try_get("query_age_ms")?,
        transaction_age_ms: row.try_get("transaction_age_ms")?,
        wait_event_type: row.try_get("wait_event_type")?,
        wait_event: row.try_get("wait_event")?,
        blockers: row.try_get("blockers")?,
        query: row.try_get("query")?,
    })
}

async fn collect_statements(
    connection: &mut PgConnection,
    include_query_text: bool,
    all_stats: bool,
) -> Result<Observation<StatementStats>, CollectorError> {
    let mut savepoint = connection.begin().await?;
    let schema_result = sqlx::query_scalar::<_, String>(
        "SELECT n.nspname::text FROM pg_catalog.pg_extension e
         JOIN pg_catalog.pg_namespace n ON n.oid = e.extnamespace
         WHERE e.extname = 'pg_stat_statements'",
    )
    .fetch_optional(&mut *savepoint)
    .await;
    let schema = match schema_result {
        Ok(schema) => schema,
        Err(error) => {
            savepoint.rollback().await?;
            return Ok(Observation::Unavailable(format!(
                "pg_stat_statements capability discovery unavailable: {}",
                CollectorError::from(error)
            )));
        }
    };
    let Some(schema) = schema else {
        savepoint.commit().await?;
        return Ok(Observation::Unavailable(
            "pg_stat_statements is not installed in this database; an administrator must provision it separately.".to_owned(),
        ));
    };
    if !all_stats {
        savepoint.commit().await?;
        return Ok(Observation::Unavailable(
            "Statement identities are restricted; pg_read_all_stats is required for a complete statement observation.".to_owned(),
        ));
    }
    let result = statement_rows(&mut savepoint, &schema, include_query_text).await;
    match result {
        Ok(stats) => {
            savepoint.commit().await?;
            Ok(Observation::Available(stats))
        }
        Err(error) => {
            savepoint.rollback().await?;
            Ok(Observation::Unavailable(format!(
                "pg_stat_statements unavailable: {error}"
            )))
        }
    }
}

/// SQL identifiers cannot be bind parameters. Escape every quote in the catalog-provided
/// schema name and retain the surrounding quotes; no other dynamic SQL is interpolated.
fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

async fn statement_rows(
    connection: &mut PgConnection,
    schema: &str,
    include_query_text: bool,
) -> Result<StatementStats, CollectorError> {
    let quoted_schema = quote_identifier(schema);
    let info_query = format!(
        "SELECT stats_reset AS reset_at, dealloc FROM {quoted_schema}.pg_stat_statements_info"
    );
    // The schema identifier is quoted above. Both relation names and all SQL are static.
    let info = sqlx::query(AssertSqlSafe(info_query.clone()))
        .fetch_one(&mut *connection)
        .await?;
    let reset_at = info.try_get("reset_at")?;
    let dealloc = info.try_get("dealloc")?;
    // PG17 added this field, but an extension can retain an older SQL definition
    // after a server upgrade. Inspect the actual view instead of guessing from
    // server_version. A missing epoch stays NULL; the global reset time does not
    // establish that a particular entry survived eviction or a targeted reset.
    let has_stats_since: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT FROM pg_catalog.pg_attribute a
            JOIN pg_catalog.pg_class c ON c.oid = a.attrelid
            JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
            WHERE n.nspname = $1 AND c.relname = 'pg_stat_statements'
                AND a.attname = 'stats_since' AND a.attnum > 0 AND NOT a.attisdropped
        )",
    )
    .bind(schema)
    .fetch_one(&mut *connection)
    .await?;
    let stats_since = if has_stats_since {
        "stats_since"
    } else {
        "NULL::timestamptz AS stats_since"
    };
    let query = format!(
        "SELECT userid::bigint AS userid, dbid::bigint AS dbid, queryid, toplevel,
                {stats_since}, calls, total_exec_time AS total_exec_ms,
                mean_exec_time AS mean_exec_ms, rows, shared_blks_hit, shared_blks_read,
                temp_blks_written, CASE WHEN $1 THEN query ELSE NULL END AS query
         FROM {quoted_schema}.pg_stat_statements
         WHERE dbid = (SELECT oid FROM pg_catalog.pg_database WHERE datname = current_database())
         ORDER BY total_exec_time DESC, userid, dbid, queryid, toplevel LIMIT $2"
    );
    let rows = sqlx::query(AssertSqlSafe(query))
        .bind(include_query_text)
        .bind((STATEMENT_LIMIT + 1) as i64)
        .fetch_all(&mut *connection)
        .await?;
    let truncated = rows.len() > STATEMENT_LIMIT;
    let entries = rows
        .into_iter()
        .take(STATEMENT_LIMIT)
        .map(statement_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    let final_info = sqlx::query(AssertSqlSafe(info_query))
        .fetch_one(&mut *connection)
        .await?;
    if reset_at != final_info.try_get("reset_at")? || dealloc != final_info.try_get("dealloc")? {
        return Err(CollectorError::StatisticsChanged);
    }
    Ok(StatementStats {
        reset_at,
        dealloc,
        truncated,
        entries,
    })
}

fn statement_from_row(row: PgRow) -> Result<Statement, CollectorError> {
    Ok(Statement {
        userid: row.try_get("userid")?,
        dbid: row.try_get("dbid")?,
        // A NULL identity is a failed/partial observation, never a fabricated query ID.
        queryid: row.try_get("queryid")?,
        toplevel: row.try_get("toplevel")?,
        stats_since: row.try_get("stats_since")?,
        calls: row.try_get("calls")?,
        total_exec_ms: row.try_get("total_exec_ms")?,
        mean_exec_ms: row.try_get("mean_exec_ms")?,
        rows: row.try_get("rows")?,
        shared_blks_hit: row.try_get("shared_blks_hit")?,
        shared_blks_read: row.try_get("shared_blks_read")?,
        temp_blks_written: row.try_get("temp_blks_written")?,
        query: row.try_get("query")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_server_versions_are_explicit_and_bounded() {
        for version in [160_000, 160_015, 170_000, 170_011, 180_000, 180_006] {
            assert!(supported_version(version));
        }
        for version in [0, 150_019, 159_999, 190_000, 200_000] {
            assert!(!supported_version(version));
        }
    }

    #[test]
    fn invalid_urls_and_driver_errors_never_expose_secrets() {
        let secret = "secret-password@example.com/private-database";
        let options = connection_options(secret, Duration::from_secs(1));
        let error = options.expect_err("not a PostgreSQL URL");
        assert_eq!(error, CollectorError::InvalidConnection);
        assert!(!format!("{error:?}: {error}").contains(secret));
        for raw_error in [
            sqlx::Error::Protocol(secret.to_owned()),
            sqlx::Error::Configuration(secret.into()),
            sqlx::Error::ColumnNotFound(secret.to_owned()),
        ] {
            let safe = CollectorError::from(raw_error);
            assert!(!format!("{safe:?}: {safe}").contains(secret));
        }
    }

    #[test]
    fn options_enforce_read_only_and_bounded_queries_without_connecting() {
        let options = connection_options(
            "postgres://monitor:private-password@localhost:55432/private_db?application_name=other&options=-c%20default_transaction_read_only%3Doff",
            Duration::from_millis(2300),
        )
        .expect("valid URL");
        assert_eq!(options.get_application_name(), Some("pgtrail"));
        assert_eq!(safe_endpoint(&options), "localhost:55432");
        let settings = options.get_options().expect("startup settings");
        assert!(settings.contains("-c statement_timeout=2300"));
        assert!(settings.contains("-c lock_timeout=2300"));
        assert!(
            settings.rfind("default_transaction_read_only=on")
                > settings.rfind("default_transaction_read_only=off")
        );
    }

    #[test]
    fn unknown_url_options_and_non_postgres_schemes_are_rejected_before_driver_parsing() {
        for dsn in [
            "postgres://localhost/db?passwrod=private-password",
            "postgres://localhost/db?pass%77rod=private-password",
            "postgres://localhost/db?options[]=private-password",
            "postgres://localhost/db#private-password",
            "https://localhost/db?password=private-password",
        ] {
            let error = connection_options(dsn, Duration::from_secs(1))
                .expect_err("reject configuration before SQLx can log unknown options");
            assert_eq!(error, CollectorError::InvalidConnection);
            assert!(!format!("{error:?}: {error}").contains("private-password"));
        }
        for dsn in [
            "postgresql://localhost/db?sslmode=require&application_name=ignored",
            "postgres://localhost/db?options%5Bstatement_timeout%5D=1000",
            "postgres:///?host=/var/run/postgresql&dbname=db&user=monitor",
        ] {
            assert!(connection_options(dsn, Duration::from_secs(1)).is_ok());
        }
    }

    #[test]
    fn endpoints_do_not_include_credentials_or_terminal_control_sequences() {
        let options = PgConnectOptions::new()
            .host("::1")
            .port(55432)
            .username("private-user")
            .password("private-password")
            .database("private-database");
        assert_eq!(safe_endpoint(&options), "[::1]:55432");
        assert_eq!(
            safe_endpoint(&options.clone().host("user:password@example.com")),
            "configured-host:55432"
        );
        assert_eq!(
            safe_endpoint(&options.host("host\u{1b}[2J")),
            "configured-host:55432"
        );
    }

    #[test]
    fn extension_identifiers_are_quoted_as_a_single_identifier() {
        assert_eq!(quote_identifier("public"), "\"public\"");
        assert_eq!(
            quote_identifier("stats\"; SELECT 'unsafe'--"),
            "\"stats\"\"; SELECT 'unsafe'--\""
        );
    }

    #[tokio::test]
    async fn zero_timeout_is_rejected_before_network_access() {
        assert!(matches!(
            Collector::new("postgres://localhost/test", false, Duration::ZERO),
            Err(CollectorError::InvalidTimeout)
        ));
    }

    /// These tests only use the loopback Compose fixture and require a second explicit opt-in.
    /// No environment variable can supply an arbitrary production connection string.
    fn fixture_configuration() -> (PgConnectOptions, String) {
        assert_eq!(
            std::env::var("PGTRAIL_LIVE_TEST").as_deref(),
            Ok("1"),
            "set PGTRAIL_LIVE_TEST=1 after starting the disposable Compose fixture"
        );
        let port = std::env::var("PGTRAIL_POSTGRES_PORT")
            .unwrap_or_else(|_| "55432".to_owned())
            .parse::<u16>()
            .expect("fixture port must be a u16");
        let admin_password = std::env::var("PGTRAIL_ADMIN_PASSWORD")
            .unwrap_or_else(|_| "pgtrail-local-admin".to_owned());
        let monitor_password = std::env::var("PGTRAIL_MONITOR_PASSWORD")
            .unwrap_or_else(|_| "pgtrail-local-monitor".to_owned());
        let admin = PgConnectOptions::new()
            .host("127.0.0.1")
            .port(port)
            .username("pgtrail_admin")
            .password(&admin_password)
            .database("pgtrail_dev")
            .application_name("pgtrail_live_test")
            .ssl_mode(sqlx::postgres::PgSslMode::Disable)
            .options([
                ("default_transaction_read_only", "off"),
                ("statement_timeout", "5000"),
            ])
            .disable_statement_logging();
        let monitor = fixture_dsn(port, "pgtrail_monitor", &monitor_password, "pgtrail_dev");
        (admin, monitor)
    }

    fn fixture_dsn(port: u16, user: &str, password: &str, database: &str) -> String {
        let password = password
            .bytes()
            .map(|byte| format!("%{byte:02X}"))
            .collect::<String>();
        // User/database names are constants or generated ASCII test identifiers.
        format!("postgres://{user}:{password}@127.0.0.1:{port}/{database}?sslmode=disable")
    }

    async fn fixture_admin(options: &PgConnectOptions) -> PgConnection {
        let mut connection = tokio::time::timeout(Duration::from_secs(5), options.connect())
            .await
            .expect("fixture connection timeout")
            .expect("start docker compose up -d --wait before the live tests");
        let is_fixture: bool = sqlx::query_scalar(
            "SELECT current_user = 'pgtrail_admin'
                    AND current_database() = 'pgtrail_dev'
                    AND EXISTS (SELECT FROM pg_catalog.pg_class c
                                JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
                                WHERE n.nspname = 'public' AND c.relname = 'pgtrail_write_probe')",
        )
        .fetch_one(&mut connection)
        .await
        .expect("verify the disposable fixture");
        assert!(is_fixture, "refusing to mutate an unrecognized fixture");
        connection
    }

    #[tokio::test]
    #[ignore = "requires a disposable PostgreSQL 16-18 fixture and PGTRAIL_LIVE_TEST=1"]
    async fn live_read_only_collection_workload_and_blocking() {
        let (admin_options, monitor_dsn) = fixture_configuration();
        let mut blocker = fixture_admin(&admin_options).await;
        let mut waiter = fixture_admin(&admin_options).await;
        let collector = Collector::new(&monitor_dsn, false, Duration::from_secs(5))
            .expect("monitoring configuration");
        let snapshot = collector.collect().await.expect("initial observation");
        assert_eq!(snapshot.source.database, "pgtrail_dev");
        let server_version: i32 =
            sqlx::query_scalar("SELECT current_setting('server_version_num')::int")
                .fetch_one(&mut blocker)
                .await
                .expect("fixture server version");
        assert!(supported_version(server_version));
        assert!(snapshot.started_at <= snapshot.completed_at);
        let database = snapshot
            .health
            .database
            .available()
            .expect("database health");
        assert!(database.size_bytes > 0);
        assert!(database.num_backends >= 1);
        assert!(database.cluster_backends >= database.num_backends);
        assert!(database.max_connections > database.reserved_connections);
        assert!(database.track_counts);
        let relations = snapshot.health.tables.available().expect("relation health");
        assert!(!relations.truncated);
        let fixture_table = relations
            .tables
            .iter()
            .find(|table| table.schema == "public" && table.name == "pgtrail_write_probe")
            .expect("known fixture table");
        assert!(fixture_table.total_bytes > 0);
        assert!(fixture_table.table_bytes > 0);
        assert!(fixture_table.index_bytes > 0);
        let fixture_index = relations
            .indexes
            .iter()
            .find(|index| {
                index.table_oid == fixture_table.oid && index.name == "pgtrail_write_probe_pkey"
            })
            .expect("known fixture primary key");
        assert!(fixture_index.primary && fixture_index.unique && fixture_index.valid);
        let wal = snapshot.health.wal.available().expect("WAL health");
        assert!(wal.records > 0 && wal.bytes > 0.0);
        let io = snapshot.health.io.available().expect("I/O health");
        assert!(!io.is_empty());
        if !database.track_io_timing {
            assert!(
                io.iter()
                    .filter(|entry| entry.object != "wal")
                    .all(|entry| { entry.read_time_ms.is_none() && entry.write_time_ms.is_none() })
            );
        }
        let replication = snapshot
            .health
            .replication
            .available()
            .expect("replication health");
        assert!(!replication.in_recovery);
        assert!(replication.replay_delay_seconds.is_none());
        assert!(replication.receive_replay_lag_bytes.is_none());
        assert!(snapshot.health.vacuum.available().is_some());
        assert!(
            snapshot
                .activity
                .available()
                .expect("activity available")
                .iter()
                .all(|session| session.query.is_none())
        );
        assert!(
            snapshot
                .statements
                .available()
                .expect("statement statistics available")
                .entries
                .iter()
                .all(|statement| statement.query.is_none())
        );
        let readonly: bool = sqlx::query_scalar(
            "SELECT current_setting('default_transaction_read_only') = 'on'
                    AND current_setting('application_name') = 'pgtrail'",
        )
        .fetch_one(&collector.pool)
        .await
        .expect("verify collector session settings");
        assert!(readonly);
        assert!(
            sqlx::query("INSERT INTO public.pgtrail_write_probe VALUES (991)")
                .execute(&collector.pool)
                .await
                .is_err(),
            "monitoring connection cannot write the fixture table"
        );

        let with_text =
            Collector::new(&monitor_dsn, true, Duration::from_secs(5)).expect("query text opt-in");
        for _ in 0..3 {
            sqlx::query("SELECT sum(value)::bigint FROM generate_series(1, 997) AS value")
                .fetch_one(&mut blocker)
                .await
                .expect("known fixture workload");
        }
        let workload_snapshot = with_text.collect().await.expect("workload observation");
        let workload = workload_snapshot
            .statements
            .available()
            .expect("statement metrics")
            .entries
            .iter()
            .find(|statement| {
                statement
                    .query
                    .as_deref()
                    .is_some_and(|query| query.contains("sum(value)::bigint FROM generate_series"))
            })
            .expect("known workload appears in aggregate statistics");
        assert!(workload.calls >= 3);
        assert!(workload.total_exec_ms >= 0.0);
        assert!(workload.mean_exec_ms >= 0.0);
        assert_eq!(workload.rows, workload.calls);
        assert_eq!(workload.stats_since.is_some(), server_version >= 170_000);
        if server_version < 170_000 {
            assert!(
                workload_snapshot.warnings.iter().any(|warning| {
                    warning.contains("Per-statement reset epochs are unavailable")
                })
            );
        }

        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut blocker)
            .await
            .expect("blocker pid");
        let waiter_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut waiter)
            .await
            .expect("waiter pid");
        sqlx::query("BEGIN")
            .execute(&mut blocker)
            .await
            .expect("begin fixture blocker");
        sqlx::query("UPDATE public.pgtrail_write_probe SET id = id WHERE id = 1")
            .execute(&mut blocker)
            .await
            .expect("acquire fixture row lock");
        let waiting = tokio::spawn(async move {
            let result = sqlx::query("UPDATE public.pgtrail_write_probe SET id = id WHERE id = 1")
                .execute(&mut waiter)
                .await;
            (waiter, result)
        });
        let mut observed_waiter = None;
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if let Ok(snapshot) = collector.collect().await {
                observed_waiter = snapshot.activity.available().and_then(|sessions| {
                    sessions
                        .iter()
                        .find(|session| {
                            session.pid == waiter_pid && session.blockers.contains(&blocker_pid)
                        })
                        .cloned()
                });
                if observed_waiter.is_some() {
                    break;
                }
            }
        }
        // Always release the lock before asserting the observed result.
        sqlx::query("ROLLBACK")
            .execute(&mut blocker)
            .await
            .expect("release fixture blocker");
        let (waiter, result) = tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("waiter must unblock")
            .expect("waiter task");
        result.expect("waiter completes after rollback");
        let observed_waiter = observed_waiter.expect("real blocker relationship is visible");
        assert_eq!(observed_waiter.wait_event_type.as_deref(), Some("Lock"));
        assert!(observed_waiter.transaction_age_ms.is_some());
        assert!(observed_waiter.query_age_ms.is_some());
        drop(waiter);
        let unblocked = collector.collect().await.expect("unblocked observation");
        assert!(
            unblocked
                .activity
                .available()
                .expect("activity remains available")
                .iter()
                .all(|session| session.pid != waiter_pid || session.blockers.is_empty())
        );
    }

    #[tokio::test]
    #[ignore = "requires a disposable PostgreSQL 16-18 fixture and PGTRAIL_LIVE_TEST=1"]
    async fn live_superuser_and_missing_extension_and_restricted_role() {
        let (admin_options, monitor_dsn) = fixture_configuration();
        let mut admin = fixture_admin(&admin_options).await;
        let admin_password = std::env::var("PGTRAIL_ADMIN_PASSWORD")
            .unwrap_or_else(|_| "pgtrail-local-admin".to_owned());
        let admin_dsn = fixture_dsn(
            admin_options.get_port(),
            "pgtrail_admin",
            &admin_password,
            "pgtrail_dev",
        );
        let collector =
            Collector::new(&admin_dsn, false, Duration::from_secs(5)).expect("admin configuration");
        assert_eq!(
            collector.collect().await.expect_err("refuse superuser"),
            CollectorError::Superuser
        );
        collector.pool.close().await;

        let invalid_dsn = fixture_dsn(
            admin_options.get_port(),
            "pgtrail_monitor",
            "deliberately-incorrect-fixture-password",
            "pgtrail_dev",
        );
        let invalid = Collector::new(&invalid_dsn, false, Duration::from_secs(5))
            .expect("parse invalid credentials");
        assert_eq!(
            invalid
                .collect()
                .await
                .expect_err("reject invalid password"),
            CollectorError::Authentication
        );
        invalid.pool.close().await;

        let suffix = std::process::id();
        let database = format!("pgtrail_test_no_extension_{suffix}");
        let role = format!("pgtrail_test_limited_{suffix}");
        sqlx::query(AssertSqlSafe(format!(
            "CREATE DATABASE {}",
            quote_identifier(&database)
        )))
        .execute(&mut admin)
        .await
        .expect("create isolated extension-free fixture database");
        let no_extension_dsn = monitor_dsn.replace("/pgtrail_dev?", &format!("/{database}?"));
        let no_extension = Collector::new(&no_extension_dsn, false, Duration::from_secs(5))
            .expect("extension-free monitoring configuration");
        let missing_snapshot = no_extension.collect().await;
        no_extension.pool.close().await;
        sqlx::query(AssertSqlSafe(format!(
            "DROP DATABASE {}",
            quote_identifier(&database)
        )))
        .execute(&mut admin)
        .await
        .expect("remove isolated fixture database");
        let missing_snapshot = missing_snapshot.expect("missing extension preserves activity");
        assert!(missing_snapshot.activity.available().is_some());
        assert!(matches!(
            missing_snapshot.statements,
            Observation::Unavailable(reason) if reason.contains("not installed")
        ));
        sqlx::query(AssertSqlSafe(format!(
            "CREATE ROLE {} LOGIN PASSWORD 'pgtrail-fixture-limited' NOSUPERUSER",
            quote_identifier(&role)
        )))
        .execute(&mut admin)
        .await
        .expect("create restricted fixture role");
        sqlx::query(AssertSqlSafe(format!(
            "GRANT CONNECT ON DATABASE pgtrail_dev TO {}",
            quote_identifier(&role)
        )))
        .execute(&mut admin)
        .await
        .expect("allow restricted fixture connection");
        let limited_dsn = fixture_dsn(
            admin_options.get_port(),
            &role,
            "pgtrail-fixture-limited",
            "pgtrail_dev",
        );
        let limited = Collector::new(&limited_dsn, false, Duration::from_secs(5))
            .expect("restricted fixture configuration");
        let limited_snapshot = limited.collect().await;
        limited.pool.close().await;
        sqlx::query(AssertSqlSafe(format!(
            "REVOKE CONNECT ON DATABASE pgtrail_dev FROM {}",
            quote_identifier(&role)
        )))
        .execute(&mut admin)
        .await
        .expect("revoke restricted fixture grant");
        sqlx::query(AssertSqlSafe(format!(
            "DROP ROLE {}",
            quote_identifier(&role)
        )))
        .execute(&mut admin)
        .await
        .expect("remove restricted fixture role");
        let limited_snapshot = limited_snapshot.expect("restricted activity remains available");
        assert!(limited_snapshot.activity.available().is_some());
        assert!(matches!(
            limited_snapshot.statements,
            Observation::Unavailable(reason) if reason.contains("restricted")
        ));
        assert!(limited_snapshot.health.database.available().is_some());
        assert!(matches!(
            limited_snapshot.health.replication,
            Observation::Unavailable(reason) if reason.contains("restricted")
        ));
        assert!(matches!(
            limited_snapshot.health.vacuum,
            Observation::Unavailable(reason) if reason.contains("restricted")
        ));
        assert!(
            limited_snapshot
                .warnings
                .iter()
                .any(|warning| warning.contains("restricted"))
        );
    }

    #[tokio::test]
    #[ignore = "requires a disposable PostgreSQL 16-18 fixture and PGTRAIL_LIVE_TEST=1"]
    async fn live_health_permission_failure_preserves_other_sections() {
        let (admin_options, monitor_dsn) = fixture_configuration();
        let mut admin = fixture_admin(&admin_options).await;
        let database = format!("pgtrail_test_health_denied_{}", std::process::id());
        sqlx::query(AssertSqlSafe(format!(
            "CREATE DATABASE {}",
            quote_identifier(&database)
        )))
        .execute(&mut admin)
        .await
        .expect("create isolated health-permission fixture database");
        let mut isolated = admin_options
            .clone()
            .database(&database)
            .connect()
            .await
            .expect("connect to isolated fixture database");
        // Catalog view ACLs are database-local; no other live test loses access.
        sqlx::query("REVOKE SELECT ON pg_catalog.pg_stat_wal FROM PUBLIC")
            .execute(&mut isolated)
            .await
            .expect("restrict one isolated statistics view");
        isolated
            .close()
            .await
            .expect("close isolated fixture admin");
        let dsn = monitor_dsn.replace("/pgtrail_dev?", &format!("/{database}?"));
        let collector =
            Collector::new(&dsn, false, Duration::from_secs(5)).expect("isolated health collector");
        let snapshot = collector.collect().await;
        collector.pool.close().await;
        sqlx::query(AssertSqlSafe(format!(
            "DROP DATABASE {}",
            quote_identifier(&database)
        )))
        .execute(&mut admin)
        .await
        .expect("remove isolated fixture database");
        let snapshot = snapshot.expect("health denial must not abort collection");
        assert!(snapshot.activity.available().is_some());
        assert!(snapshot.health.database.available().is_some());
        assert!(snapshot.health.tables.available().is_some());
        assert!(matches!(snapshot.health.wal,
            Observation::Unavailable(reason) if reason.contains("permission denied")
        ));
        assert!(
            snapshot.health.io.available().is_some(),
            "query after denied section succeeds"
        );
        assert!(snapshot.health.vacuum.available().is_some());
    }

    #[tokio::test]
    #[ignore = "requires a disposable PostgreSQL 16-18 fixture and PGTRAIL_LIVE_TEST=1"]
    async fn live_relation_candidates_are_bounded_and_truncation_is_explicit() {
        let (admin_options, monitor_dsn) = fixture_configuration();
        let mut admin = fixture_admin(&admin_options).await;
        let database = format!("pgtrail_test_many_relations_{}", std::process::id());
        sqlx::query(AssertSqlSafe(format!(
            "CREATE DATABASE {}",
            quote_identifier(&database)
        )))
        .execute(&mut admin)
        .await
        .expect("create isolated many-relations fixture database");
        let mut isolated = admin_options
            .clone()
            .database(&database)
            .connect()
            .await
            .expect("connect to isolated fixture database");
        // Build more relations than the observation limit without affecting the
        // normal fixture's health or concurrent collector integration tests.
        sqlx::query(
            "DO $$ BEGIN FOR i IN 1..1002 LOOP
                EXECUTE format('CREATE TABLE public.pgtrail_small_%s (id integer PRIMARY KEY)', i);
             END LOOP; END $$",
        )
        .execute(&mut isolated)
        .await
        .expect("create bounded-sampling fixture tables");
        sqlx::query("CREATE TABLE public.pgtrail_large (id integer PRIMARY KEY)")
            .execute(&mut isolated)
            .await
            .expect("create large fixture table");
        sqlx::query("INSERT INTO public.pgtrail_large SELECT generate_series(1, 10000)")
            .execute(&mut isolated)
            .await
            .expect("populate large fixture table");
        sqlx::query("ANALYZE public.pgtrail_large")
            .execute(&mut isolated)
            .await
            .expect("update fixture catalog size estimates");
        isolated
            .close()
            .await
            .expect("close isolated fixture admin");
        let dsn = monitor_dsn.replace("/pgtrail_dev?", &format!("/{database}?"));
        let collector =
            Collector::new(&dsn, false, Duration::from_secs(5)).expect("many-relations collector");
        let snapshot = collector.collect().await;
        collector.pool.close().await;
        sqlx::query(AssertSqlSafe(format!(
            "DROP DATABASE {}",
            quote_identifier(&database)
        )))
        .execute(&mut admin)
        .await
        .expect("remove many-relations fixture database");
        let snapshot = snapshot.expect("bounded relation observation");
        let relations = snapshot
            .health
            .tables
            .available()
            .expect("relation statistics");
        assert!(relations.truncated);
        assert_eq!(relations.tables.len(), 1000);
        assert_eq!(relations.indexes.len(), 1000);
        assert_eq!(relations.tables[0].name, "pgtrail_large");
        assert_eq!(relations.indexes[0].name, "pgtrail_large_pkey");
        assert!(relations.tables[0].table_bytes > 0);
        assert_eq!(
            relations.tables[0].total_bytes,
            relations.tables[0].table_bytes + relations.tables[0].index_bytes
        );
        assert!(snapshot.warnings.iter().any(|warning| {
            warning.contains("catalog size estimates") && warning.contains("omitted")
        }));
    }
}
