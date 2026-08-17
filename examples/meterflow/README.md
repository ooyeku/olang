# meterflow

A multi-source ETL on the ods data stack. Electricity meter telemetry
arrives as JSON lines, one object per reading, in a file too large to
hold; two CSV dimension tables say which site each meter belongs to and
what its tariff costs. The job cleans the readings, aggregates them,
prices them, and writes a daily report with charts.

```bash
olang main.ol                             # the checked-in sample
olang gen.ol 400000 data/big.jsonl        # a larger input, kept out of git
olang main.ol data/big.jsonl 5000         # run it, choosing the chunk size
olang test .                              # the pipeline's own invariants
```

## What it exercises

Every stage is one a real pipeline has, and each maps onto part of the
stack:

| stage | verbs |
|---|---|
| bounded-memory ingest | `open_jsonl` · `next_chunk` · `rows_read` |
| quality gate | `all_of` · `eq` · comparison masks · subscript |
| aggregation across chunks | `group_by` · `concat` |
| dimension joins | `join` |
| derived measures | column arithmetic · `with_column` |
| rollups | `group_by` · `sort_by` |
| cached artifact | `write_frame` · `read_frame` · `frame_info` |
| output | `plot.line` · `plot.bar` · `write_csv` · `json.stringify` |

## The property that matters

Streaming turns one pass into many partial aggregates, so the pipeline is
only correct if the chunk boundaries do not change the answer. They do
not: over 400,000 readings, the streamed run and a whole-file computation
agree exactly — 390,033 rows kept and 781,262.26 kWh either way, across
80 chunk boundaries.

`olang test .` checks the same property from the inside: every reading is
either rejected or counted, with none lost in between.

## Files

```text
main.ol           the pipeline
gen.ol            seeded input generator, any size
data/meters.csv   meter -> site, tariff
data/tariffs.csv  tariff -> rate, standing charge
data/readings.jsonl  the fact table (2,000-row sample, checked in)
```
