package main
import ("fmt"; "time")
func main() {
    n, k, iters := 200000, 10, 15
    xs := make([]float64, n)
    ys := make([]float64, n)
    var seed int64 = 42
    for i := 0; i < n; i++ {
        seed = seed * 48271 % 2147483647
        xs[i] = 10.0 * float64(seed) / 2147483647.0
        seed = seed * 48271 % 2147483647
        ys[i] = 10.0 * float64(seed) / 2147483647.0
    }
    t0 := time.Now()
    stride := n / k
    cx := make([]float64, k)
    cy := make([]float64, k)
    for c := 0; c < k; c++ { cx[c] = xs[c*stride]; cy[c] = ys[c*stride] }
    assign := make([]int, n)
    for it := 0; it < iters; it++ {
        for i := 0; i < n; i++ {
            best, bd := 0, 1000000.0
            for c := 0; c < k; c++ {
                dx := xs[i] - cx[c]
                dy := ys[i] - cy[c]
                t1 := dx * dx
                t2 := dy * dy
                d := t1 + t2
                if d < bd { bd = d; best = c }
            }
            assign[i] = best
        }
        sx := make([]float64, k)
        sy := make([]float64, k)
        ct := make([]int, k)
        for i := 0; i < n; i++ {
            c := assign[i]
            sx[c] += xs[i]; sy[c] += ys[i]; ct[c]++
        }
        for c := 0; c < k; c++ {
            if ct[c] > 0 { cx[c] = sx[c] / float64(ct[c]); cy[c] = sy[c] / float64(ct[c]) }
        }
    }
    sizes := make([]int, k)
    for i := 0; i < n; i++ { sizes[assign[i]]++ }
    largest := 0
    for c := 0; c < k; c++ { if sizes[c] > largest { largest = sizes[c] } }
    fmt.Printf("CHECK %d\nMS %d\n", largest, time.Since(t0).Milliseconds())
}
