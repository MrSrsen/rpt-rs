#!/bin/sh
# Auto-seed the permanent "meridian" database on container init.
#
# The compose data dir is a tmpfs, so /docker-entrypoint-initdb.d runs on every `up` from empty.
# The postgres entrypoint only runs *.sql against $POSTGRES_DB (rptfixtures); the meridian corpus
# lives in its own database so a DSN bound to it sees only meridian tables. Hence this shell
# wrapper: create the DB, then load the seed into it. The seed itself is mounted read-only at
# /opt/meridian/meridian.sql (NOT under initdb.d, so the entrypoint does not also run it against
# rptfixtures).
set -e

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname postgres <<-EOSQL
	CREATE DATABASE meridian OWNER $POSTGRES_USER;
EOSQL

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname meridian -q -f /opt/meridian/meridian.sql

echo "seeded meridian database from /opt/meridian/meridian.sql"

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname meridian -q -f /opt/meridian/25-meridian-views.sql

echo "created meridian helper views from 25-meridian-views.sql"
