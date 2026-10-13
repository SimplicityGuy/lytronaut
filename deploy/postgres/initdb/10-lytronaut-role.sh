#!/bin/sh
# Runs once, inside the official image, when the PostgreSQL data directory is
# first initialized. Creates the restricted application role and its schema.
# The generated password is read by psql itself (\set with a backtick command),
# so it never appears in a process argument, environment variable or SQL echo,
# and error statements are kept out of the server log for this session.
set -eu
test -s /run/secrets/lytronaut_database_password
psql --no-psqlrc --quiet -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" <<'SQL'
SET log_min_error_statement = panic;
SET log_statement = none;
\set lytronaut_password `head -n 1 /run/secrets/lytronaut_database_password`
CREATE ROLE lytronaut LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD :'lytronaut_password';
\unset lytronaut_password
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
REVOKE ALL ON DATABASE lytronaut FROM PUBLIC;
-- CREATE on the database lets the application create its isolated restore schemas.
GRANT CONNECT, CREATE, TEMPORARY ON DATABASE lytronaut TO lytronaut;
CREATE SCHEMA lytronaut AUTHORIZATION lytronaut;
SQL
