# The pipeline benchmark

An end-to-end ETL pass — read a 1M-row CSV, clean, derive, filter,
group, join, sort, window, write — implemented three times with
identical semantics: in olang on the ods data stack
([pipeline.ol](pipeline.ol)), in pandas
([pipeline_pandas.py](pipeline_pandas.py)), and in Polars
([pipeline_polars.py](pipeline_polars.py)). This is the DP4 benchmark
from the roadmap; the published results and methodology live in
[docs/ods.md](../docs/ods.md#the-pipeline-benchmark).

Every engine prints one `STAGE <name> <ms> <checksum>` line per stage.
The runner executes each engine several times and **refuses the result
if any checksum disagrees** across engines or repetitions — timings are
only ever reported for byte-identical answers.

## Run

```bash
# a Python with pandas and polars (any venv works)
python3 -m venv .venv && .venv/bin/pip install pandas polars

# five repetitions over a generated 1M-row dataset
PYTHON=.venv/bin/python benchmarks/run.sh
```

`run.sh [reps] [rows]` controls repetitions and dataset size; `OLANG=`
selects the binary (defaults to `olang` on PATH). The dataset is
generated deterministically by [gen_data.ol](gen_data.ol) (seeded, so
every engine on every machine reads the same bytes for a given row
count) and is not committed.

## The stages

| Stage | Work |
|---|---|
| load | read `sales.csv` (id, date, region, category, units, price; 2% null prices) |
| clean | drop null-price rows; derive `revenue = units × price` |
| filter | keep bulk orders (`units >= 5`) |
| group | revenue sum, mean units, order count per region × category |
| join | attach each region's manager from the dimension table |
| sort | order cells by revenue, descending |
| daily | revenue per day, 7-day trailing mean over the calendar |
| write | the ranked summary back to CSV |

Each engine runs as it ships: ods under the language's automatic
parallelism policy, pandas single-threaded, Polars on its default
thread pool. Elapsed time is measured inside each process per stage, so
interpreter and import startup are excluded from every engine equally.

## The record pipeline

[records.ol](records.ol) is the other shape an application's query layer
runs: `filter → map → sort` over 2,000 record maps, with a few `map_get`s
and a string compare per row. It prints one `run over 2000: <ms>` line
and is the benchmark roadmap W15 (collection pipelines) measures against
Node; `olang bench benchmarks/records.ol --save base.json` pins it, and
`olang bench --in-task` runs it as an http worker would.
