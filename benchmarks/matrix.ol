// Matrices: products, the Gram matrix, k×k routines, distances and
// correlation, as a program calls them: milliseconds per call, the
// fastest of seven. matrix_numpy.py builds the same data and times
// NumPy, SciPy and pandas on it:
//
//   olang benchmarks/matrix.ol
//   python3 benchmarks/matrix_numpy.py
//   OLANG_PARALLELISM=1 olang benchmarks/matrix.ol

fn best(f) = {
    f()
    let mut t = 1e18
    for round in 0..7 {
        let t0 = time.monotonic()
        f()
        let ms = time.monotonic() - t0
        if ms < t => { t = ms }
    }
    t
}

fn row(name, ms) = println(`${str.pad_end(name, 45, " ")} ${str.pad_start(str.fixed(ms, 3), 10, " ")} ms`)

// k columns of n rows: col_j[i] = sin(0.001 (j + 1) i) + ((i (j + 7)) % 97) / 100
fn columns(n, k) = {
    let i = ods.arange(n)
    ods.frame(map(range(0, k), (j) => [`x${j}`, (i * (0.001 * to_float(j + 1))).sin() + ((i * (j + 7)) % 97) * 0.01]))
}

let n = 1000000
let f = columns(n, 10)
let names = ods.columns(f)
let X = ods.matrix(f, names)
let b = map(range(0, 10), (j) => to_float(j) * 0.1 - 0.4)
let y = f["x0"] * 2.0 - f["x3"]
let B = mat.of(map(range(0, 10), (j) => [to_float(j), 1.0, -to_float(j)]))
let g20 = ods.matrix(columns(2000, 20)).gram()
let g100 = ods.matrix(columns(5000, 100)).gram()
let pts = ods.matrix(columns(10000, 10))
let centres = ods.matrix(columns(8, 10))
let sample = ods.matrix(columns(2000, 10))
let small = columns(100000, 10)

println(`threads: ${os.env("OLANG_PARALLELISM") ?? "all"}, n = ${n}`)
row("ods.matrix(f, 10 columns) (no copy)", best(() => ods.matrix(f, names)))
row("X @ b (1M x 10)", best(() => X @ b))
row("X @ B (1M x 10 @ 10 x 3)", best(() => X @ B))
row("X.t() @ y (1M x 10)", best(() => X.t() @ y))
row("X.gram() (1M x 10)", best(() => X.gram()))
row("mat.eigh 20 x 20", best(() => mat.eigh(g20)))
row("mat.eigh 100 x 100", best(() => mat.eigh(g100)))
row("mat.solve 100 x 100", best(() => mat.solve(g100, map(range(0, 100), (j) => 1.0))))
row("mat.inv 100 x 100", best(() => mat.inv(g100)))
row("mat.cdist 10,000 x 8 (k = 10)", best(() => mat.cdist(pts, centres)))
row("mat.cdist 2,000 x 2,000 euclidean (k = 10)", best(() => mat.cdist(sample, sample, metric: "euclidean")))
row("ods.corr pearson (1M x 10)", best(() => ods.corr(f)))
row("ods.cov (1M x 10)", best(() => ods.cov(f)))
row("ods.corr spearman (100k x 10)", best(() => ods.corr(small, method: "spearman")))

// A formula fit: a million rows, two numbers, a square and 72 cells of
// kind × hour (71 columns), encoded and solved.
let g = f.with({ kind: ods.cast(ods.arange(n) % 5 % 3, "String"), hour: ods.arange(n) % 24, y: f.x0 * 2.0 - f.x1 + f.x2 })
row("learn.linear formula (1M, 75 columns)", best(() => learn.linear(g, "y ~ x0 + x1 + I(x0^2) + C(kind):C(hour)")))
