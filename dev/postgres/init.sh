#!/bin/sh
set -eu

psql --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" \
    --set=ON_ERROR_STOP=1 --set=monitor_password="$PGTRAIL_MONITOR_PASSWORD" <<'SQL'
CREATE EXTENSION pg_stat_statements;

CREATE ROLE pgtrail_monitor LOGIN PASSWORD :'monitor_password'
    NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
GRANT pg_read_all_stats TO pgtrail_monitor;
REVOKE ALL ON DATABASE pgtrail_dev FROM PUBLIC;
GRANT CONNECT ON DATABASE pgtrail_dev TO pgtrail_monitor;
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
GRANT USAGE ON SCHEMA public TO pgtrail_monitor;
ALTER ROLE pgtrail_monitor SET default_transaction_read_only = on;
ALTER ROLE pgtrail_monitor SET statement_timeout = '5s';

-- A disposable fixture for checking that monitoring cannot write application data.
CREATE TABLE public.pgtrail_write_probe (id integer PRIMARY KEY);
INSERT INTO public.pgtrail_write_probe VALUES (1);
SQL
