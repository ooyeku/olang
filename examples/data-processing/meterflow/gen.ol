// gen.ol — generate the meterflow inputs.
//
// The checked-in data/ holds a small sample so the example runs
// instantly under the harness. This script writes an input of any size
// with the same schema, for reproducing the figures in the data-stack
// chapter (docs/ods.md):
//
//   olang gen.ol                          # 2000 readings -> data/readings.jsonl
//   olang gen.ol 2000000 data/big.jsonl   # a larger run, kept out of git
//
// The checked-in data/readings.jsonl is the small sample the harness
// runs; write large inputs to another path so a scale run never shows up
// as a modified file. Generation is seeded, so a given count always
// produces the same output.

let args = os.args()
let readings = if len(args) > 1 => unwrap(str.parse_int(args[1])) else => 2000
let out_path = if len(args) > 2 => args[2] else => "data/readings.jsonl"

random.seed(7)

let sites = ["harbour", "ridgeway", "old mill", "beacon hill"]
let tariffs = ["standard", "economy", "peak"]
let rates = [0.284, 0.191, 0.412]

// ── the dimension tables ──
let meter_count = 60
let mut meters = "meter,site,tariff,installed\n"
for i in 0..meter_count {
    meters = meters + "M" + str.pad_start(show(i), 4, "0")
        + "," + sites[i % len(sites)]
        + "," + tariffs[i % len(tariffs)]
        + ",2024-0" + show((i % 9) + 1) + "-01\n"
}
unwrap(fs.write_file("data/meters.csv", meters))

let mut tariff_csv = "tariff,rate,standing\n"
for (i, name) in enumerate(tariffs) {
    tariff_csv = tariff_csv + name + "," + show(rates[i]) + "," + show(0.42 + to_float(i) * 0.05) + "\n"
}
unwrap(fs.write_file("data/tariffs.csv", tariff_csv))

// ── the fact table ──
//
// Written as JSON lines because that is the shape telemetry arrives in,
// and large enough that the pipeline must stream it rather than hold it.
// Roughly one reading in forty is unusable — a failed sensor reports a
// null, and a miscalibrated one reports an impossible negative — because
// a pipeline that has never seen bad input is not a realistic one.
let days = 28
let mut out = ""
for i in 0..readings {
    let meter = "M" + str.pad_start(show(i % meter_count), 4, "0")
    let day = "2026-07-" + str.pad_start(show((i / meter_count) % days + 1), 2, "0")
    let base = 0.4 + random.random() * 3.2
    let hour = to_float(i % 24)
    // A daily shape: consumption peaks late afternoon.
    let kwh = math.round((base * (1.0 + 0.6 * math.sin((hour - 6.0) / 24.0 * 6.283))) * 1000.0) / 1000.0
    let roll = random.randint(0, 999)
    let record = if roll < 15 =>
            "{\"meter\": \"" + meter + "\", \"day\": \"" + day + "\", \"kwh\": null, \"quality\": \"fail\"}"
        else if roll < 25 =>
            "{\"meter\": \"" + meter + "\", \"day\": \"" + day + "\", \"kwh\": " + show(0.0 - kwh) + ", \"quality\": \"ok\"}"
        else =>
            "{\"meter\": \"" + meter + "\", \"day\": \"" + day + "\", \"kwh\": " + show(kwh) + ", \"quality\": \"ok\"}"
    out = out + record + "\n"
}
unwrap(fs.write_file(out_path, out))

println("wrote data/meters.csv (" + show(meter_count) + " meters)")
println("wrote data/tariffs.csv (" + show(len(tariffs)) + " tariffs)")
println("wrote " + out_path + " (" + show(readings) + " readings)")
