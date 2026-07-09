# Convenience targets for the render-parity test corpus. The data-driven render tests
# (crates/rpt-render/tests/postgres_fixtures.rs) seed committed SQL fixtures into a PostgreSQL server
# and diff the rendered HTML against committed baselines. PostgreSQL is the single DB technology for
# render testing (see docker-compose.yml). The database is provisioned by docker compose.

RPT_DB_PORT ?= 55432
RPT_DB_URL  ?= postgres://rpt:rpt@localhost:$(RPT_DB_PORT)/rptfixtures
export RPT_DB_URL

.PHONY: db-up db-down test-fixtures bless-fixtures test-fixtures-clean

## Start the test PostgreSQL (blocks until healthy).
db-up:
	docker compose up -d --wait

## Stop and discard the test PostgreSQL (data is ephemeral).
db-down:
	docker compose down

## Run the render-parity corpus against the running PostgreSQL.
test-fixtures:
	cargo test -p rpt-render --test postgres_fixtures

## Regenerate the committed HTML baselines from the current render.
bless-fixtures:
	RPT_BLESS=1 cargo test -p rpt-render --test postgres_fixtures

## One-shot: bring the DB up, run the corpus, tear it down.
test-fixtures-clean: db-up
	$(MAKE) test-fixtures; status=$$?; docker compose down; exit $$status
