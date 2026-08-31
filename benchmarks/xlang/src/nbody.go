package main
import ("fmt"; "math"; "time")
func main() {
    pi := 3.141592653589793
    solar := 4.0 * pi * pi
    days := 365.24
    dt := 0.01
    nSteps := 2000000
    x := []float64{0.0, 4.841431442464721, 8.34336671824458, 12.894369562139131, 15.379697114850917}
    y := []float64{0.0, -1.1603200440274284, 4.124798564124305, -15.111151401698631, -25.919314609987964}
    z := []float64{0.0, -0.10362204447112311, -0.4035234171143214, -0.22330757889265573, 0.17925877295037118}
    vx := []float64{0.0, 0.001660076642744037, -0.002767425107268624, 0.002964601375647616, 0.0026806777249038932}
    vy := []float64{0.0, 0.007699011184197404, 0.004998528012349172, 0.0023784717395948095, 0.001628241700382423}
    vz := []float64{0.0, -6.90460016972063e-05, 2.3041729757376393e-05, -2.9658956854023756e-05, -9.515922545197159e-05}
    m := []float64{1.0, 0.0009547919384243266, 0.0002858859806661308, 4.366244043351563e-05, 5.1513890204661145e-05}
    for i := 0; i < 5; i++ { m[i] *= solar; vx[i] *= days; vy[i] *= days; vz[i] *= days }
    px, py, pz := 0.0, 0.0, 0.0
    for i := 0; i < 5; i++ {
        t1 := vx[i] * m[i]; px += t1
        t2 := vy[i] * m[i]; py += t2
        t3 := vz[i] * m[i]; pz += t3
    }
    vx[0] = -px / solar; vy[0] = -py / solar; vz[0] = -pz / solar
    t0 := time.Now()
    for s := 0; s < nSteps; s++ {
        for i := 0; i < 5; i++ {
            for j := i + 1; j < 5; j++ {
                dx := x[i] - x[j]; dy := y[i] - y[j]; dz := z[i] - z[j]
                t1 := dx * dx; t2 := dy * dy; t3 := dz * dz
                d2 := t1 + t2 + t3
                dist := math.Sqrt(d2)
                mag := dt / (d2 * dist)
                mi := m[i] * mag; mj := m[j] * mag
                u1 := dx * mj; vx[i] -= u1
                u2 := dy * mj; vy[i] -= u2
                u3 := dz * mj; vz[i] -= u3
                u4 := dx * mi; vx[j] += u4
                u5 := dy * mi; vy[j] += u5
                u6 := dz * mi; vz[j] += u6
            }
        }
        for i := 0; i < 5; i++ {
            w1 := dt * vx[i]; x[i] += w1
            w2 := dt * vy[i]; y[i] += w2
            w3 := dt * vz[i]; z[i] += w3
        }
    }
    e := 0.0
    for i := 0; i < 5; i++ {
        t1 := vx[i] * vx[i]; t2 := vy[i] * vy[i]; t3 := vz[i] * vz[i]
        sq := t1 + t2 + t3
        h := 0.5 * m[i] * sq
        e += h
        for j := i + 1; j < 5; j++ {
            dx := x[i] - x[j]; dy := y[i] - y[j]; dz := z[i] - z[j]
            u1 := dx * dx; u2 := dy * dy; u3 := dz * dz
            d2 := u1 + u2 + u3
            q := m[i] * m[j] / math.Sqrt(d2)
            e -= q
        }
    }
    fmt.Printf("CHECK %d\nMS %d\n", int64(math.Round(e*1000000.0)), time.Since(t0).Milliseconds())
}
