# Test fixtures

Public Crystal Reports `.rpt` files and their baselines, used by two regression tests: the XML
exporter baseline (`crates/rpt-cli/tests/baseline.rs`) and the data-driven HTML render baseline
(`crates/rpt-render/tests/postgres_fixtures.rs`).

- `reports/` — the `.rpt` fixtures.
- `baselines/xml/` — the committed XML export baselines (`<group>/<name>.xml`).
- `baselines/html/` — the committed HTML render baselines (`<group>/<name>.html`), one per report seeded
  from `sql/<group>/`. `baselines/html/private/` holds gitignored baselines for private reports.
- `sql/` — SQL migrations (schema + synthetic seed) for the data-driven render test; see `sql/README.md`.

The test exports each report with `rpt xml-dump` inside a [Bubblewrap](https://github.com/containers/bubblewrap) sandbox,
with the report bind-mounted at a fixed path, so path-derived attributes are identical on every machine and the
comparison is deterministic. The output must match the baseline exactly.

Regenerate the baselines after an intentional change to the XML output:

```sh
RPT_BLESS=1 cargo test -p rpt-cli --test baseline
```

## Sources and attribution

These are publicly available sample reports. All rights remain with their respective authors; they are included here
only as test fixtures.

| Prefix           | Source                                                                                                   |
| ---------------- | -------------------------------------------------------------------------------------------------------- |
| `ajryan_*`       | [ajryan/RptToXml](https://github.com/ajryan/RptToXml) — SAP Business One sample reports.                 |
| `worrall_*`      | [worrallbrian/crystal_reports](https://github.com/worrallbrian/crystal_reports)                          |
| `benbrahim777_*` | [benbrahim777/Crystal-Reports](https://github.com/benbrahim777/Crystal-Reports) — Xtreme sample reports. |
