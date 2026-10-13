#!/bin/sh
# Runs once, inside the official image, when the PostgreSQL data directory is
# first initialized. Creates the restricted application role and its schema.
#
# The secret is normalized exactly like the application's
# LYTRONAUT_DATABASE_PASSWORD_FILE reader: one trailing LF, or CRLF, is removed;
# the result must be 1-1024 bytes without control characters. psql reads the
# normalized value itself (\set with a backtick command on a private temporary
# file), so it never appears in a process argument, environment variable or
# SQL echo, and error statements are kept out of the server log for this session.
set -eu
secret=/run/secrets/lytronaut_database_password
lf=$(printf '\nx')
lf=${lf%x}
cr=$(printf '\r')
password=$(cat "$secret"; printf x)
password=${password%x}
case $password in
*"$lf")
	password=${password%"$lf"}
	case $password in *"$cr") password=${password%"$cr"} ;; esac
	;;
esac
length=$(printf %s "$password" | wc -c)
case $password in *"$lf"*) length=0 ;; esac # an embedded line break is a control character
if [ "$length" -lt 1 ] || [ "$length" -gt 1024 ] ||
	printf %s "$password" | LC_ALL=C grep -q '[[:cntrl:]]'; then
	echo "lytronaut_database_password must be one line of 1-1024 bytes without control characters" >&2
	exit 1
fi
umask 077
normalized=$(mktemp)
trap 'rm -f "$normalized"' EXIT
printf %s "$password" >"$normalized"
unset password
psql --no-psqlrc --quiet -v ON_ERROR_STOP=1 -v secret_file="$normalized" \
	--username "$POSTGRES_USER" --dbname "$POSTGRES_DB" <<'SQL'
SET log_min_error_statement = panic;
SET log_statement = none;
\set lytronaut_password `cat :'secret_file'`
CREATE ROLE lytronaut LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD :'lytronaut_password';
\unset lytronaut_password
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
REVOKE ALL ON DATABASE lytronaut FROM PUBLIC;
-- CREATE on the database lets the application create its isolated restore schemas.
GRANT CONNECT, CREATE, TEMPORARY ON DATABASE lytronaut TO lytronaut;
CREATE SCHEMA lytronaut AUTHORIZATION lytronaut;
SQL
