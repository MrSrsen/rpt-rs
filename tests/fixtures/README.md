# Test fixtures

Public Crystal Reports `.rpt` files and their XML baselines, used by the regression test in
`crates/rpt-to-xml/tests/baseline.rs`.

- `reports/` — the `.rpt` fixtures.
- `baselines/` — the expected XML output for each report (`<name>.xml`).

The test exports each report with `rpt-to-xml` inside a [Bubblewrap](https://github.com/containers/bubblewrap) sandbox,
with the report bind-mounted at a fixed path, so path-derived attributes are identical on every machine and the
comparison is deterministic. The output must match the baseline exactly.

Regenerate the baselines after an intentional change to the XML output:

```sh
RPT_BLESS=1 cargo test -p rpt-to-xml --test baseline
```

## Sources and attribution

These are publicly available sample reports. All rights remain with their respective authors; they are included here
only as test fixtures.

| Prefix           | Source                                                                                                   |
| ---------------- | -------------------------------------------------------------------------------------------------------- |
| `ajryan_*`       | [ajryan/RptToXml](https://github.com/ajryan/RptToXml) — SAP Business One sample reports.                 |
| `worrall_*`      | [worrallbrian/crystal_reports](https://github.com/worrallbrian/crystal_reports)                          |
| `benbrahim777_*` | [benbrahim777/Crystal-Reports](https://github.com/benbrahim777/Crystal-Reports) — Xtreme sample reports. |
