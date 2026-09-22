#!/bin/sh
# Validate only the project's disposable Compose database, never a supplied DSN.
set -eu
cd "$(dirname "$0")/.."

monitor_psql() {
    docker compose exec -T postgres sh -c '
        export PGPASSWORD="$PGTRAIL_MONITOR_PASSWORD"
        exec psql -X -h 127.0.0.1 -U pgtrail_monitor -d pgtrail_dev \
            --set=ON_ERROR_STOP=1 --set=VERBOSITY=verbose "$@"
    ' sh "$@"
}

result=$(monitor_psql -At <<'SQL'
SELECT current_user = 'pgtrail_monitor'
    AND NOT (SELECT rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls
             FROM pg_roles WHERE rolname = current_user)
    AND pg_has_role(current_user, 'pg_read_all_stats', 'USAGE')
    AND current_setting('default_transaction_read_only') = 'on'
    AND EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'pg_stat_statements')
    AND EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname = current_database());
SELECT count(*) >= 0 FROM pg_stat_statements;
SQL
)
if [ "$result" != "$(printf 't\nt')" ]; then
    printf '%s\n' "Unexpected monitoring capabilities: $result" >&2
    exit 1
fi

# READ WRITE bypasses the default read-only setting: grants must still deny writes.
if output=$(monitor_psql -c 'BEGIN READ WRITE; INSERT INTO public.pgtrail_write_probe VALUES (2); COMMIT;' 2>&1); then
    printf '%s\n' 'Monitoring role unexpectedly wrote application data.' >&2
    exit 1
fi
case "$output" in
    *'42501:'*'permission denied for table pgtrail_write_probe'*) ;;
    *) printf '%s\n' "Unexpected write failure: $output" >&2; exit 1 ;;
esac

if output=$(monitor_psql -c 'BEGIN READ WRITE; CREATE TABLE public.pgtrail_forbidden (id integer); COMMIT;' 2>&1); then
    printf '%s\n' 'Monitoring role unexpectedly created a table.' >&2
    exit 1
fi
case "$output" in
    *'42501:'*'permission denied for schema public'*) ;;
    *) printf '%s\n' "Unexpected schema failure: $output" >&2; exit 1 ;;
esac

printf '%s\n' 'PostgreSQL monitoring checks passed; table writes and schema creation are denied.'
