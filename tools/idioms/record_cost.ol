let N = 200000
fn t(label, f) = { let t0 = time.monotonic(); let r = f(); println(label, math.round(time.monotonic() - t0)); r }
let M = #{ "op": "line", "x1": 1.0, "y1": 0.0, "x2": 2.0, "y2": 50.0, "color": "#888", "width": 1.0, "key": "k" }
let R = { op: "line", x1: 1.0, y1: 0.0, x2: 2.0, y2: 50.0, color: "#888", width: 1.0, key: "k" }
let K = ["op", "x1", "y2", "color"]
fn pair(label, fm, fr) = { t(label + " map   ", fm); t(label + " record", fr) }
pair("literal        ", () => { let mut s = 0; for i in 0..N { let v = #{ "op": "line", "x1": i, "y1": 0.0, "x2": 2.0, "y2": 50.0, "color": "#888", "width": 1.0 }; s = s + v.x1 }; s },
                       () => { let mut s = 0; for i in 0..N { let v = { op: "line", x1: i, y1: 0.0, x2: 2.0, y2: 50.0, color: "#888", width: 1.0 }; s = s + v.x1 }; s })
pair("field read     ", () => { let mut s = 0.0; for i in 0..N { s = s + M.x1 + M.y2 }; s }, () => { let mut s = 0.0; for i in 0..N { s = s + R.x1 + R.y2 }; s })
pair("map_get dyn key", () => { let mut s = 0; for i in 0..N { if map_get(M, K[i % 4]) != () => { s = s + 1 } }; s }, () => { let mut s = 0; for i in 0..N { if map_get(R, K[i % 4]) != () => { s = s + 1 } }; s })
pair("index dyn key  ", () => { let mut s = 0; for i in 0..N { if M[K[i % 4]] != () => { s = s + 1 } }; s }, () => { let mut s = 0; for i in 0..N { if R[K[i % 4]] != () => { s = s + 1 } }; s })
pair("map_set one-off", () => { let mut s = 0.0; for i in 0..N { s = s + map_set(M, "x1", 3.0).x1 }; s }, () => { let mut s = 0.0; for i in 0..N { s = s + map_set(R, "x1", 3.0).x1 }; s })
pair("spread same    ", () => { let mut s = 0.0; for i in 0..N { s = s + (#{ ...M, "x1": 3.0 }).x1 }; s }, () => { let mut s = 0.0; for i in 0..N { s = s + ({ ...R, x1: 3.0 }).x1 }; s })
pair("spread as map  ", () => { let mut s = 0.0; for i in 0..N { s = s + (#{ ...M, "x1": 3.0 }).x1 }; s }, () => { let mut s = 0.0; for i in 0..N { s = s + (#{ ...R, "x1": 3.0 }).x1 }; s })
pair("map_has_key    ", () => { let mut s = 0; for i in 0..N { if map_has_key(M, "y2") => { s = s + 1 } }; s }, () => { let mut s = 0; for i in 0..N { if map_has_key(R, "y2") => { s = s + 1 } }; s })
pair("map_keys       ", () => { let mut s = 0; for i in 0..N { s = s + len(map_keys(M)) }; s }, () => { let mut s = 0; for i in 0..N { s = s + len(map_keys(R)) }; s })
pair("map_merge      ", () => { let mut s = 0.0; for i in 0..N { s = s + map_merge(M, M).x1 }; s }, () => { let mut s = 0.0; for i in 0..N { s = s + map_merge(R, R).x1 }; s })
pair("?? on missing  ", () => { let mut s = 0; for i in 0..N { s = s + (M.zz ?? 1) }; s }, () => { let mut s = 0; for i in 0..N { s = s + (R.zz ?? 1) }; s })
pair("== equal       ", () => { let mut s = 0; let m2 = map_set(M, "op", "line"); for i in 0..N { if M == m2 => { s = s + 1 } }; s }, () => { let mut s = 0; let r2 = map_set(R, "op", "line"); for i in 0..N { if R == r2 => { s = s + 1 } }; s })
pair("pass to fn+read", () => { let f = (v) => v.x1 + v.x2; let mut s = 0.0; for i in 0..N { s = s + f(M) }; s }, () => { let f = (v) => v.x1 + v.x2; let mut s = 0.0; for i in 0..N { s = s + f(R) }; s })
pair("list of 1000 .f", () => { let xs = map(range(0, 1000), (i) => map_set(M, "x1", to_float(i))); let mut s = 0.0; for j in 0..200 { for v in xs { s = s + v.x1 } }; s }, () => { let xs = map(range(0, 1000), (i) => map_set(R, "x1", to_float(i))); let mut s = 0.0; for j in 0..200 { for v in xs { s = s + v.x1 } }; s })
