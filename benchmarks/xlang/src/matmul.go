package main
import ("fmt"; "math"; "time")
func main() {
    n := 300
    a := make([][]float64, n)
    b := make([][]float64, n)
    c := make([][]float64, n)
    for i := 0; i < n; i++ {
        a[i] = make([]float64, n); b[i] = make([]float64, n); c[i] = make([]float64, n)
        for j := 0; j < n; j++ {
            a[i][j] = float64((i*j)%100) * 0.01
            b[i][j] = float64((i+j)%100) * 0.01
        }
    }
    t0 := time.Now()
    for i := 0; i < n; i++ {
        for j := 0; j < n; j++ {
            s := 0.0
            for k := 0; k < n; k++ {
                p := a[i][k] * b[k][j]
                s += p
            }
            c[i][j] = s
        }
    }
    t := 0.0
    for i := 0; i < n; i++ { for j := 0; j < n; j++ { t += c[i][j] } }
    fmt.Printf("CHECK %d\nMS %d\n", int64(math.Round(t)), time.Since(t0).Milliseconds())
}
