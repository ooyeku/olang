# climate — a real data-science workstream

An end-to-end analysis of global CO2 emissions and the energy
transition, on real data downloaded as part of the run. Nothing is
bundled: the first stage fetches Our World in Data's maintained
datasets (plain CSV, ~24 MB total), caches them under `data/`, and
every later stage works from that local copy.

    olang run main.ol

The run is self-contained and re-runnable: downloads are cached (and
re-fetched if a partial file is detected), all outputs regenerate into
`out/`, and neither directory is tracked by git.

## The workstream

1. **fetch** — both datasets, with retries, size sanity checks, and
   atomic writes (a partial download can never be mistaken for a
   cached file).
2. **load** — `ods.read_csv_file` on both files; schema and row counts
   reported.
3. **prep** — narrow to the analysis columns, split country rows from
   aggregate rows (World, EU 27, international transport), derive
   per-capita series.
4. **global trajectory** — world CO2 by year, its peak, and a rolling
   decade of growth rates.
5. **top emitters** — total vs per-capita emissions for the latest
   year, and the share of the global total the top ten hold.
6. **decoupling** — countries whose GDP grew over the last measured
   decade while CO2 fell: the absolute-decoupling list.
7. **energy transition** — renewables' share of primary energy, the
   biggest movers since 2000.
8. **relationships** — a cross-sectional join of both datasets on
   country: correlation matrix, a linear model of per-capita CO2
   against per-capita energy use, and a t-test comparing emissions of
   richer and poorer halves.
9. **report** — everything lands in `out/report.md` with SVG charts
   and derived CSVs beside it.

## Data

- [owid/co2-data](https://github.com/owid/co2-data) — CO2 and
  greenhouse-gas emissions, one row per country-year.
- [owid/energy-data](https://github.com/owid/energy-data) — energy
  consumption, mix, and intensity, one row per country-year.

Both are maintained by Our World in Data and published under
Creative Commons BY.
