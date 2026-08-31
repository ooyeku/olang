package main
import ("fmt"; "time")
func steps(n int64) int64 {
    var c int64 = 0
    for n != 1 {
        if n%2 == 0 { n = n / 2 } else { n = 3*n + 1 }
        c++
    }
    return c
}
func main() {
    t0 := time.Now()
    var total int64 = 0
    for i := int64(1); i <= 300000; i++ { total += steps(i) }
    fmt.Printf("CHECK %d\nMS %d\n", total, time.Since(t0).Milliseconds())
}
