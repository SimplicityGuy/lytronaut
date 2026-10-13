#!/bin/sh
# Runs once, inside the official image, when the PostgreSQL data directory is
# first initialized. Creates the restricted application role and its schema.
#
# The server reads the secret itself (pg_read_file, as the bootstrap superuser),
# so it never appears in a process argument, environment variable or SQL text,
# and error statements are kept out of the server log for this session. It is
# validated exactly like the application's LYTRONAUT_DATABASE_PASSWORD_FILE
# reader: valid UTF-8 of at most 1026 bytes; one trailing LF, or CRLF, removed;
# then 1-1024 bytes with no control character (Unicode Cc: U+0000-U+001F and
# U+007F-U+009F, including multi-byte C1 controls).
set -eu
test -r /run/secrets/lytronaut_database_password
psql --no-psqlrc --quiet -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" <<'SQL'
SET log_min_error_statement = panic;
SET log_statement = none;
DO $bootstrap$
DECLARE
    secret text := pg_catalog.pg_read_file('/run/secrets/lytronaut_database_password');
BEGIN
    IF octet_length(secret) > 1026 THEN
        RAISE EXCEPTION 'lytronaut_database_password must be one line of 1-1024 bytes without control characters';
    END IF;
    IF right(secret, 1) = E'\n' THEN
        secret := left(secret, -1);
        IF right(secret, 1) = E'\r' THEN
            secret := left(secret, -1);
        END IF;
    END IF;
    IF octet_length(secret) NOT BETWEEN 1 AND 1024 OR secret ~ '[\x01-\x1f\x7f-\x9f]' THEN
        RAISE EXCEPTION 'lytronaut_database_password must be one line of 1-1024 bytes without control characters';
    END IF;
    EXECUTE format('CREATE ROLE lytronaut LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD %L', secret);
END
$bootstrap$;
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
REVOKE ALL ON DATABASE lytronaut FROM PUBLIC;
-- CREATE on the database lets the application create its isolated restore schemas.
GRANT CONNECT, CREATE, TEMPORARY ON DATABASE lytronaut TO lytronaut;
CREATE SCHEMA lytronaut AUTHORIZATION lytronaut;
SQL
